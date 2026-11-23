//! Panel model hooks: linking a new split to its master, moving resources when an admin edits a split,
//! passing suspension and owner changes of a master on to its splits, returning a deleted split's
//! resources, and mirroring master subuser changes to the splits.

use crate::{
    model,
    pool::{self, Charged, LedgerEntry, Limits},
    routes::client::{get_subservers, splitter_data},
    settings::{ServerSplitterSettingsData, SwapMode},
};
use shared::{
    Extendible, State,
    models::{
        ByUuid, CreatableModel, DeletableModel, ListenerPriority, UpdatableModel,
        node::PermissionsInvalidation,
        server::{DeleteServerOptions, Server, UpdateServerOptions},
        server_subuser::{ServerSubuser, UpdateServerSubuserOptions},
    },
    response::DisplayError,
};
use sqlx::Row;

/// How a split delete in progress should treat its master. Set by the splitter's own delete route and
/// by a master's delete; a delete started anywhere else (admin) returns ports and credits the master.
#[derive(Clone, Copy, Debug)]
pub struct SplitDelete {
    pub return_ports: bool,
    /// The master is being deleted too: nothing to give back or sync.
    pub master_going: bool,
}

tokio::task_local! {
    pub static SPLIT_DELETE: SplitDelete;
}

pub async fn config(state: &State) -> ServerSplitterSettingsData {
    match state.settings.get().await {
        Ok(settings) => settings
            .find_extension_settings::<ServerSplitterSettingsData>()
            .cloned()
            .unwrap_or_default(),
        Err(err) => {
            tracing::warn!("failed to read server splitter settings: {err:?}");
            ServerSplitterSettingsData::default()
        }
    }
}

fn display_error(message: String) -> shared::database::DatabaseError {
    anyhow::Error::new(DisplayError::new(message)).into()
}

pub fn register() {
    // A split is inserted already linked to its master, taking over the master's allocation in the
    // same transaction. Other servers get `feature_limits.splits` from the payload, else the default.
    Server::register_create_handler(
        ListenerPriority::Normal,
        |options, query_builder, state, transaction| {
            Box::pin(async move {
                if let Ok(pending) = pool::PENDING_SPLIT.try_with(|pending| *pending) {
                    query_builder
                        .set("parent_uuid", pending.master_uuid)
                        .set("splits", 0);

                    if let Some(server_allocation) = pending.transferred_server_allocation {
                        let moved = sqlx::query(
                            "DELETE FROM server_allocations WHERE uuid = $1 AND server_uuid = $2",
                        )
                        .bind(server_allocation)
                        .bind(pending.master_uuid)
                        .execute(&mut **transaction)
                        .await?;
                        if moved.rows_affected() != 1 {
                            return Err(anyhow::anyhow!(
                                "the master's allocation changed while creating the split"
                            )
                            .into());
                        }
                    }

                    return Ok(());
                }

                let splits = match options
                    .feature_limits
                    .parse_extended::<model::ExtendedApiServerFeatureLimits>()
                    .ok()
                    .and_then(|extended| extended.splits)
                {
                    Some(splits) => splits,
                    None => config(state).await.default_splits,
                };
                query_builder.set("splits", splits.max(0));
                Ok(())
            })
        },
    );

    // The split's ledger entry is written in the split's own insert transaction, so a split never
    // exists without a record of what it took.
    Server::register_after_create_handler(
        ListenerPriority::Normal,
        |server, _options, _state, transaction| {
            Box::pin(async move {
                if let Ok(pending) = pool::PENDING_SPLIT.try_with(|pending| *pending) {
                    pool::save_ledger(
                        &mut **transaction,
                        &LedgerEntry {
                            split_uuid: server.uuid,
                            master_uuid: pending.master_uuid,
                            granted: pending.granted,
                            charged: pending.charged,
                            port_from_master: pending.transferred_server_allocation.is_some(),
                        },
                    )
                    .await?;

                    // Other extensions' per-server limits (subdomains, proxies, ...) aren't part of
                    // the pool: a split starts with none instead of each extension's default.
                    let columns: Vec<String> = sqlx::query_scalar(
                        r#"
                        SELECT column_name::text
                        FROM information_schema.columns
                        WHERE table_schema = current_schema()
                            AND table_name = 'servers'
                            AND data_type = 'integer'
                            AND column_name LIKE '%\_limit'
                            AND column_name NOT IN ('allocation_limit', 'database_limit', 'backup_limit', 'schedule_limit')
                        "#,
                    )
                    .fetch_all(&mut **transaction)
                    .await?;
                    for column in columns {
                        if !column
                            .chars()
                            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
                        {
                            continue;
                        }
                        sqlx::query(sqlx::AssertSqlSafe(format!(
                            r#"UPDATE servers SET "{column}" = 0 WHERE uuid = $1"#
                        )))
                        .bind(server.uuid)
                        .execute(&mut **transaction)
                        .await?;
                    }
                }
                Ok(())
            })
        },
    );

    Server::register_update_handler(
        ListenerPriority::Normal,
        |server, options, query_builder, state, transaction| {
            Box::pin(async move {
                if let Some(feature_limits) = &options.feature_limits
                    && let Ok(extended) =
                        feature_limits.parse_extended::<model::ExtendedApiServerFeatureLimits>()
                    && let Some(value) = extended.splits
                {
                    query_builder.set("splits", Some(value));
                }

                let data = splitter_data(server);
                let config = config(state).await;

                match data.parent_uuid {
                    Some(master_uuid) => {
                        if options.suspended == Some(false) {
                            query_builder.set("split_suspended_by_master", Some(false));
                        }
                        regrant_split_edit(server, master_uuid, options, &config, state, transaction)
                            .await
                    }
                    None => cascade_master_changes(server, options, &config, state, transaction).await,
                }
            })
        },
    );

    // Runs inside the panel's delete transaction, which commits only once Wings has deleted the server.
    Server::register_delete_handler(
        ListenerPriority::Normal,
        |server, options, state, transaction| {
            Box::pin(async move {
                let data = splitter_data(server);
                let mode = SPLIT_DELETE.try_with(|mode| *mode).unwrap_or(SplitDelete {
                    return_ports: true,
                    master_going: false,
                });

                // A split: give back what it holds, under the master's lock so a concurrent resize is
                // credited at its committed size, then sync the master once this transaction ends.
                if let Some(parent_uuid) = data.parent_uuid {
                    if mode.master_going {
                        return Ok(());
                    }
                    if pool::lock_master(transaction, parent_uuid).await?.is_some() {
                        pool::credit_master(transaction, server.uuid).await?;
                        if mode.return_ports {
                            pool::return_ports(transaction, server.uuid, parent_uuid).await?;
                        }
                        pool::sync_after_unlock(state.clone(), parent_uuid, vec![parent_uuid]);
                    }
                    return Ok(());
                }

                // A master: delete its splits properly first. Removing their rows along with the
                // master would leave their containers running on Wings and their databases on the
                // database hosts.
                for split in get_subservers(state, server.uuid).await? {
                    SPLIT_DELETE
                        .scope(
                            SplitDelete {
                                return_ports: false,
                                master_going: true,
                            },
                            split.delete(
                                state,
                                DeleteServerOptions {
                                    force: options.force,
                                },
                            ),
                        )
                        .await?;
                }

                Ok(())
            })
        },
    );

    // Master subusers: permission changes and removals reach the splits they were synced to. The
    // after-handlers run once the row is written but before commit; the work waits for the commit.
    ServerSubuser::register_after_update_handler(
        ListenerPriority::Normal,
        |subuser, state, _transaction| {
            Box::pin(async move {
                let (state, server_uuid, user_uuid) =
                    (state.clone(), subuser.server.uuid, subuser.user.uuid);
                tokio::spawn(async move {
                    if let Err(err) = mirror_subuser(&state, server_uuid, user_uuid).await {
                        tracing::warn!(server = %server_uuid, user = %user_uuid, "failed to mirror subuser to splits: {err:?}");
                    }
                });
                Ok(())
            })
        },
    );

    ServerSubuser::register_after_delete_handler(
        ListenerPriority::Normal,
        |subuser, _options, state, _transaction| {
            Box::pin(async move {
                let (state, server_uuid, user_uuid) =
                    (state.clone(), subuser.server.uuid, subuser.user.uuid);
                tokio::spawn(async move {
                    if let Err(err) = mirror_subuser(&state, server_uuid, user_uuid).await {
                        tracing::warn!(server = %server_uuid, user = %user_uuid, "failed to mirror subuser to splits: {err:?}");
                    }
                });
                Ok(())
            })
        },
    );
}

/// An admin (or anything else going through the panel's server update) changed a split's limits:
/// take the difference from its master or give it back, so the pool stays whole. Fails when the
/// master can't cover it.
async fn regrant_split_edit(
    split: &Server,
    master_uuid: uuid::Uuid,
    options: &UpdateServerOptions,
    config: &ServerSplitterSettingsData,
    state: &State,
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<(), shared::database::DatabaseError> {
    if options.limits.is_none() && options.feature_limits.is_none() {
        return Ok(());
    }

    let current = Limits::of(split);
    let mut requested = current;
    if let Some(limits) = &options.limits {
        requested.cpu = limits.cpu;
        requested.memory = limits.memory;
        requested.swap = limits.swap;
        requested.disk = limits.disk;
    }
    if let Some(feature_limits) = &options.feature_limits {
        requested.allocations = feature_limits.allocations;
        requested.databases = feature_limits.databases;
        requested.backups = feature_limits.backups;
        requested.schedules = feature_limits.schedules;
    }
    if requested == current {
        return Ok(());
    }

    let Some(master) = pool::lock_master(transaction, master_uuid).await? else {
        return Ok(());
    };
    let entry = match pool::ledger_entry(&mut **transaction, split.uuid).await? {
        Some(entry) => entry,
        None => untracked_entry(split, master_uuid, &Limits::of(&master)),
    };

    let (debit, charged) = pool::regrant(
        &Limits::of(&master),
        config.swap_mode == SwapMode::Pooled,
        &entry.granted,
        &entry.charged,
        &requested,
    );
    if !pool::debit_master(transaction, master.uuid, &debit).await? {
        return Err(display_error(format!(
            "{} is a split of {}, which doesn't have enough left for this change. Raise {}'s limits first, or resize the split from the Server Splitter card on {}'s overview.",
            split.name, master.name, master.name, master.name
        )));
    }

    pool::save_ledger(
        &mut **transaction,
        &LedgerEntry {
            granted: requested,
            charged,
            ..entry
        },
    )
    .await?;
    pool::sync_after_unlock(state.clone(), master.uuid, vec![master.uuid]);

    Ok(())
}

/// A ledger entry for a split that has none: granted = its limits, charged the way 1.5.2 credited
/// back on delete (both master and split limited).
pub fn untracked_entry(split: &Server, master_uuid: uuid::Uuid, master: &Limits) -> LedgerEntry {
    let granted = Limits::of(split);
    LedgerEntry {
        split_uuid: split.uuid,
        master_uuid,
        granted,
        charged: Charged {
            cpu: if master.cpu > 0 && granted.cpu > 0 { granted.cpu } else { 0 },
            memory: if master.memory > 0 && granted.memory > 0 {
                granted.memory
            } else {
                0
            },
            swap: 0,
            disk: if master.disk > 0 && granted.disk > 0 {
                granted.disk
            } else {
                0
            },
        },
        port_from_master: false,
    }
}

/// Suspension and owner changes of a master reach its splits (admin settings, on by default).
async fn cascade_master_changes(
    master: &Server,
    options: &UpdateServerOptions,
    config: &ServerSplitterSettingsData,
    state: &State,
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<(), shared::database::DatabaseError> {
    let suspend = options
        .suspended
        .filter(|suspended| config.cascade_suspension && *suspended != master.suspended);
    let new_owner = options
        .owner_uuid
        .filter(|owner| config.cascade_owner && *owner != master.owner.uuid);
    if suspend.is_none() && new_owner.is_none() {
        return Ok(());
    }

    // Held until the master's update commits; the sync below waits for it.
    sqlx::query("SELECT 1 FROM servers WHERE uuid = $1 FOR NO KEY UPDATE")
        .bind(master.uuid)
        .execute(&mut **transaction)
        .await?;

    let mut changed: Vec<uuid::Uuid> = Vec::new();

    if let Some(suspended) = suspend {
        let rows = if suspended {
            sqlx::query(
                r#"
                UPDATE servers SET suspended = TRUE, split_suspended_by_master = TRUE
                WHERE parent_uuid = $1 AND suspended = FALSE
                RETURNING uuid
                "#,
            )
        } else {
            sqlx::query(
                r#"
                UPDATE servers SET suspended = FALSE, split_suspended_by_master = FALSE
                WHERE parent_uuid = $1 AND split_suspended_by_master
                RETURNING uuid
                "#,
            )
        }
        .bind(master.uuid)
        .fetch_all(&mut **transaction)
        .await?;
        changed.extend(rows.iter().map(|row| row.get::<uuid::Uuid, _>("uuid")));
    }

    let mut moved: Vec<uuid::Uuid> = Vec::new();
    if let Some(owner) = new_owner {
        let rows = sqlx::query(
            "UPDATE servers SET owner_uuid = $1 WHERE parent_uuid = $2 AND owner_uuid = $3 RETURNING uuid",
        )
        .bind(owner)
        .bind(master.uuid)
        .bind(master.owner.uuid)
        .fetch_all(&mut **transaction)
        .await?;
        moved.extend(rows.iter().map(|row| row.get::<uuid::Uuid, _>("uuid")));
        changed.extend(moved.iter().copied());
    }

    if changed.is_empty() {
        return Ok(());
    }
    changed.sort();
    changed.dedup();

    let state = state.clone();
    let (master_uuid, previous_owner) = (master.uuid, master.owner.uuid);
    tokio::spawn(async move {
        pool::wait_for_unlock(&state, master_uuid).await;
        pool::refresh_and_sync(&state, &changed).await;

        if let Some(owner) = new_owner {
            for split_uuid in moved {
                let node = async {
                    let split = Server::by_uuid_optional(&state.database, split_uuid)
                        .await?
                        .ok_or_else(|| anyhow::anyhow!("split not found"))?;
                    Ok::<_, anyhow::Error>(split.node.fetch_cached(&state.database).await?)
                }
                .await;
                let invalidation = PermissionsInvalidation {
                    users: vec![previous_owner, owner],
                    ..Default::default()
                };
                match node {
                    Ok(node) => node.invalidate_permissions(&state.database, invalidation),
                    Err(_) => shared::models::node::Node::invalidate_permissions_everywhere(
                        &state.database,
                        invalidation,
                    ),
                }
            }
        }
    });

    Ok(())
}

/// After a master subuser changed or was removed (the row lock is held until the change commits),
/// makes the master's splits match: removed users go, permission changes apply to synced users.
/// Users marked custom on a split keep their permissions there; removal still reaches them.
async fn mirror_subuser(
    state: &State,
    server_uuid: uuid::Uuid,
    user_uuid: uuid::Uuid,
) -> Result<(), anyhow::Error> {
    if !config(state).await.mirror_subusers {
        return Ok(());
    }

    // Blocks until the subuser change commits or rolls back.
    let master_row = {
        let mut transaction = state.database.write().begin().await?;
        let row = sqlx::query(
            "SELECT permissions, ignored_files FROM server_subusers WHERE server_uuid = $1 AND user_uuid = $2 FOR SHARE",
        )
        .bind(server_uuid)
        .bind(user_uuid)
        .fetch_optional(&mut *transaction)
        .await?;
        transaction.commit().await?;
        row
    };

    let splits: Vec<(uuid::Uuid, bool)> = sqlx::query(
        r#"
        SELECT s.uuid, (c.user_uuid IS NOT NULL) AS custom
        FROM servers s
        JOIN servers m ON m.uuid = s.parent_uuid
        JOIN server_subusers ss ON ss.server_uuid = s.uuid AND ss.user_uuid = $2
        LEFT JOIN serversplitter_custom_subusers c ON c.split_uuid = s.uuid AND c.user_uuid = $2
        WHERE s.parent_uuid = $1 AND s.owner_uuid = m.owner_uuid
        "#,
    )
    .bind(server_uuid)
    .bind(user_uuid)
    .fetch_all(state.database.read())
    .await?
    .into_iter()
    .map(|row| (row.get("uuid"), row.get("custom")))
    .collect();

    for (split_uuid, custom) in splits {
        let Some(split_subuser) =
            ServerSubuser::by_server_uuid_user_uuid(&state.database, split_uuid, user_uuid).await?
        else {
            continue;
        };

        match &master_row {
            None => split_subuser.delete(state, ()).await?,
            Some(_) if custom => {}
            Some(row) => {
                let permissions: Vec<compact_str::CompactString> = row.get("permissions");
                let ignored_files: Vec<compact_str::CompactString> = row.get("ignored_files");
                if split_subuser.permissions != permissions
                    || split_subuser.ignored_files != ignored_files
                {
                    let mut split_subuser = split_subuser;
                    split_subuser
                        .update(
                            state,
                            UpdateServerSubuserOptions {
                                permissions: Some(permissions),
                                ignored_files: Some(ignored_files),
                            },
                        )
                        .await?;
                }
            }
        }
    }

    Ok(())
}
