//! Pool health: compares every split's limits with its ledger entry, and puts them back in line.

use crate::{
    hooks,
    pool::{self, LedgerEntry, Limits},
    settings::{ServerSplitterSettingsData, SwapMode},
};
use serde::Serialize;
use shared::{
    State,
    models::{ByUuid, server::Server},
};
use sqlx::{PgConnection, Row};
use utoipa::ToSchema;

#[derive(ToSchema, Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SplitHealth {
    Balanced,
    /// Its limits differ from what the splitter granted: changed outside the splitter.
    Drift,
    /// It has no ledger entry.
    Untracked,
}

#[derive(ToSchema, Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IssueKind {
    Drift,
    Untracked,
    /// A ledger entry for a server that is no longer this master's split.
    Orphaned,
}

#[derive(ToSchema, Serialize, Clone, Debug)]
pub struct PoolIssue {
    pub split_uuid: uuid::Uuid,
    pub split_name: compact_str::CompactString,
    pub kind: IssueKind,
    pub detail: String,
}

/// A split of a master with its current limits and ledger entry.
#[derive(Clone, Debug)]
pub struct SplitState {
    pub uuid: uuid::Uuid,
    pub name: compact_str::CompactString,
    pub limits: Limits,
    pub ledger: Option<LedgerEntry>,
}

impl SplitState {
    pub fn health(&self) -> SplitHealth {
        match &self.ledger {
            None => SplitHealth::Untracked,
            Some(entry) if entry.granted != self.limits => SplitHealth::Drift,
            Some(_) => SplitHealth::Balanced,
        }
    }

    pub fn issue(&self) -> Option<PoolIssue> {
        let (kind, detail) = match &self.ledger {
            None => (
                IssueKind::Untracked,
                "No record of what this split took from its master.".to_string(),
            ),
            Some(entry) if entry.granted != self.limits => (
                IssueKind::Drift,
                format!(
                    "Limits changed outside the splitter ({}).",
                    entry.granted.differences(&self.limits).join(", ")
                ),
            ),
            Some(_) => return None,
        };

        Some(PoolIssue {
            split_uuid: self.uuid,
            split_name: self.name.clone(),
            kind,
            detail,
        })
    }
}

/// Every split of `master_uuid` (any owner) with its ledger entry.
pub async fn split_states(
    conn: &mut PgConnection,
    master_uuid: uuid::Uuid,
) -> Result<Vec<SplitState>, sqlx::Error> {
    let rows = sqlx::query(
        r#"
        SELECT
            s.uuid, s.name, s.cpu, s.memory, s.swap, s.disk,
            s.allocation_limit, s.database_limit, s.backup_limit, s.schedule_limit
        FROM servers s
        WHERE s.parent_uuid = $1
        ORDER BY s.created ASC
        "#,
    )
    .bind(master_uuid)
    .fetch_all(&mut *conn)
    .await?;

    let ledger = pool::ledger_by_master(&mut *conn, master_uuid).await?;

    Ok(rows
        .into_iter()
        .map(|row| {
            let uuid: uuid::Uuid = row.get("uuid");
            SplitState {
                uuid,
                name: row.get::<String, _>("name").into(),
                limits: Limits {
                    cpu: row.get("cpu"),
                    memory: row.get("memory"),
                    swap: row.get("swap"),
                    disk: row.get("disk"),
                    allocations: row.get("allocation_limit"),
                    databases: row.get("database_limit"),
                    backups: row.get("backup_limit"),
                    schedules: row.get("schedule_limit"),
                },
                ledger: ledger.iter().find(|entry| entry.split_uuid == uuid).copied(),
            }
        })
        .collect())
}

/// Ledger entries charged to `master_uuid` whose server is no longer its split.
pub async fn orphaned(
    conn: &mut PgConnection,
    master_uuid: uuid::Uuid,
) -> Result<Vec<(LedgerEntry, compact_str::CompactString)>, sqlx::Error> {
    let splits: Vec<uuid::Uuid> =
        sqlx::query_scalar("SELECT uuid FROM servers WHERE parent_uuid = $1")
            .bind(master_uuid)
            .fetch_all(&mut *conn)
            .await?;

    let mut result = Vec::new();
    for entry in pool::ledger_by_master(&mut *conn, master_uuid).await? {
        if splits.contains(&entry.split_uuid) {
            continue;
        }
        let name: Option<String> = sqlx::query_scalar("SELECT name FROM servers WHERE uuid = $1")
            .bind(entry.split_uuid)
            .fetch_optional(&mut *conn)
            .await?;
        result.push((entry, name.unwrap_or_default().into()));
    }

    Ok(result)
}

/// Every problem in `master_uuid`'s pool.
pub async fn issues(
    conn: &mut PgConnection,
    master_uuid: uuid::Uuid,
) -> Result<Vec<PoolIssue>, sqlx::Error> {
    let mut issues: Vec<PoolIssue> = split_states(&mut *conn, master_uuid)
        .await?
        .iter()
        .filter_map(SplitState::issue)
        .collect();

    for (entry, name) in orphaned(&mut *conn, master_uuid).await? {
        issues.push(PoolIssue {
            split_uuid: entry.split_uuid,
            split_name: name,
            kind: IssueKind::Orphaned,
            detail: "Recorded as this master's split, but the server isn't linked to it anymore."
                .to_string(),
        });
    }

    Ok(issues)
}

#[derive(ToSchema, Serialize, Clone, Debug)]
pub struct ReconcileFailure {
    pub split_uuid: uuid::Uuid,
    pub split_name: compact_str::CompactString,
    pub reason: String,
}

#[derive(ToSchema, Serialize, Clone, Debug, Default)]
pub struct ReconcileResult {
    pub fixed: usize,
    pub failed: Vec<ReconcileFailure>,
}

/// Puts `master_uuid`'s pool back in line, under the master's lock:
/// - drift: the split's current limits become its grant, the difference moves to or from the master
///   (a split the master can't cover is left as it is and reported);
/// - untracked: recorded from its current limits, the way 1.5.2 accounted for it;
/// - orphaned: what it was charged goes back to the master and the entry is dropped.
pub async fn reconcile(state: &State, master_uuid: uuid::Uuid) -> Result<ReconcileResult, anyhow::Error> {
    let config: ServerSplitterSettingsData = hooks::config(state).await;
    let mut result = ReconcileResult::default();

    let mut transaction = state.database.write().begin().await?;
    let Some(master) = pool::lock_master(&mut transaction, master_uuid).await? else {
        return Ok(result);
    };

    for split in split_states(&mut transaction, master_uuid).await? {
        match split.health() {
            SplitHealth::Balanced => {}
            SplitHealth::Untracked => {
                let server = Server::by_uuid_optional_with_transaction(&mut transaction, split.uuid)
                    .await?;
                if let Some(server) = server {
                    let fresh = Server::by_uuid_optional_with_transaction(&mut transaction, master_uuid)
                        .await?
                        .map(|master| Limits::of(&master))
                        .unwrap_or_else(|| Limits::of(&master));
                    pool::save_ledger(
                        &mut transaction,
                        &hooks::untracked_entry(&server, master_uuid, &fresh),
                    )
                    .await?;
                    result.fixed += 1;
                }
            }
            SplitHealth::Drift => {
                let Some(entry) = split.ledger else { continue };
                let fresh = Server::by_uuid_optional_with_transaction(&mut transaction, master_uuid)
                    .await?
                    .map(|master| Limits::of(&master))
                    .unwrap_or_else(|| Limits::of(&master));
                let (debit, charged) = pool::regrant(
                    &fresh,
                    config.swap_mode == SwapMode::Pooled,
                    &entry.granted,
                    &entry.charged,
                    &split.limits,
                );
                if pool::debit_master(&mut transaction, master_uuid, &debit).await? {
                    pool::save_ledger(
                        &mut transaction,
                        &LedgerEntry {
                            granted: split.limits,
                            charged,
                            ..entry
                        },
                    )
                    .await?;
                    result.fixed += 1;
                } else {
                    result.failed.push(ReconcileFailure {
                        split_uuid: split.uuid,
                        split_name: split.name.clone(),
                        reason: format!(
                            "{} doesn't have enough left to cover {} ({}). Raise the master's limits or lower the split's.",
                            master.name,
                            split.name,
                            entry.granted.differences(&split.limits).join(", ")
                        ),
                    });
                }
            }
        }
    }

    for (entry, _) in orphaned(&mut transaction, master_uuid).await? {
        sqlx::query("DELETE FROM serversplitter_splits WHERE split_uuid = $1")
            .bind(entry.split_uuid)
            .execute(&mut *transaction)
            .await?;
        pool::debit_master(&mut transaction, master_uuid, &entry.refund()).await?;
        result.fixed += 1;
    }

    transaction.commit().await?;
    pool::refresh_and_sync(state, &[master_uuid]).await;

    Ok(result)
}

/// Masters that have splits or ledger entries.
pub async fn masters_with_splits(conn: &mut PgConnection) -> Result<Vec<uuid::Uuid>, sqlx::Error> {
    sqlx::query_scalar(
        r#"
        SELECT DISTINCT parent_uuid FROM servers WHERE parent_uuid IS NOT NULL
        UNION
        SELECT DISTINCT master_uuid FROM serversplitter_splits
        "#,
    )
    .fetch_all(conn)
    .await
}

/// Logs pool problems at startup and then hourly, so drift shows up in the panel log too.
pub fn spawn_periodic_check(state: State) {
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(3600));

        loop {
            interval.tick().await;

            let check = async {
                let mut conn = state.database.read().acquire().await?;
                let mut problems = 0usize;
                for master in masters_with_splits(&mut conn).await? {
                    let issues = issues(&mut conn, master).await?;
                    for issue in &issues {
                        tracing::warn!(master = %master, split = %issue.split_uuid, "server splitter pool: {}", issue.detail);
                    }
                    problems += issues.len();
                }
                Ok::<_, sqlx::Error>(problems)
            };

            match check.await {
                Ok(0) => tracing::debug!("server splitter pools are balanced"),
                Ok(problems) => tracing::warn!(
                    "server splitter: {problems} split(s) need attention, see Admin > Extensions > Server Splitter > Splits"
                ),
                Err(err) => tracing::warn!("server splitter pool check failed: {err:?}"),
            }
        }
    });
}
