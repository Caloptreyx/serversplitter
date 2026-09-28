//! A master server's resource pool: what its splits may take, and the locked debits and credits
//! that move resources between the master and its splits.
//!
//! Every change to a master's pool first locks the master row (`FOR NO KEY UPDATE`) and re-reads
//! it inside that transaction. The lock serializes concurrent split operations on one master,
//! and the fresh read bypasses the panel's server cache, which raw `UPDATE`s here don't touch.
//! `FOR NO KEY UPDATE` still lets other transactions insert rows referencing the master, which
//! [`Server::create`] does while a split is being created under the lock.

use crate::settings::ServerSplitterSettingsData;
use shared::{
    State,
    database::DatabaseError,
    models::{ByUuid, server::Server},
};
use sqlx::{PgConnection, Postgres, Row, Transaction};

/// cpu (%), memory and disk (MB), and feature limits. For cpu/memory/disk 0 means unlimited on a
/// server, and -1 means unlimited in a pool computed by [`remaining`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Limits {
    pub cpu: i32,
    pub memory: i64,
    pub disk: i64,
    pub allocations: i32,
    pub databases: i32,
    pub backups: i32,
    pub schedules: i32,
}

impl Limits {
    pub fn of(server: &Server) -> Self {
        Self {
            cpu: server.cpu,
            memory: server.memory,
            disk: server.disk,
            allocations: server.allocation_limit,
            databases: server.database_limit,
            backups: server.backup_limit,
            schedules: server.schedule_limit,
        }
    }

    /// `self - other`, field by field.
    pub fn minus(&self, other: &Self) -> Self {
        Self {
            cpu: self.cpu - other.cpu,
            memory: self.memory - other.memory,
            disk: self.disk - other.disk,
            allocations: self.allocations - other.allocations,
            databases: self.databases - other.databases,
            backups: self.backups - other.backups,
            schedules: self.schedules - other.schedules,
        }
    }
}

/// What a server uses of its feature limits, counted the way the panel enforces them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FeatureUsage {
    pub allocations: i64,
    /// Databases plus database instances: both count against `database_limit`.
    pub databases: i64,
    pub backups: i64,
    pub schedules: i64,
}

pub async fn feature_usage(
    conn: &mut PgConnection,
    server_uuid: uuid::Uuid,
) -> Result<FeatureUsage, sqlx::Error> {
    let row = sqlx::query(
        r#"
        SELECT
            (SELECT count(*) FROM server_allocations WHERE server_uuid = $1) AS allocations,
            (SELECT count(*) FROM server_databases WHERE server_uuid = $1)
                + (SELECT count(*) FROM server_database_instances WHERE server_uuid = $1)
                AS databases,
            (
                SELECT count(*) FROM server_backups
                WHERE server_uuid = $1
                    AND system_backup_policy_uuid IS NULL
                    AND deleted IS NULL
            ) AS backups,
            (SELECT count(*) FROM server_schedules WHERE server_uuid = $1) AS schedules
        "#,
    )
    .bind(server_uuid)
    .fetch_one(conn)
    .await?;

    Ok(FeatureUsage {
        allocations: row.get("allocations"),
        databases: row.get("databases"),
        backups: row.get("backups"),
        schedules: row.get("schedules"),
    })
}

/// A non-primary allocation of the master that a new split takes over as its primary one.
pub struct TransferableAllocation {
    pub server_allocation_uuid: uuid::Uuid,
    pub allocation_uuid: uuid::Uuid,
    pub notes: Option<String>,
    pub created: chrono::NaiveDateTime,
}

pub async fn transferable_allocation(
    conn: &mut PgConnection,
    master_uuid: uuid::Uuid,
) -> Result<Option<TransferableAllocation>, sqlx::Error> {
    let row = sqlx::query(
        r#"
        SELECT sa.uuid, sa.allocation_uuid, sa.notes, sa.created
        FROM server_allocations sa
        JOIN servers s ON s.uuid = sa.server_uuid
        WHERE sa.server_uuid = $1
          AND (s.allocation_uuid IS NULL OR sa.uuid != s.allocation_uuid)
        ORDER BY sa.created ASC
        LIMIT 1
        "#,
    )
    .bind(master_uuid)
    .fetch_optional(conn)
    .await?;

    Ok(row.map(|row| TransferableAllocation {
        server_allocation_uuid: row.get("uuid"),
        allocation_uuid: row.get("allocation_uuid"),
        notes: row.get("notes"),
        created: row.get("created"),
    }))
}

/// Gives a transferred allocation back to the master after a failed split creation. A no-op when
/// the transfer was rolled back and the master still holds it.
pub async fn restore_allocation(
    state: &State,
    master_uuid: uuid::Uuid,
    allocation: &TransferableAllocation,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO server_allocations (uuid, server_uuid, allocation_uuid, notes, created)
        VALUES ($1, $2, $3, $4, $5)
        ON CONFLICT DO NOTHING
        "#,
    )
    .bind(allocation.server_allocation_uuid)
    .bind(master_uuid)
    .bind(allocation.allocation_uuid)
    .bind(&allocation.notes)
    .bind(allocation.created)
    .execute(state.database.write())
    .await?;

    Ok(())
}

/// Disk the master's files currently use, in MB, when the config counts it against the pool.
pub async fn disk_usage_mb(
    state: &State,
    master: &Server,
    config: &ServerSplitterSettingsData,
) -> i64 {
    if !config.include_disk_usage {
        return 0;
    }

    let Ok(node) = master.node.fetch_cached(&state.database).await else {
        return 0;
    };
    let Ok(resources) = node.fetch_server_resources(&state.database).await else {
        return 0;
    };

    resources
        .get(&master.uuid)
        .map(|r| (r.disk_bytes / 1024 / 1024) as i64)
        .unwrap_or(0)
}

/// The most a split may hold of each resource. `split` is the split being resized (its current
/// holdings go back into the pool); `transferable_allocation` is whether a new split would take
/// over one of the master's existing allocations.
///
/// A limited master always keeps at least 1 cpu/memory/disk (or its reserve, if larger): 0 would
/// make it unlimited. Feature limits never exceed what the master has left unused.
pub fn remaining(
    config: &ServerSplitterSettingsData,
    master: &Limits,
    master_usage: &FeatureUsage,
    master_disk_usage_mb: i64,
    split: Option<&Limits>,
    transferable_allocation: bool,
) -> Limits {
    let held = split.copied().unwrap_or_default();

    let resource = |limit: i64, reserved: i64, used: i64, held: i64| {
        if limit > 0 {
            (limit - reserved.max(1) - used).max(0) + held
        } else {
            -1
        }
    };
    let feature =
        |limit: i32, used: i64, held: i32| ((limit as i64 - used).max(0) + held as i64) as i32;

    // The split's primary allocation either moves over from the master (freeing one of the
    // master's allocations) or comes from the node (using up one of the master's free slots).
    let free_allocations = (master.allocations as i64 - master_usage.allocations).max(0)
        + transferable_allocation as i64;
    let allocations =
        free_allocations.min(master.allocations.max(0) as i64) + held.allocations as i64;

    Limits {
        cpu: resource(
            master.cpu as i64,
            config.reserved_cpu as i64,
            0,
            held.cpu as i64,
        ) as i32,
        memory: resource(master.memory, config.reserved_memory, 0, held.memory),
        disk: resource(
            master.disk,
            config.reserved_disk,
            master_disk_usage_mb,
            held.disk,
        ),
        allocations: allocations as i32,
        databases: feature(master.databases, master_usage.databases, held.databases),
        backups: feature(master.backups, master_usage.backups, held.backups),
        schedules: feature(master.schedules, master_usage.schedules, held.schedules),
    }
}

/// Why `requested` doesn't fit in `remaining`, if it doesn't.
pub fn exceeded(requested: &Limits, remaining: &Limits) -> Option<&'static str> {
    let over = |requested: i64, remaining: i64| remaining >= 0 && requested > remaining;

    if over(requested.cpu as i64, remaining.cpu as i64) {
        Some("CPU limit exceeded.")
    } else if over(requested.memory, remaining.memory) {
        Some("Memory limit exceeded.")
    } else if over(requested.disk, remaining.disk) {
        Some("Disk limit exceeded.")
    } else if over(requested.allocations as i64, remaining.allocations as i64) {
        Some("Allocation limit exceeded.")
    } else if over(requested.databases as i64, remaining.databases as i64) {
        Some("Database limit exceeded.")
    } else if over(requested.backups as i64, remaining.backups as i64) {
        Some("Backup limit exceeded.")
    } else if over(requested.schedules as i64, remaining.schedules as i64) {
        Some("Schedule limit exceeded.")
    } else {
        None
    }
}

/// Why a split can't shrink its feature limits to `requested`, if it can't. The panel only checks
/// limits when something is created, so a limit below current usage would hand the difference
/// back to the master while the split keeps what it has. Limits equal to `current` are kept.
pub fn below_usage(requested: &Limits, current: &Limits, usage: &FeatureUsage) -> Option<String> {
    let checks = [
        (
            "allocations",
            requested.allocations,
            current.allocations,
            usage.allocations,
        ),
        (
            "databases",
            requested.databases,
            current.databases,
            usage.databases,
        ),
        ("backups", requested.backups, current.backups, usage.backups),
        (
            "schedules",
            requested.schedules,
            current.schedules,
            usage.schedules,
        ),
    ];

    checks
        .into_iter()
        .find(|(_, requested, current, used)| requested != current && (*requested as i64) < *used)
        .map(|(name, _, _, used)| {
            format!(
                "The split currently uses {used} {name}; remove some before lowering its limit."
            )
        })
}

/// Locks the master row and reads it fresh. `None` when the master no longer exists.
pub async fn lock_master(
    transaction: &mut Transaction<'_, Postgres>,
    master_uuid: uuid::Uuid,
) -> Result<Option<Server>, DatabaseError> {
    let locked = sqlx::query("SELECT 1 FROM servers WHERE uuid = $1 FOR NO KEY UPDATE")
        .bind(master_uuid)
        .fetch_optional(&mut **transaction)
        .await?;
    if locked.is_none() {
        return Ok(None);
    }

    Server::by_uuid_optional_with_transaction(transaction, master_uuid).await
}

/// Takes `amount` out of the master's pool (a negative field gives it back). Returns `false`,
/// changing nothing, when that would leave a limited master unlimited or a feature limit
/// negative; callers validate against [`remaining`] first, so that means the pool changed.
pub async fn debit_master(
    transaction: &mut Transaction<'_, Postgres>,
    master_uuid: uuid::Uuid,
    amount: &Limits,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        r#"
        UPDATE servers
        SET
            cpu = CASE WHEN cpu > 0 THEN cpu - $1 ELSE cpu END,
            memory = CASE WHEN memory > 0 THEN memory - $2 ELSE memory END,
            disk = CASE WHEN disk > 0 THEN disk - $3 ELSE disk END,
            allocation_limit = allocation_limit - $4,
            database_limit = database_limit - $5,
            backup_limit = backup_limit - $6,
            schedule_limit = schedule_limit - $7
        WHERE uuid = $8
            AND (cpu = 0 OR cpu - $1 > 0)
            AND (memory = 0 OR memory - $2 > 0)
            AND (disk = 0 OR disk - $3 > 0)
            AND allocation_limit - $4 >= 0
            AND database_limit - $5 >= 0
            AND backup_limit - $6 >= 0
            AND schedule_limit - $7 >= 0
        "#,
    )
    .bind(amount.cpu)
    .bind(amount.memory)
    .bind(amount.disk)
    .bind(amount.allocations)
    .bind(amount.databases)
    .bind(amount.backups)
    .bind(amount.schedules)
    .bind(master_uuid)
    .execute(&mut **transaction)
    .await?;

    Ok(result.rows_affected() == 1)
}

/// Returns everything `split_uuid` holds to its master. Reads the split's limits in the statement
/// itself, after the caller has locked the master, so a resize that committed while this
/// transaction waited for the lock is credited at its new size.
pub async fn credit_master(
    transaction: &mut Transaction<'_, Postgres>,
    split_uuid: uuid::Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE servers p
        SET
            cpu = CASE WHEN p.cpu > 0 AND s.cpu > 0 THEN p.cpu + s.cpu ELSE p.cpu END,
            memory = CASE WHEN p.memory > 0 AND s.memory > 0 THEN p.memory + s.memory ELSE p.memory END,
            disk = CASE WHEN p.disk > 0 AND s.disk > 0 THEN p.disk + s.disk ELSE p.disk END,
            allocation_limit = p.allocation_limit + s.allocation_limit,
            database_limit = p.database_limit + s.database_limit,
            backup_limit = p.backup_limit + s.backup_limit,
            schedule_limit = p.schedule_limit + s.schedule_limit
        FROM servers s
        WHERE s.uuid = $1 AND p.uuid = s.parent_uuid
        "#,
    )
    .bind(split_uuid)
    .execute(&mut **transaction)
    .await?;

    Ok(())
}

/// Invalidates the cached copies of `uuids` and pushes their current limits to Wings.
pub async fn refresh_and_sync(state: &State, uuids: &[uuid::Uuid]) {
    let database = std::sync::Arc::new(state.database.clone());

    for &uuid in uuids {
        Server::invalidate_cached(&state.database, uuid).await;

        match Server::by_uuid_optional(&state.database, uuid).await {
            Ok(Some(server)) => server.batch_sync(&database).await,
            Ok(None) => {}
            Err(err) => tracing::warn!(server = %uuid, "failed to reload server for sync: {err:?}"),
        }
    }
}

/// Syncs the master once the transaction currently holding its row lock ends. For hooks that run
/// inside a panel transaction and have no after-commit callback: waiting for the lock is the
/// commit signal.
pub fn refresh_and_sync_after_unlock(state: State, master_uuid: uuid::Uuid) {
    tokio::spawn(async move {
        let wait = async {
            let mut transaction = state.database.write().begin().await?;
            sqlx::query("SELECT 1 FROM servers WHERE uuid = $1 FOR SHARE")
                .bind(master_uuid)
                .execute(&mut *transaction)
                .await?;
            transaction.commit().await
        };

        if let Err(err) = wait.await {
            tracing::warn!(server = %master_uuid, "failed to wait for master unlock: {err:?}");
        }

        refresh_and_sync(&state, &[master_uuid]).await;
    });
}

/// Set while [`Server::create`] runs for a new split, so the extension's create handler inserts
/// the server already linked to its master and takes over the master's allocation in the same
/// transaction.
#[derive(Clone, Copy, Debug)]
pub struct PendingSplit {
    pub master_uuid: uuid::Uuid,
    pub transferred_server_allocation: Option<uuid::Uuid>,
}

tokio::task_local! {
    pub static PENDING_SPLIT: PendingSplit;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(cpu: i32, memory: i64, disk: i64) -> ServerSplitterSettingsData {
        ServerSplitterSettingsData {
            reserved_cpu: cpu,
            reserved_memory: memory,
            reserved_disk: disk,
            ..Default::default()
        }
    }

    fn limits(cpu: i32, memory: i64, disk: i64, features: i32) -> Limits {
        Limits {
            cpu,
            memory,
            disk,
            allocations: features,
            databases: features,
            backups: features,
            schedules: features,
        }
    }

    // 0 means unlimited: taking a limited master's whole limit would turn it unlimited.
    #[test]
    fn limited_master_keeps_at_least_one_without_a_reserve() {
        let master = limits(400, 4096, 10_000, 0);
        let pool = remaining(
            &config(0, 0, 0),
            &master,
            &FeatureUsage::default(),
            0,
            None,
            false,
        );

        assert_eq!((pool.cpu, pool.memory, pool.disk), (399, 4095, 9_999));
        assert!(exceeded(&limits(400, 1, 1, 0), &pool).is_some());
        assert!(exceeded(&limits(1, 4096, 1, 0), &pool).is_some());
        assert!(exceeded(&limits(1, 1, 10_000, 0), &pool).is_some());
    }

    #[test]
    fn reserve_and_disk_usage_stay_with_the_master() {
        let master = limits(400, 4096, 10_000, 0);
        let pool = remaining(
            &config(10, 128, 256),
            &master,
            &FeatureUsage::default(),
            1_000,
            None,
            false,
        );

        assert_eq!((pool.cpu, pool.memory, pool.disk), (390, 3968, 8_744));
    }

    #[test]
    fn unlimited_master_resources_stay_unlimited() {
        let master = limits(0, 0, 0, 0);
        let pool = remaining(
            &config(10, 128, 256),
            &master,
            &FeatureUsage::default(),
            500,
            None,
            false,
        );

        assert_eq!((pool.cpu, pool.memory, pool.disk), (-1, -1, -1));
        assert!(exceeded(&limits(10_000, 1 << 40, 1 << 40, 0), &pool).is_none());
    }

    // A resized split can keep what it holds, even when the master has nothing left to give.
    #[test]
    fn resize_adds_the_split_holdings_back() {
        let master = limits(10, 128, 256, 0);
        let split = limits(200, 2048, 4096, 2);
        let pool = remaining(
            &config(10, 128, 256),
            &master,
            &FeatureUsage::default(),
            0,
            Some(&split),
            false,
        );

        assert_eq!((pool.cpu, pool.memory, pool.disk), (200, 2048, 4096));
        assert_eq!(pool.databases, 2);
    }

    #[test]
    fn feature_limits_exclude_what_the_master_uses() {
        let master = limits(0, 0, 0, 5);
        let usage = FeatureUsage {
            allocations: 1,
            databases: 3,
            backups: 5,
            schedules: 7,
        };
        let pool = remaining(&config(0, 0, 0), &master, &usage, 0, None, false);

        assert_eq!(pool.allocations, 4);
        assert_eq!(pool.databases, 2);
        assert_eq!(pool.backups, 0);
        assert_eq!(pool.schedules, 0);
    }

    // Only one of the master's extra allocations moves to the split, so extras the master keeps
    // don't count as free quota.
    #[test]
    fn allocations_count_one_transferred_allocation() {
        let usage = FeatureUsage {
            allocations: 3,
            ..Default::default()
        };

        // Limit 3, holding primary + 2 extras: one moves over, nothing else is free.
        let pool = remaining(&config(0, 0, 0), &limits(0, 0, 0, 3), &usage, 0, None, true);
        assert_eq!(pool.allocations, 1);

        // Admin-assigned extras beyond a limit of 0 can't create quota out of nothing.
        let pool = remaining(&config(0, 0, 0), &limits(0, 0, 0, 0), &usage, 0, None, true);
        assert_eq!(pool.allocations, 0);
    }

    #[test]
    fn shrinking_below_usage_is_rejected() {
        let usage = FeatureUsage {
            allocations: 1,
            databases: 3,
            backups: 0,
            schedules: 0,
        };

        let current = limits(0, 0, 0, 5);
        assert!(below_usage(&limits(0, 0, 0, 2), &current, &usage).is_some());
        assert!(below_usage(&limits(0, 0, 0, 3), &current, &usage).is_none());
        // a limit that stays below usage (e.g. set by an admin) doesn't block other changes
        assert!(below_usage(&limits(0, 0, 0, 2), &limits(0, 0, 0, 2), &usage).is_none());
    }
}
