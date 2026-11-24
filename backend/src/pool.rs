//! A master server's resource pool: what its splits may take, the ledger of what each split was granted,
//! and the locked debits and credits that move resources between the master and its splits.
//!
//! Every change to a master's pool first locks the master row (`FOR NO KEY UPDATE`) and re-reads
//! it inside that transaction. The lock serializes concurrent split operations on one master,
//! and the fresh read bypasses the panel's server cache, which raw `UPDATE`s here don't touch.
//! `FOR NO KEY UPDATE` still lets other transactions insert rows referencing the master, which
//! [`Server::create`] does while a split is being created under the lock.

use crate::settings::{ExternalResize, ServerSplitterSettingsData, SwapMode};
use shared::{
    State,
    database::DatabaseError,
    models::{ByUuid, server::Server, server_backup::ServerBackup},
};
use sqlx::{PgConnection, Postgres, Row, Transaction};

/// cpu (%), memory, swap and disk (MB), and feature limits. For cpu/memory/disk 0 means unlimited on a
/// server, and -1 means unlimited in a pool computed by [`remaining`]. Swap is -1 for unlimited, 0 for none.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Limits {
    pub cpu: i32,
    pub memory: i64,
    pub swap: i64,
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
            swap: server.swap,
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
            swap: self.swap - other.swap,
            disk: self.disk - other.disk,
            allocations: self.allocations - other.allocations,
            databases: self.databases - other.databases,
            backups: self.backups - other.backups,
            schedules: self.schedules - other.schedules,
        }
    }

    pub fn plus(&self, other: &Self) -> Self {
        Self {
            cpu: self.cpu + other.cpu,
            memory: self.memory + other.memory,
            swap: self.swap + other.swap,
            disk: self.disk + other.disk,
            allocations: self.allocations + other.allocations,
            databases: self.databases + other.databases,
            backups: self.backups + other.backups,
            schedules: self.schedules + other.schedules,
        }
    }

    /// Fields that differ, as `name: self -> other`.
    pub fn differences(&self, other: &Self) -> Vec<String> {
        let fields: [(&str, i64, i64); 8] = [
            ("cpu", self.cpu as i64, other.cpu as i64),
            ("memory", self.memory, other.memory),
            ("swap", self.swap, other.swap),
            ("disk", self.disk, other.disk),
            ("allocations", self.allocations as i64, other.allocations as i64),
            ("databases", self.databases as i64, other.databases as i64),
            ("backups", self.backups as i64, other.backups as i64),
            ("schedules", self.schedules as i64, other.schedules as i64),
        ];

        fields
            .into_iter()
            .filter(|(_, a, b)| a != b)
            .map(|(name, a, b)| format!("{name}: {a} -> {b}"))
            .collect()
    }
}

/// What of a split's cpu/memory/swap/disk was taken from its master's limits. Feature limits are
/// always taken in full.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Charged {
    pub cpu: i32,
    pub memory: i64,
    pub swap: i64,
    pub disk: i64,
}

impl Charged {
    pub fn plus(&self, other: &Self) -> Self {
        Self {
            cpu: self.cpu + other.cpu,
            memory: self.memory + other.memory,
            swap: self.swap + other.swap,
            disk: self.disk + other.disk,
        }
    }
}

/// What the splits of an over-committed master still have to free: its plan total was set below what
/// they were charged plus the master's reserve, so the master is held above its share of the plan.
/// Resources given back by its splits pay this down before they reach the master.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Overcommit {
    pub cpu: i32,
    pub memory: i64,
    pub swap: i64,
    pub disk: i64,
}

impl Overcommit {
    pub fn is_zero(&self) -> bool {
        *self == Self::default()
    }
}

/// One split in the ledger: what it was granted and what of that its master was charged.
#[derive(Clone, Copy, Debug)]
pub struct LedgerEntry {
    pub split_uuid: uuid::Uuid,
    pub master_uuid: uuid::Uuid,
    pub granted: Limits,
    pub charged: Charged,
    pub port_from_master: bool,
}

impl LedgerEntry {
    /// What deleting the split gives back to its master, as a (negative) debit.
    pub fn refund(&self) -> Limits {
        Limits {
            cpu: -self.charged.cpu,
            memory: -self.charged.memory,
            swap: -self.charged.swap,
            disk: -self.charged.disk,
            allocations: -self.granted.allocations,
            databases: -self.granted.databases,
            backups: -self.granted.backups,
            schedules: -self.granted.schedules,
        }
    }
}

fn map_ledger(row: &sqlx::postgres::PgRow) -> LedgerEntry {
    LedgerEntry {
        split_uuid: row.get("split_uuid"),
        master_uuid: row.get("master_uuid"),
        granted: Limits {
            cpu: row.get("cpu"),
            memory: row.get("memory"),
            swap: row.get("swap"),
            disk: row.get("disk"),
            allocations: row.get("allocations"),
            databases: row.get("databases"),
            backups: row.get("backups"),
            schedules: row.get("schedules"),
        },
        charged: Charged {
            cpu: row.get("charged_cpu"),
            memory: row.get("charged_memory"),
            swap: row.get("charged_swap"),
            disk: row.get("charged_disk"),
        },
        port_from_master: row.get("port_from_master"),
    }
}

const LEDGER_COLUMNS: &str = "split_uuid, master_uuid, cpu, memory, swap, disk, allocations, databases, backups, schedules, charged_cpu, charged_memory, charged_swap, charged_disk, port_from_master";

pub async fn ledger_entry(
    conn: &mut PgConnection,
    split_uuid: uuid::Uuid,
) -> Result<Option<LedgerEntry>, sqlx::Error> {
    let row = sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT {LEDGER_COLUMNS} FROM serversplitter_splits WHERE split_uuid = $1"
    )))
    .bind(split_uuid)
    .fetch_optional(conn)
    .await?;

    Ok(row.as_ref().map(map_ledger))
}

pub async fn ledger_by_master(
    conn: &mut PgConnection,
    master_uuid: uuid::Uuid,
) -> Result<Vec<LedgerEntry>, sqlx::Error> {
    let rows = sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT {LEDGER_COLUMNS} FROM serversplitter_splits WHERE master_uuid = $1 ORDER BY created ASC"
    )))
    .bind(master_uuid)
    .fetch_all(conn)
    .await?;

    Ok(rows.iter().map(map_ledger).collect())
}

/// Inserts or replaces a split's ledger entry.
pub async fn save_ledger(conn: &mut PgConnection, entry: &LedgerEntry) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO serversplitter_splits (
            split_uuid, master_uuid, cpu, memory, swap, disk, allocations, databases, backups, schedules,
            charged_cpu, charged_memory, charged_swap, charged_disk, port_from_master
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
        ON CONFLICT (split_uuid) DO UPDATE SET
            master_uuid = EXCLUDED.master_uuid,
            cpu = EXCLUDED.cpu,
            memory = EXCLUDED.memory,
            swap = EXCLUDED.swap,
            disk = EXCLUDED.disk,
            allocations = EXCLUDED.allocations,
            databases = EXCLUDED.databases,
            backups = EXCLUDED.backups,
            schedules = EXCLUDED.schedules,
            charged_cpu = EXCLUDED.charged_cpu,
            charged_memory = EXCLUDED.charged_memory,
            charged_swap = EXCLUDED.charged_swap,
            charged_disk = EXCLUDED.charged_disk,
            port_from_master = EXCLUDED.port_from_master,
            updated = NOW()
        "#,
    )
    .bind(entry.split_uuid)
    .bind(entry.master_uuid)
    .bind(entry.granted.cpu)
    .bind(entry.granted.memory)
    .bind(entry.granted.swap)
    .bind(entry.granted.disk)
    .bind(entry.granted.allocations)
    .bind(entry.granted.databases)
    .bind(entry.granted.backups)
    .bind(entry.granted.schedules)
    .bind(entry.charged.cpu)
    .bind(entry.charged.memory)
    .bind(entry.charged.swap)
    .bind(entry.charged.disk)
    .bind(entry.port_from_master)
    .execute(conn)
    .await?;

    Ok(())
}

/// What to debit from one master resource when a split's grant changes by `delta`, given the split
/// was charged `charged` for it so far. Returns `(debit, new_charged)`; a negative debit gives back.
/// A master that isn't limited for the resource is never charged, and a shrink gives back at most what
/// was charged, so the pool can't grow out of nothing.
pub fn charge(master_limited: bool, charged: i64, delta: i64) -> (i64, i64) {
    if delta >= 0 {
        if master_limited {
            (delta, charged + delta)
        } else {
            (0, charged)
        }
    } else {
        let back = (-delta).min(charged.max(0));
        (-back, charged - back)
    }
}

/// Moves a split from `old` to `new` granted limits against `master` (its locked, fresh limits).
/// Returns the debit to apply to the master and the split's new charged amounts.
pub fn regrant(
    master: &Limits,
    swap_pooled: bool,
    old: &Limits,
    charged: &Charged,
    new: &Limits,
) -> (Limits, Charged) {
    let (cpu, charged_cpu) = charge(
        master.cpu > 0,
        charged.cpu as i64,
        (new.cpu - old.cpu) as i64,
    );
    let (memory, charged_memory) = charge(master.memory > 0, charged.memory, new.memory - old.memory);
    let (swap, charged_swap) = charge(
        swap_pooled && master.swap >= 0,
        charged.swap,
        new.swap - old.swap,
    );
    let (disk, charged_disk) = charge(master.disk > 0, charged.disk, new.disk - old.disk);

    (
        Limits {
            cpu: cpu as i32,
            memory,
            swap,
            disk,
            allocations: new.allocations - old.allocations,
            databases: new.databases - old.databases,
            backups: new.backups - old.backups,
            schedules: new.schedules - old.schedules,
        },
        Charged {
            cpu: charged_cpu as i32,
            memory: charged_memory,
            swap: charged_swap,
            disk: charged_disk,
        },
    )
}

/// One resource of a master set from outside the splitter, read as a plan total: the master keeps
/// `total` minus what its splits were charged, but never less than `floor` (its reserve) or more than
/// `total` itself. Returns `(master, overcommit)`; the overcommit is what the splits hold beyond the
/// plan, so `master + charged - overcommit == total`. An unlimited total stays unlimited.
pub fn plan_share(total: i64, charged: i64, floor: i64, unlimited: bool) -> (i64, i64) {
    if unlimited {
        return (total, 0);
    }
    let share = total - charged.max(0);
    let master = share.max(floor.min(total));
    (master, master - share)
}

/// A master's cpu/memory/swap/disk after an edit from outside the splitter set them to `requested`.
/// `current` and `owed` are its locked values now, `charged` what its splits were charged in total.
/// Fields equal to `current` stay as they are, overcommit included: the panel's admin form sends every
/// limit on each save. Changed fields are plan totals with [`ExternalResize::PlanTotal`] (swap only
/// when pooled), else what the master keeps, which clears their overcommit. Other fields are `current`.
pub fn resize_master(
    config: &ServerSplitterSettingsData,
    requested: &Limits,
    current: &Limits,
    charged: &Charged,
    owed: &Overcommit,
) -> (Limits, Overcommit) {
    let plan = config.external_resize == ExternalResize::PlanTotal;
    let field = |requested: i64,
                 current: i64,
                 charged: i64,
                 owed: i64,
                 floor: i64,
                 unlimited: bool,
                 plan: bool| {
        if requested == current {
            (current, owed)
        } else if !plan {
            (requested, 0)
        } else {
            plan_share(requested, charged, floor, unlimited)
        }
    };

    let (cpu, owed_cpu) = field(
        requested.cpu as i64,
        current.cpu as i64,
        charged.cpu as i64,
        owed.cpu as i64,
        config.reserved_cpu.max(1) as i64,
        requested.cpu <= 0,
        plan,
    );
    let (memory, owed_memory) = field(
        requested.memory,
        current.memory,
        charged.memory,
        owed.memory,
        config.reserved_memory.max(1),
        requested.memory <= 0,
        plan,
    );
    let (swap, owed_swap) = field(
        requested.swap,
        current.swap,
        charged.swap,
        owed.swap,
        0,
        requested.swap < 0,
        plan && config.swap_mode == SwapMode::Pooled,
    );
    let (disk, owed_disk) = field(
        requested.disk,
        current.disk,
        charged.disk,
        owed.disk,
        config.reserved_disk.max(1),
        requested.disk <= 0,
        plan,
    );

    (
        Limits {
            cpu: cpu as i32,
            memory,
            swap,
            disk,
            ..*current
        },
        Overcommit {
            cpu: owed_cpu as i32,
            memory: owed_memory,
            swap: owed_swap,
            disk: owed_disk,
        },
    )
}

/// A master's plan total: what it keeps plus what its splits were charged, less what they still owe.
/// cpu/memory/disk 0 and swap -1 are unlimited, as on the master.
pub fn plan_total(master: &Limits, charged: &Charged, owed: &Overcommit) -> Charged {
    let total = |master: i64, charged: i64, owed: i64, limited: bool, unlimited: i64| {
        if limited {
            master + charged - owed
        } else {
            unlimited
        }
    };

    Charged {
        cpu: total(
            master.cpu as i64,
            charged.cpu as i64,
            owed.cpu as i64,
            master.cpu > 0,
            0,
        ) as i32,
        memory: total(master.memory, charged.memory, owed.memory, master.memory > 0, 0),
        swap: total(master.swap, charged.swap, owed.swap, master.swap >= 0, -1),
        disk: total(master.disk, charged.disk, owed.disk, master.disk > 0, 0),
    }
}

/// Splits through what a debit gives back to an over-committed master: what is owed is paid down
/// first, only the rest reaches the master. Returns the debit to apply and what is still owed.
pub fn pay_down(amount: &Limits, owed: &Overcommit) -> (Limits, Overcommit) {
    let field = |amount: i64, owed: i64| {
        if amount < 0 && owed > 0 {
            let paid = (-amount).min(owed);
            (amount + paid, owed - paid)
        } else {
            (amount, owed)
        }
    };

    let (cpu, owed_cpu) = field(amount.cpu as i64, owed.cpu as i64);
    let (memory, owed_memory) = field(amount.memory, owed.memory);
    let (swap, owed_swap) = field(amount.swap, owed.swap);
    let (disk, owed_disk) = field(amount.disk, owed.disk);

    (
        Limits {
            cpu: cpu as i32,
            memory,
            swap,
            disk,
            ..*amount
        },
        Overcommit {
            cpu: owed_cpu as i32,
            memory: owed_memory,
            swap: owed_swap,
            disk: owed_disk,
        },
    )
}

/// The swap a split with `memory` gets. `held` is the swap it already holds of its master's (it goes
/// back into the pool on a resize).
pub fn split_swap(mode: SwapMode, master_swap: i64, memory: i64, held: i64) -> i64 {
    match mode {
        SwapMode::None => 0,
        SwapMode::Copy => master_swap,
        SwapMode::Pooled => {
            let wanted = memory.max(0) / 4;
            if master_swap < 0 {
                wanted
            } else {
                wanted.min(master_swap + held.max(0))
            }
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
                    AND (deleting IS NULL OR deletion_retries >= $2)
            ) AS backups,
            (SELECT count(*) FROM server_schedules WHERE server_uuid = $1) AS schedules
        "#,
    )
    .bind(server_uuid)
    .bind(ServerBackup::MAX_DELETION_RETRIES)
    .fetch_one(conn)
    .await?;

    Ok(FeatureUsage {
        allocations: row.get("allocations"),
        databases: row.get("databases"),
        backups: row.get("backups"),
        schedules: row.get("schedules"),
    })
}

/// A non-primary allocation of the master that a new split can take over as its primary one.
#[derive(Clone, Debug)]
pub struct TransferableAllocation {
    pub server_allocation_uuid: uuid::Uuid,
    pub allocation_uuid: uuid::Uuid,
    pub ip: String,
    pub ip_alias: Option<String>,
    pub port: i32,
    pub notes: Option<String>,
    pub created: chrono::NaiveDateTime,
}

pub async fn transferable_allocations(
    conn: &mut PgConnection,
    master_uuid: uuid::Uuid,
) -> Result<Vec<TransferableAllocation>, sqlx::Error> {
    let rows = sqlx::query(
        r#"
        SELECT sa.uuid, sa.allocation_uuid, host(na.ip) AS ip, na.ip_alias, na.port, sa.notes, sa.created
        FROM server_allocations sa
        JOIN servers s ON s.uuid = sa.server_uuid
        JOIN node_allocations na ON na.uuid = sa.allocation_uuid
        WHERE sa.server_uuid = $1
          AND (s.allocation_uuid IS NULL OR sa.uuid != s.allocation_uuid)
        ORDER BY sa.created ASC
        "#,
    )
    .bind(master_uuid)
    .fetch_all(conn)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| TransferableAllocation {
            server_allocation_uuid: row.get("uuid"),
            allocation_uuid: row.get("allocation_uuid"),
            ip: row.get("ip"),
            ip_alias: row.get("ip_alias"),
            port: row.get("port"),
            notes: row.get("notes"),
            created: row.get("created"),
        })
        .collect())
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

/// Moves a deleted split's ports back to its master, up to the master's free allocation slots. Runs in
/// the split's delete transaction, before the split's row goes. The split's primary allocation is
/// cleared first so the panel's delete doesn't take it along.
pub async fn return_ports(
    transaction: &mut Transaction<'_, Postgres>,
    split_uuid: uuid::Uuid,
    master_uuid: uuid::Uuid,
) -> Result<u64, sqlx::Error> {
    let free: i64 = sqlx::query_scalar(
        r#"
        SELECT GREATEST(
            s.allocation_limit - (SELECT count(*) FROM server_allocations WHERE server_uuid = s.uuid),
            0
        )
        FROM servers s
        WHERE s.uuid = $1
        "#,
    )
    .bind(master_uuid)
    .fetch_optional(&mut **transaction)
    .await?
    .unwrap_or(0);
    if free == 0 {
        return Ok(0);
    }

    sqlx::query("UPDATE servers SET allocation_uuid = NULL WHERE uuid = $1")
        .bind(split_uuid)
        .execute(&mut **transaction)
        .await?;

    // the split's primary port first, then the others oldest first
    let moved = sqlx::query(
        r#"
        UPDATE server_allocations
        SET server_uuid = $2
        WHERE uuid IN (
            SELECT sa.uuid
            FROM server_allocations sa
            WHERE sa.server_uuid = $1
            ORDER BY sa.created ASC
            LIMIT $3
        )
        "#,
    )
    .bind(split_uuid)
    .bind(master_uuid)
    .bind(free)
    .execute(&mut **transaction)
    .await?;

    Ok(moved.rows_affected())
}

/// Disk the master's files currently use, in MB, when the config counts it against the pool. `None`
/// when it should be counted but Wings couldn't be asked.
pub async fn disk_usage_mb(
    state: &State,
    master: &Server,
    config: &ServerSplitterSettingsData,
) -> Option<i64> {
    if !config.include_disk_usage {
        return Some(0);
    }

    let node = master.node.fetch_cached(&state.database).await.ok()?;
    let resources = node.fetch_server_resources(&state.database).await.ok()?;

    Some(
        resources
            .get(&master.uuid)
            .map(|r| (r.disk_bytes / 1024 / 1024) as i64)
            .unwrap_or(0),
    )
}

/// The most a split may hold of each resource. `granted` is what the split being resized holds now
/// (it goes back into the pool); `transferable_allocation` is whether a new split takes over one of the
/// master's existing allocations. Swap isn't bounded here, see [`split_swap`].
///
/// A limited master always keeps at least 1 cpu/memory/disk (or its reserve, if larger): 0 would
/// make it unlimited. Feature limits never exceed what the master has left unused.
pub fn remaining(
    config: &ServerSplitterSettingsData,
    master: &Limits,
    master_usage: &FeatureUsage,
    master_disk_usage_mb: i64,
    granted: Option<&Limits>,
    transferable_allocation: bool,
) -> Limits {
    let held = granted.copied().unwrap_or_default();

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
        swap: 0,
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

/// Why `requested` doesn't fit in `remaining`, if it doesn't. Swap isn't checked.
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

/// Lower bounds for a split's limits: the minimum size where the master is limited (a smaller current
/// value may be kept), and what it uses of its feature limits (a lower current limit may be kept).
pub fn resize_minimum(
    config: &ServerSplitterSettingsData,
    master: &Limits,
    current: &Limits,
    usage: &FeatureUsage,
) -> Limits {
    let resource = |master: i64, minimum: i64, current: i64| {
        if master > 0 {
            minimum.min(if current > 0 { current } else { minimum })
        } else {
            0
        }
    };
    let feature = |used: i64, current: i32| (used.min(current as i64)) as i32;

    Limits {
        cpu: resource(master.cpu as i64, config.min_cpu() as i64, current.cpu as i64) as i32,
        memory: resource(master.memory, config.min_memory(), current.memory),
        swap: 0,
        disk: resource(master.disk, config.min_disk(), current.disk),
        allocations: feature(usage.allocations, current.allocations).max(1),
        databases: feature(usage.databases, current.databases),
        backups: feature(usage.backups, current.backups),
        schedules: feature(usage.schedules, current.schedules),
    }
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

/// What the splits of `master_uuid` still owe it (zero when it isn't over-committed).
pub async fn overcommit(
    conn: &mut PgConnection,
    master_uuid: uuid::Uuid,
) -> Result<Overcommit, sqlx::Error> {
    let row = sqlx::query(
        "SELECT cpu, memory, swap, disk FROM serversplitter_overcommit WHERE master_uuid = $1",
    )
    .bind(master_uuid)
    .fetch_optional(conn)
    .await?;

    Ok(row
        .map(|row| Overcommit {
            cpu: row.get("cpu"),
            memory: row.get("memory"),
            swap: row.get("swap"),
            disk: row.get("disk"),
        })
        .unwrap_or_default())
}

/// Records what the splits of `master_uuid` owe it; zero drops the record. The caller has locked the
/// master.
pub async fn save_overcommit(
    conn: &mut PgConnection,
    master_uuid: uuid::Uuid,
    owed: &Overcommit,
) -> Result<(), sqlx::Error> {
    if owed.is_zero() {
        sqlx::query("DELETE FROM serversplitter_overcommit WHERE master_uuid = $1")
            .bind(master_uuid)
            .execute(conn)
            .await?;
        return Ok(());
    }

    sqlx::query(
        r#"
        INSERT INTO serversplitter_overcommit (master_uuid, cpu, memory, swap, disk)
        VALUES ($1, $2, $3, $4, $5)
        ON CONFLICT (master_uuid) DO UPDATE SET
            cpu = EXCLUDED.cpu,
            memory = EXCLUDED.memory,
            swap = EXCLUDED.swap,
            disk = EXCLUDED.disk,
            updated = NOW()
        "#,
    )
    .bind(master_uuid)
    .bind(owed.cpu)
    .bind(owed.memory)
    .bind(owed.swap)
    .bind(owed.disk)
    .execute(conn)
    .await?;

    Ok(())
}

/// Takes `amount` out of the master's limits (a negative field gives it back). Returns `false`,
/// changing nothing, when that would leave a limited master unlimited, its swap negative or a feature
/// limit negative; callers validate against [`remaining`] first, so that means the pool changed.
/// cpu/memory/disk of an unlimited master are left alone; [`charge`] never charges them. What is
/// given back to an over-committed master pays down its overcommit first ([`pay_down`]).
pub async fn debit_master(
    transaction: &mut Transaction<'_, Postgres>,
    master_uuid: uuid::Uuid,
    amount: &Limits,
) -> Result<bool, sqlx::Error> {
    let owed = overcommit(&mut **transaction, master_uuid).await?;
    let (amount, still_owed) = pay_down(amount, &owed);

    let result = sqlx::query(
        r#"
        UPDATE servers
        SET
            cpu = CASE WHEN cpu > 0 THEN cpu - $1 ELSE cpu END,
            memory = CASE WHEN memory > 0 THEN memory - $2 ELSE memory END,
            swap = CASE WHEN swap >= 0 THEN swap - $3 ELSE swap END,
            disk = CASE WHEN disk > 0 THEN disk - $4 ELSE disk END,
            allocation_limit = allocation_limit - $5,
            database_limit = database_limit - $6,
            backup_limit = backup_limit - $7,
            schedule_limit = schedule_limit - $8
        WHERE uuid = $9
            AND (cpu = 0 OR cpu - $1 > 0)
            AND (memory = 0 OR memory - $2 > 0)
            AND (swap < 0 OR swap - $3 >= 0)
            AND (disk = 0 OR disk - $4 > 0)
            AND allocation_limit - $5 >= 0
            AND database_limit - $6 >= 0
            AND backup_limit - $7 >= 0
            AND schedule_limit - $8 >= 0
        "#,
    )
    .bind(amount.cpu)
    .bind(amount.memory)
    .bind(amount.swap)
    .bind(amount.disk)
    .bind(amount.allocations)
    .bind(amount.databases)
    .bind(amount.backups)
    .bind(amount.schedules)
    .bind(master_uuid)
    .execute(&mut **transaction)
    .await?;

    if result.rows_affected() != 1 {
        return Ok(false);
    }
    if still_owed != owed {
        save_overcommit(&mut **transaction, master_uuid, &still_owed).await?;
    }

    Ok(true)
}

/// Returns everything `split_uuid` holds to its master and drops its ledger entry. The caller has
/// locked the master. A split without a ledger entry (created before 1.5.3 and missed by the
/// migration) gives back its current limits the way 1.5.2 did.
pub async fn credit_master(
    transaction: &mut Transaction<'_, Postgres>,
    split_uuid: uuid::Uuid,
) -> Result<(), sqlx::Error> {
    let entry = sqlx::query(sqlx::AssertSqlSafe(format!(
        "DELETE FROM serversplitter_splits WHERE split_uuid = $1 RETURNING {LEDGER_COLUMNS}"
    )))
    .bind(split_uuid)
    .fetch_optional(&mut **transaction)
    .await?
    .as_ref()
    .map(map_ledger);

    if let Some(entry) = entry {
        let refund = entry.refund();
        if !debit_master(transaction, entry.master_uuid, &refund).await? {
            tracing::warn!(split = %split_uuid, master = %entry.master_uuid, "could not credit the master for a deleted split");
        }
        return Ok(());
    }

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
    for &uuid in uuids {
        Server::invalidate_cached(&state.database, uuid).await;

        match Server::by_uuid_optional(&state.database, uuid).await {
            Ok(Some(server)) => server.batch_sync(state).await,
            Ok(None) => {}
            Err(err) => tracing::warn!(server = %uuid, "failed to reload server for sync: {err:?}"),
        }
    }
}

/// Waits until the transaction currently holding `lock_uuid`'s row lock ends. For hooks that run
/// inside a panel transaction and have no after-commit callback: waiting for the lock is the commit
/// signal.
pub async fn wait_for_unlock(state: &State, lock_uuid: uuid::Uuid) {
    let wait = async {
        let mut transaction = state.database.write().begin().await?;
        sqlx::query("SELECT 1 FROM servers WHERE uuid = $1 FOR SHARE")
            .bind(lock_uuid)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await
    };

    if let Err(err) = wait.await {
        tracing::warn!(server = %lock_uuid, "failed to wait for server unlock: {err:?}");
    }
}

/// Syncs `uuids` once the transaction holding `lock_uuid`'s row lock ends. Servers that are gone by
/// then are skipped.
pub fn sync_after_unlock(state: State, lock_uuid: uuid::Uuid, uuids: Vec<uuid::Uuid>) {
    tokio::spawn(async move {
        wait_for_unlock(&state, lock_uuid).await;
        refresh_and_sync(&state, &uuids).await;
    });
}

/// Set while [`Server::create`] runs for a new split, so the extension's create handler inserts
/// the server already linked to its master, takes over the master's allocation and writes the
/// split's ledger entry in the same transaction.
#[derive(Clone, Copy, Debug)]
pub struct PendingSplit {
    pub master_uuid: uuid::Uuid,
    pub transferred_server_allocation: Option<uuid::Uuid>,
    pub granted: Limits,
    pub charged: Charged,
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
            swap: 0,
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

    #[test]
    fn charge_takes_from_limited_masters_only() {
        assert_eq!(charge(true, 100, 50), (50, 150));
        assert_eq!(charge(false, 100, 50), (0, 100));
    }

    // An admin raises a split while the master is unlimited, then the master becomes limited: shrinking
    // must not give back more than was ever taken (1.5.2 grew the pool here).
    #[test]
    fn shrink_gives_back_at_most_what_was_charged() {
        assert_eq!(charge(true, 100, -40), (-40, 60));
        assert_eq!(charge(true, 100, -400), (-100, 0));
        assert_eq!(charge(true, 0, -400), (0, 0));
    }

    #[test]
    fn refund_returns_charged_resources_and_granted_features() {
        let entry = LedgerEntry {
            split_uuid: uuid::Uuid::nil(),
            master_uuid: uuid::Uuid::nil(),
            granted: limits(100, 2048, 4096, 2),
            charged: Charged {
                cpu: 100,
                memory: 1024,
                swap: 256,
                disk: 0,
            },
            port_from_master: false,
        };
        let refund = entry.refund();
        assert_eq!(
            (refund.cpu, refund.memory, refund.swap, refund.disk),
            (-100, -1024, -256, 0)
        );
        assert_eq!(refund.allocations, -2);
    }

    #[test]
    fn regrant_moves_the_difference() {
        let master = limits(300, 4096, 8192, 4);
        let old = limits(100, 1024, 2048, 1);
        let charged = Charged {
            cpu: 100,
            memory: 1024,
            swap: 0,
            disk: 2048,
        };
        let new = limits(150, 512, 2048, 2);
        let (debit, charged) = regrant(&master, true, &old, &charged, &new);

        assert_eq!((debit.cpu, debit.memory, debit.disk), (50, -512, 0));
        assert_eq!(debit.allocations, 1);
        assert_eq!((charged.cpu, charged.memory, charged.disk), (150, 512, 2048));
    }

    #[test]
    fn pooled_swap_is_capped_by_the_master() {
        assert_eq!(split_swap(SwapMode::Pooled, 512, 4096, 0), 512);
        assert_eq!(split_swap(SwapMode::Pooled, 512, 1024, 0), 256);
        // what the split already holds counts as available on a resize
        assert_eq!(split_swap(SwapMode::Pooled, 0, 4096, 300), 300);
        assert_eq!(split_swap(SwapMode::Pooled, 0, 4096, 0), 0);
        assert_eq!(split_swap(SwapMode::Pooled, -1, 4096, 0), 1024);
        assert_eq!(split_swap(SwapMode::None, 512, 4096, 0), 0);
        assert_eq!(split_swap(SwapMode::Copy, 512, 4096, 0), 512);
    }

    #[test]
    fn resize_minimum_keeps_smaller_current_values() {
        let config = config(10, 128, 256);
        let master = limits(400, 4096, 8192, 4);
        let usage = FeatureUsage {
            allocations: 2,
            databases: 1,
            ..Default::default()
        };

        let min = resize_minimum(&config, &master, &limits(5, 512, 512, 3), &usage);
        assert_eq!((min.cpu, min.memory, min.disk), (5, 128, 256));
        assert_eq!((min.allocations, min.databases, min.backups), (2, 1, 0));

        let unlimited = resize_minimum(&config, &limits(0, 0, 0, 4), &limits(0, 0, 0, 1), &usage);
        assert_eq!((unlimited.cpu, unlimited.memory, unlimited.disk), (0, 0, 0));
    }

    fn charged(cpu: i32, memory: i64, swap: i64, disk: i64) -> Charged {
        Charged {
            cpu,
            memory,
            swap,
            disk,
        }
    }

    fn master(cpu: i32, memory: i64, swap: i64, disk: i64) -> Limits {
        Limits {
            swap,
            ..limits(cpu, memory, disk, 4)
        }
    }

    // A billing panel upgrades a 4 GB plan with a 1 GB split to 8 GB: the master keeps 7 GB, not 8.
    #[test]
    fn plan_upgrade_leaves_the_splits_share_out() {
        let current = master(300, 3072, 768, 15_360);
        let splits = charged(100, 1024, 256, 5_120);
        let (kept, owed) = resize_master(
            &config(10, 128, 256),
            &master(600, 8192, 2048, 40_960),
            &current,
            &splits,
            &Overcommit::default(),
        );

        assert_eq!(
            (kept.cpu, kept.memory, kept.swap, kept.disk),
            (500, 7168, 1792, 35_840)
        );
        assert!(owed.is_zero());
        // feature limits aren't part of the plan
        assert_eq!(kept.allocations, current.allocations);
        assert_eq!(
            plan_total(&kept, &splits, &owed),
            charged(600, 8192, 2048, 40_960)
        );
    }

    #[test]
    fn plan_downgrade_that_still_fits_shrinks_only_the_master() {
        let (kept, owed) = resize_master(
            &config(10, 128, 256),
            &master(200, 2048, 512, 10_240),
            &master(300, 3072, 768, 15_360),
            &charged(100, 1024, 256, 5_120),
            &Overcommit::default(),
        );

        assert_eq!(
            (kept.cpu, kept.memory, kept.swap, kept.disk),
            (100, 1024, 256, 5_120)
        );
        assert!(owed.is_zero());
    }

    // A plan below what the splits hold plus the reserve: the master is held at its reserve and the
    // rest is owed by the splits; the books still add up to the plan.
    #[test]
    fn plan_below_the_splits_holds_the_master_at_its_reserve() {
        let splits = charged(300, 3072, 768, 6_144);
        let (kept, owed) = resize_master(
            &config(10, 128, 256),
            &master(200, 2048, 0, 4_096),
            &master(100, 1024, 256, 2_048),
            &splits,
            &Overcommit::default(),
        );

        assert_eq!(
            (kept.cpu, kept.memory, kept.swap, kept.disk),
            (10, 128, 0, 256)
        );
        assert_eq!(
            owed,
            Overcommit {
                cpu: 110,
                memory: 1152,
                swap: 768,
                disk: 2_304,
            }
        );
        assert_eq!(plan_total(&kept, &splits, &owed), charged(200, 2048, 0, 4_096));
    }

    // A plan smaller than the reserve itself: the master never gets more than the whole plan.
    #[test]
    fn plan_below_the_reserve_never_exceeds_the_plan() {
        assert_eq!(plan_share(100, 0, 128, false), (100, 0));
        assert_eq!(plan_share(100, 40, 128, false), (100, 40));
    }

    #[test]
    fn unlimited_plans_stay_unlimited() {
        let (kept, owed) = resize_master(
            &config(10, 128, 256),
            &master(0, 0, -1, 0),
            &master(300, 3072, 768, 15_360),
            &charged(100, 1024, 256, 5_120),
            &Overcommit {
                cpu: 5,
                memory: 5,
                swap: 5,
                disk: 5,
            },
        );

        assert_eq!((kept.cpu, kept.memory, kept.swap, kept.disk), (0, 0, -1, 0));
        assert!(owed.is_zero());
        assert_eq!(
            plan_total(&kept, &charged(100, 1024, 256, 5_120), &owed),
            charged(0, 0, -1, 0)
        );
    }

    // The admin form sends every limit on each save: values equal to the master's own are untouched,
    // and so is what its splits still owe.
    #[test]
    fn unchanged_values_are_left_alone() {
        let current = master(10, 128, 0, 256);
        let owed = Overcommit {
            cpu: 110,
            memory: 1152,
            swap: 768,
            disk: 2_304,
        };
        let (kept, still_owed) = resize_master(
            &config(10, 128, 256),
            &current,
            &current,
            &charged(300, 3072, 768, 6_144),
            &owed,
        );

        assert_eq!(kept, current);
        assert_eq!(still_owed, owed);
    }

    #[test]
    fn master_only_keeps_the_value_and_clears_what_was_owed() {
        let config = ServerSplitterSettingsData {
            external_resize: ExternalResize::MasterOnly,
            ..config(10, 128, 256)
        };
        let (kept, owed) = resize_master(
            &config,
            &master(200, 2048, 512, 4_096),
            &master(10, 128, 0, 256),
            &charged(300, 3072, 768, 6_144),
            &Overcommit {
                cpu: 110,
                memory: 1152,
                swap: 768,
                disk: 2_304,
            },
        );

        assert_eq!(
            (kept.cpu, kept.memory, kept.swap, kept.disk),
            (200, 2048, 512, 4_096)
        );
        assert!(owed.is_zero());
    }

    // Swap not taken from the master isn't part of its plan.
    #[test]
    fn unpooled_swap_is_what_the_master_keeps() {
        let config = ServerSplitterSettingsData {
            swap_mode: SwapMode::Copy,
            ..config(10, 128, 256)
        };
        let (kept, _) = resize_master(
            &config,
            &master(300, 3072, 1024, 15_360),
            &master(300, 3072, 512, 15_360),
            &charged(100, 1024, 0, 5_120),
            &Overcommit::default(),
        );

        assert_eq!(kept.swap, 1024);
    }

    // Freeing resources on an over-committed master pays the overcommit off before the master grows.
    #[test]
    fn credits_pay_down_the_overcommit_first() {
        let owed = Overcommit {
            cpu: 0,
            memory: 1152,
            swap: 0,
            disk: 0,
        };
        let refund = Limits {
            memory: -1024,
            cpu: -100,
            allocations: -1,
            ..Default::default()
        };

        let (debit, owed) = pay_down(&refund, &owed);
        assert_eq!((debit.memory, debit.cpu, debit.allocations), (0, -100, -1));
        assert_eq!(owed.memory, 128);

        let (debit, owed) = pay_down(&refund, &owed);
        assert_eq!(debit.memory, -896);
        assert!(owed.is_zero());

        // taking from the master doesn't touch what is owed
        let take = Limits {
            memory: 512,
            ..Default::default()
        };
        let owing = Overcommit {
            memory: 100,
            ..Default::default()
        };
        assert_eq!(pay_down(&take, &owing), (take, owing));
    }
}
