use crate::{
    health::{self, SplitHealth},
    hooks::{self, SPLIT_DELETE, SplitDelete},
    model::{ServerExtension, ServerSplitterData},
    pool::{self, Charged, LedgerEntry, Limits},
    settings::{ServerSplitterSettingsData, SwapMode},
};
use axum::{
    extract::{Path, Query},
    http::StatusCode,
};
use garde::Validate;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use shared::{
    ApiError, GetState, State,
    models::{
        BaseModel, ByUuid, CreatableModel, DeletableModel, IntoApiObject,
        nest::Nest,
        nest_egg::NestEgg,
        nest_egg_variable::NestEggVariable,
        node_allocation::NodeAllocation,
        server::{
            AdminApiServerLimits, ApiServer, ApiServerBandwidth, ApiServerFeatureLimits,
            CreateServerOptions, GetServer, GetServerActivityLogger, Server, ServerStatus,
        },
        server_subuser::{CreateServerSubuserOptions, ServerSubuser},
        user::{GetPermissionManager, GetUser, PermissionManager},
    },
    response::{ApiResponse, ApiResponseResult, DisplayError},
};
use sqlx::Row;
use std::{borrow::Cow, collections::HashMap};
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};

/// A split's limits: cpu %, memory/swap/disk MB, feature limits. In a `max`, -1 means unlimited.
#[derive(ToSchema, Serialize, Clone, Copy, Debug, Default)]
pub struct Amounts {
    pub cpu: i32,
    pub memory: i64,
    pub swap: i64,
    pub disk: i64,
    pub allocations: i32,
    pub databases: i32,
    pub backups: i32,
    pub schedules: i32,
}

impl From<Limits> for Amounts {
    fn from(limits: Limits) -> Self {
        Self {
            cpu: limits.cpu,
            memory: limits.memory,
            swap: limits.swap,
            disk: limits.disk,
            allocations: limits.allocations,
            databases: limits.databases,
            backups: limits.backups,
            schedules: limits.schedules,
        }
    }
}

#[derive(ToSchema, Serialize)]
pub struct PoolResource {
    /// The master's own limit (0 = unlimited; swap -1 = unlimited).
    pub master: i64,
    /// Part of `master` that is never handed out.
    pub reserved: i64,
    /// Sum of what the splits took from the master.
    pub splits: i64,
    /// `master + splits` (0 when the master is unlimited).
    pub total: i64,
    /// The most a new split may take (-1 = unlimited).
    pub free: i64,
}

#[derive(ToSchema, Serialize)]
pub struct FeaturePool {
    pub master: i64,
    pub used: i64,
    pub splits: i64,
    /// The most a new split getting a new port may take.
    pub free: i64,
}

#[derive(ToSchema, Serialize)]
pub struct SplitterPort {
    /// server allocation uuid
    pub uuid: uuid::Uuid,
    pub ip: String,
    pub ip_alias: Option<String>,
    pub port: i32,
    pub notes: Option<String>,
}

#[derive(ToSchema, Serialize)]
pub struct PoolHealth {
    pub status: &'static str,
    pub issues: Vec<String>,
}

#[derive(ToSchema, Serialize)]
pub struct Minimum {
    pub cpu: i32,
    pub memory: i64,
    pub disk: i64,
}

#[derive(ToSchema, Serialize)]
pub struct SplitCount {
    pub used: i64,
    pub limit: i32,
}

#[derive(ToSchema, Serialize)]
pub struct Pool {
    pub cpu: PoolResource,
    pub memory: PoolResource,
    pub swap: PoolResource,
    pub disk: PoolResource,
    pub allocations: FeaturePool,
    pub databases: FeaturePool,
    pub backups: FeaturePool,
    pub schedules: FeaturePool,
    pub splits: SplitCount,
    pub minimum: Minimum,
    pub allocations_free_with_master_port: i64,
    pub transferable_ports: Vec<SplitterPort>,
    pub swap_mode: SwapMode,
    pub display_reserved: bool,
    pub health: PoolHealth,
}

#[derive(ToSchema, Serialize)]
pub struct SplitEntry {
    pub server: ApiServer,
    pub ledger: Amounts,
    pub min: Amounts,
    pub max: Amounts,
    pub port_from_master: bool,
    pub synced_subusers: i64,
    pub custom_subusers: i64,
    pub health: SplitHealth,
    pub suspended_by_master: bool,
}

#[derive(ToSchema, Serialize)]
pub struct SplitShare {
    pub ledger: Amounts,
    pub health: SplitHealth,
    pub suspended_by_master: bool,
}

#[derive(ToSchema, Serialize)]
pub struct ParentServer {
    pub uuid: uuid::Uuid,
    pub name: compact_str::CompactString,
}

#[derive(ToSchema, Serialize)]
pub struct ClientIndexResponse {
    pub role: &'static str,
    pub parent: Option<ParentServer>,
    pub pool: Option<Pool>,
    pub splits: Vec<SplitEntry>,
    pub share: Option<SplitShare>,
}

#[derive(ToSchema, Serialize)]
pub struct EggVariableItem {
    pub env_variable: compact_str::CompactString,
    pub name: compact_str::CompactString,
    pub description: Option<compact_str::CompactString>,
    pub default_value: Option<String>,
    pub rules: Vec<compact_str::CompactString>,
}

#[derive(ToSchema, Serialize)]
pub struct NestEggItem {
    pub uuid: uuid::Uuid,
    pub name: compact_str::CompactString,
    pub description: Option<compact_str::CompactString>,
    pub variables: Vec<EggVariableItem>,
}

#[derive(ToSchema, Validate, Deserialize)]
pub struct FeatureLimitsInput {
    #[garde(range(min = 1))]
    #[schema(minimum = 1)]
    pub allocations: i32,
    #[serde(default)]
    #[garde(range(min = 0))]
    #[schema(minimum = 0)]
    pub databases: i32,
    #[serde(default)]
    #[garde(range(min = 0))]
    #[schema(minimum = 0)]
    pub backups: i32,
    #[serde(default)]
    #[garde(range(min = 0))]
    #[schema(minimum = 0)]
    pub schedules: i32,
}

/// Where a new split's primary port comes from.
#[derive(ToSchema, Deserialize, Clone, Copy, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PortChoice {
    /// One of the master's non-primary ports (server allocation uuid).
    Master { allocation_uuid: uuid::Uuid },
    /// A free port on the master's node, preferring the master's IP.
    Node,
}

#[derive(ToSchema, Validate, Deserialize)]
pub struct VariableInput {
    #[garde(length(chars, min = 1, max = 255))]
    pub env_variable: compact_str::CompactString,
    #[garde(length(max = 4096))]
    pub value: String,
}

fn default_true() -> bool {
    true
}

#[derive(ToSchema, Validate, Deserialize)]
pub struct CreateSplitPayload {
    #[garde(length(chars, min = 3, max = 255))]
    #[schema(min_length = 3, max_length = 255)]
    pub name: compact_str::CompactString,
    #[garde(length(chars, max = 1024))]
    #[schema(max_length = 1024)]
    pub description: Option<compact_str::CompactString>,
    #[garde(range(min = 0))]
    #[schema(minimum = 0)]
    pub cpu: i32,
    #[garde(range(min = 0))]
    #[schema(minimum = 0)]
    pub memory: i64,
    #[garde(range(min = 0))]
    #[schema(minimum = 0)]
    pub disk: i64,
    #[garde(dive)]
    pub feature_limits: FeatureLimitsInput,
    #[garde(skip)]
    pub egg_uuid: Option<uuid::Uuid>,
    /// Missing: one of the master's spare ports if it has one, else a new one.
    #[serde(default)]
    #[garde(skip)]
    pub port: Option<PortChoice>,
    #[serde(default)]
    #[garde(dive)]
    pub variables: Vec<VariableInput>,
    #[serde(default = "default_true")]
    #[garde(skip)]
    pub sync_subusers: bool,
}

#[derive(ToSchema, Validate, Deserialize)]
pub struct UpdateSplitPayload {
    #[garde(length(chars, min = 3, max = 255))]
    #[schema(min_length = 3, max_length = 255)]
    pub name: Option<compact_str::CompactString>,
    /// An empty description clears it.
    #[garde(length(chars, max = 1024))]
    #[schema(max_length = 1024)]
    pub description: Option<compact_str::CompactString>,
    #[garde(range(min = 0))]
    #[schema(minimum = 0)]
    pub cpu: Option<i32>,
    #[garde(range(min = 0))]
    #[schema(minimum = 0)]
    pub memory: Option<i64>,
    #[garde(range(min = 0))]
    #[schema(minimum = 0)]
    pub disk: Option<i64>,
    #[garde(dive)]
    pub feature_limits: Option<FeatureLimitsInput>,
}

#[derive(ToSchema, Serialize, Default)]
pub struct SyncResult {
    pub added: usize,
    pub updated: usize,
    /// Emails of master subusers left out: the acting user may not grant their permissions.
    pub skipped: Vec<compact_str::CompactString>,
}

pub fn splitter_data(server: &Server) -> ServerSplitterData {
    server
        .parse_model_extension::<ServerExtension>()
        .unwrap_or(ServerSplitterData {
            parent_uuid: None,
            splits: 0,
        })
}

/// Splits are managed only from their master server: permissions granted on a split must not
/// reach the master's resource pool or its sibling splits.
fn child_server_error() -> ApiResponseResult {
    ApiResponse::error("Splits can only be managed from the master server.")
        .with_status(StatusCode::FORBIDDEN)
        .ok()
}

/// An error that `?` turns into a response with `status` and `message`.
fn rejected_with(status: StatusCode, message: impl Into<Cow<'static, str>>) -> anyhow::Error {
    DisplayError::new(message).with_status(status).into()
}

fn rejected(message: impl Into<Cow<'static, str>>) -> anyhow::Error {
    rejected_with(StatusCode::BAD_REQUEST, message)
}

async fn splitter_config(state: &State) -> Result<ServerSplitterSettingsData, anyhow::Error> {
    let settings = state.settings.get().await?;

    Ok(settings
        .find_extension_settings::<ServerSplitterSettingsData>()
        .cloned()
        .unwrap_or_default())
}

/// Rejects split sizes below the configured minimums. A limit of 0 means unlimited, so a split of
/// a limited master gets at least 1 of that resource. Values equal to `current` (the split's size
/// before a resize) are accepted as they are.
fn resource_minimum_error(
    config: &ServerSplitterSettingsData,
    master: &Limits,
    requested: &Limits,
    current: Option<&Limits>,
) -> Option<String> {
    let changed = |field: fn(&Limits) -> i64| current.is_none_or(|c| field(c) != field(requested));

    let min_cpu = config.min_cpu();
    if master.cpu != 0 && requested.cpu < min_cpu && changed(|l| l.cpu as i64) {
        return Some(format!("CPU must be at least {min_cpu}%."));
    }

    let min_memory = config.min_memory();
    if master.memory != 0 && requested.memory < min_memory && changed(|l| l.memory) {
        return Some(format!("Memory must be at least {min_memory}MB."));
    }

    let min_disk = config.min_disk();
    if master.disk != 0 && requested.disk < min_disk && changed(|l| l.disk) {
        return Some(format!("Disk must be at least {min_disk}MB."));
    }

    None
}

/// Egg for a new split: the master's egg needs a rule, and the requested egg (default: the
/// master's own egg) must be one that rule allows.
fn split_egg_uuid(
    config: &ServerSplitterSettingsData,
    master_egg: uuid::Uuid,
    requested: Option<uuid::Uuid>,
) -> Result<uuid::Uuid, &'static str> {
    let rule = config
        .egg_rules
        .iter()
        .find(|rule| rule.eggs.contains(&master_egg))
        .ok_or("Splitting is not enabled for this server's egg.")?;

    let egg = requested.unwrap_or(master_egg);
    if rule.allowed_eggs.contains(&egg) {
        Ok(egg)
    } else {
        Err("Invalid egg ID provided.")
    }
}

/// What a split action needs of the split's state.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Need {
    /// Resize, sync: the split must be available (`unavailable_reason`).
    Available,
    /// Delete: also allowed after a failed install or backup restore, which deleting resolves.
    Deletable,
}

/// A split `master` may manage: linked to it, owned by the same user (the link survives an admin
/// changing either server's owner), and in a state that allows the action.
fn managed_split(master: &Server, split: Option<Server>, need: Need) -> Result<Server, anyhow::Error> {
    let split = split.ok_or_else(|| rejected_with(StatusCode::NOT_FOUND, "subserver not found"))?;

    if splitter_data(&split).parent_uuid != Some(master.uuid) {
        return Err(rejected("subserver does not belong to this parent server"));
    }
    if split.owner.uuid != master.owner.uuid {
        return Err(rejected_with(
            StatusCode::FORBIDDEN,
            "subserver is owned by another user",
        ));
    }

    let failed_state = matches!(
        split.status,
        Some(ServerStatus::InstallFailed | ServerStatus::BackupRestoreFailed)
    );
    if let Some(reason) = split.unavailable_reason()
        && !(need == Need::Deletable && failed_state)
    {
        return Err(rejected_with(StatusCode::CONFLICT, format!("subserver: {reason}")));
    }

    Ok(split)
}

pub async fn get_subservers(
    state: &State,
    parent_uuid: uuid::Uuid,
) -> Result<Vec<Server>, anyhow::Error> {
    let rows = sqlx::query(sqlx::AssertSqlSafe(format!(
        r#"
        SELECT {}
        FROM servers
        LEFT JOIN server_allocations ON server_allocations.uuid = servers.allocation_uuid
        LEFT JOIN node_allocations ON node_allocations.uuid = server_allocations.allocation_uuid
        JOIN users ON users.uuid = servers.owner_uuid
        LEFT JOIN roles ON roles.uuid = users.role_uuid
        JOIN nest_eggs ON nest_eggs.uuid = servers.egg_uuid
        JOIN nests ON nests.uuid = nest_eggs.nest_uuid
        WHERE servers.parent_uuid = $1
        ORDER BY servers.created ASC
        "#,
        Server::columns_sql(None)
    )))
    .bind(parent_uuid)
    .fetch_all(state.database.read())
    .await?;

    let mut subservers = Vec::with_capacity(rows.len());
    for row in rows {
        subservers.push(Server::map(None, &row)?);
    }

    Ok(subservers)
}

fn unlimited_as_minus_one(limits: Limits) -> Limits {
    // `remaining` already uses -1 for unlimited cpu/memory/disk; swap isn't bounded there
    Limits { swap: -1, ..limits }
}

/// A split's ledger entry, or one built from its limits when it has none.
pub async fn entry_or_untracked(
    conn: &mut sqlx::PgConnection,
    split: &Server,
    master: &Server,
) -> Result<(LedgerEntry, bool), sqlx::Error> {
    Ok(match pool::ledger_entry(conn, split.uuid).await? {
        Some(entry) => (entry, true),
        None => (
            hooks::untracked_entry(split, master.uuid, &Limits::of(master)),
            false,
        ),
    })
}

/// Resize bounds for `split` (its ledger entry `entry`) of `master`.
pub async fn resize_bounds(
    conn: &mut sqlx::PgConnection,
    config: &ServerSplitterSettingsData,
    master: &Server,
    master_usage: &pool::FeatureUsage,
    disk_usage_mb: i64,
    split: &Server,
    entry: &LedgerEntry,
) -> Result<(Limits, Limits), sqlx::Error> {
    let master_limits = Limits::of(master);
    let usage = pool::feature_usage(conn, split.uuid).await?;
    let min = pool::resize_minimum(config, &master_limits, &Limits::of(split), &usage);
    let max = unlimited_as_minus_one(pool::remaining(
        config,
        &master_limits,
        master_usage,
        disk_usage_mb,
        Some(&entry.granted),
        false,
    ));

    Ok((min, max))
}

fn port(allocation: &pool::TransferableAllocation) -> SplitterPort {
    SplitterPort {
        uuid: allocation.server_allocation_uuid,
        ip: allocation.ip.clone(),
        ip_alias: allocation.ip_alias.clone(),
        port: allocation.port,
        notes: allocation.notes.clone(),
    }
}

/// The master's pool as shown on the splitter page. `master` must be a master server.
async fn build_pool(
    state: &State,
    config: &ServerSplitterSettingsData,
    master: &Server,
    master_data: &ServerSplitterData,
    conn: &mut sqlx::PgConnection,
    usage: &pool::FeatureUsage,
    disk_usage_mb: i64,
) -> Result<Pool, anyhow::Error> {
    let _ = state;
    let transferable = pool::transferable_allocations(conn, master.uuid).await?;
    let ledger = pool::ledger_by_master(conn, master.uuid).await?;
    let issues = health::issues(conn, master.uuid).await?;
    let used: i64 = sqlx::query_scalar("SELECT count(*) FROM servers WHERE parent_uuid = $1")
        .bind(master.uuid)
        .fetch_one(&mut *conn)
        .await?;

    let limits = Limits::of(master);
    let remaining = pool::remaining(config, &limits, usage, disk_usage_mb, None, false);
    let with_master_port = if transferable.is_empty() {
        remaining
    } else {
        pool::remaining(config, &limits, usage, disk_usage_mb, None, true)
    };

    let charged = ledger.iter().fold(Charged::default(), |sum, entry| Charged {
        cpu: sum.cpu + entry.charged.cpu,
        memory: sum.memory + entry.charged.memory,
        swap: sum.swap + entry.charged.swap,
        disk: sum.disk + entry.charged.disk,
    });
    let granted = ledger
        .iter()
        .fold(Limits::default(), |sum, entry| sum.plus(&entry.granted));

    let resource = |master: i64, reserved: i64, splits: i64, free: i64| PoolResource {
        master,
        reserved: if master > 0 { reserved.max(1).min(master) } else { 0 },
        splits,
        total: if master > 0 { master + splits } else { 0 },
        free,
    };
    let feature = |master: i32, used: i64, splits: i32, free: i32| FeaturePool {
        master: master as i64,
        used,
        splits: splits as i64,
        free: free as i64,
    };

    let swap_free = match config.swap_mode {
        SwapMode::Pooled if limits.swap >= 0 => limits.swap,
        SwapMode::Pooled => -1,
        SwapMode::None => 0,
        SwapMode::Copy => limits.swap,
    };

    Ok(Pool {
        cpu: resource(
            limits.cpu as i64,
            config.reserved_cpu as i64,
            charged.cpu as i64,
            remaining.cpu as i64,
        ),
        memory: resource(limits.memory, config.reserved_memory, charged.memory, remaining.memory),
        swap: PoolResource {
            master: limits.swap,
            reserved: 0,
            splits: charged.swap,
            total: if limits.swap >= 0 { limits.swap + charged.swap } else { -1 },
            free: swap_free,
        },
        disk: resource(limits.disk, config.reserved_disk, charged.disk, remaining.disk),
        allocations: feature(
            limits.allocations,
            usage.allocations,
            granted.allocations,
            remaining.allocations,
        ),
        databases: feature(limits.databases, usage.databases, granted.databases, remaining.databases),
        backups: feature(limits.backups, usage.backups, granted.backups, remaining.backups),
        schedules: feature(limits.schedules, usage.schedules, granted.schedules, remaining.schedules),
        splits: SplitCount {
            used,
            limit: master_data.splits,
        },
        minimum: Minimum {
            cpu: if limits.cpu > 0 { config.min_cpu() } else { 0 },
            memory: if limits.memory > 0 { config.min_memory() } else { 0 },
            disk: if limits.disk > 0 { config.min_disk() } else { 0 },
        },
        allocations_free_with_master_port: with_master_port.allocations as i64,
        transferable_ports: transferable.iter().map(port).collect(),
        swap_mode: config.swap_mode,
        display_reserved: config.display_reserved_limits,
        health: PoolHealth {
            status: if issues.is_empty() { "balanced" } else { "drift" },
            issues: issues
                .iter()
                .map(|issue| format!("{}: {}", issue.split_name, issue.detail))
                .collect(),
        },
    })
}

mod get_index {
    use super::*;

    #[utoipa::path(get, path = "/", responses(
        (status = OK, body = inline(ClientIndexResponse)),
    ))]
    pub async fn route(
        state: GetState,
        permissions: GetPermissionManager,
        user: GetUser,
        server: GetServer,
    ) -> ApiResponseResult {
        permissions.has_server_permission("splitter.read")?;

        let server = server.0;
        let data = splitter_data(&server);
        let config = splitter_config(&state).await?;
        let mut conn = state.database.read().acquire().await?;

        // A split only learns which master it belongs to and what it holds, not the master's pool
        // or its siblings.
        if let Some(parent_uuid) = data.parent_uuid {
            let master = Server::by_uuid(&state.database, parent_uuid).await?;
            let (entry, tracked) = entry_or_untracked(&mut conn, &server, &master).await?;
            let suspended_by_master: bool = sqlx::query_scalar(
                "SELECT split_suspended_by_master FROM servers WHERE uuid = $1",
            )
            .bind(server.uuid)
            .fetch_one(&mut *conn)
            .await?;

            let health = if !tracked {
                SplitHealth::Untracked
            } else if entry.granted != Limits::of(&server) {
                SplitHealth::Drift
            } else {
                SplitHealth::Balanced
            };

            return ApiResponse::new_serialized(ClientIndexResponse {
                role: "split",
                parent: Some(ParentServer {
                    uuid: master.uuid,
                    name: master.name,
                }),
                pool: None,
                splits: Vec::new(),
                share: Some(SplitShare {
                    ledger: entry.granted.into(),
                    health,
                    suspended_by_master: suspended_by_master && server.suspended,
                }),
            })
            .ok();
        }

        let usage = pool::feature_usage(&mut conn, server.uuid).await?;
        let disk_usage_mb = pool::disk_usage_mb(&state, &server, &config).await.unwrap_or(0);
        let pool = build_pool(&state, &config, &server, &data, &mut conn, &usage, disk_usage_mb).await?;

        let extras: HashMap<uuid::Uuid, (bool, i64, i64)> = sqlx::query(
            r#"
            SELECT
                s.uuid,
                s.split_suspended_by_master,
                (
                    SELECT count(*) FROM server_subusers ss
                    JOIN server_subusers ms ON ms.server_uuid = $1 AND ms.user_uuid = ss.user_uuid
                    WHERE ss.server_uuid = s.uuid
                        AND NOT EXISTS (
                            SELECT 1 FROM serversplitter_custom_subusers c
                            WHERE c.split_uuid = s.uuid AND c.user_uuid = ss.user_uuid
                        )
                ) AS synced,
                (SELECT count(*) FROM serversplitter_custom_subusers c WHERE c.split_uuid = s.uuid) AS custom
            FROM servers s
            WHERE s.parent_uuid = $1
            "#,
        )
        .bind(server.uuid)
        .fetch_all(&mut *conn)
        .await?
        .into_iter()
        .map(|row| {
            (
                row.get("uuid"),
                (
                    row.get("split_suspended_by_master"),
                    row.get("synced"),
                    row.get("custom"),
                ),
            )
        })
        .collect();

        let mut splits = Vec::new();
        for split in get_subservers(&state, server.uuid).await? {
            // splits given to another user stay linked, but aren't this master's to show
            if split.owner.uuid != server.owner.uuid {
                continue;
            }

            let (entry, tracked) = entry_or_untracked(&mut conn, &split, &server).await?;
            let (min, max) =
                resize_bounds(&mut conn, &config, &server, &usage, disk_usage_mb, &split, &entry)
                    .await?;
            let health = if !tracked {
                SplitHealth::Untracked
            } else if entry.granted != Limits::of(&split) {
                SplitHealth::Drift
            } else {
                SplitHealth::Balanced
            };
            let (suspended_by_master, synced, custom) =
                extras.get(&split.uuid).copied().unwrap_or_default();
            let suspended = split.suspended;

            splits.push(SplitEntry {
                ledger: entry.granted.into(),
                min: min.into(),
                max: max.into(),
                port_from_master: entry.port_from_master,
                synced_subusers: synced,
                custom_subusers: custom,
                health,
                suspended_by_master: suspended_by_master && suspended,
                server: split.into_api_object(&state, &user).await?,
            });
        }

        ApiResponse::new_serialized(ClientIndexResponse {
            role: "master",
            parent: None,
            pool: Some(pool),
            splits,
            share: None,
        })
        .ok()
    }
}

mod get_nests {
    use super::*;

    #[utoipa::path(get, path = "/nests", responses(
        (status = OK, body = inline(IndexMap<compact_str::CompactString, Vec<NestEggItem>>)),
    ))]
    pub async fn route(
        state: GetState,
        permissions: GetPermissionManager,
        server: GetServer,
    ) -> ApiResponseResult {
        permissions.has_server_permission("splitter.read")?;

        let parent = server.0;
        if splitter_data(&parent).parent_uuid.is_some() {
            return child_server_error();
        }
        let config = splitter_config(&state).await?;

        let allowed_egg_uuids = config
            .egg_rules
            .iter()
            .find(|rule| rule.eggs.contains(&parent.egg.uuid))
            .map(|rule| rule.allowed_eggs.clone())
            .unwrap_or_default();

        let mut result = IndexMap::<compact_str::CompactString, Vec<NestEggItem>>::new();
        if allowed_egg_uuids.is_empty() {
            return ApiResponse::new_serialized(result).ok();
        }

        let nest_rows = sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT {} FROM nests ORDER BY nests.name ASC",
            Nest::columns_sql(None)
        )))
        .fetch_all(state.database.read())
        .await?;

        for row in nest_rows {
            let nest = Nest::map(None, &row)?;
            let mut items = Vec::new();
            for egg in NestEgg::all_by_nest_uuid(&state.database, nest.uuid).await? {
                if !allowed_egg_uuids.contains(&egg.uuid) {
                    continue;
                }

                let mut variables = NestEggVariable::all_by_egg_uuid(&state.database, egg.uuid)
                    .await?
                    .into_iter()
                    .filter(|variable| variable.user_viewable && variable.user_editable)
                    .collect::<Vec<_>>();
                variables.sort_by_key(|variable| variable.order);

                items.push(NestEggItem {
                    uuid: egg.uuid,
                    name: egg.name,
                    description: egg.description,
                    variables: variables
                        .into_iter()
                        .map(|variable| EggVariableItem {
                            env_variable: variable.env_variable,
                            name: variable.name,
                            description: variable.description,
                            default_value: if variable.secret {
                                None
                            } else {
                                variable.default_value
                            },
                            rules: variable.rules,
                        })
                        .collect(),
                });
            }

            if !items.is_empty() {
                result.insert(nest.name, items);
            }
        }

        ApiResponse::new_serialized(result).ok()
    }
}

/// Copies the master's subusers to `split`. With `update_existing`, users already on the split who
/// are synced (not custom) get the master's permissions. Users whose permissions or file restrictions
/// the acting user couldn't grant themselves are skipped, and the panel's subuser limit applies.
async fn copy_subusers(
    state: &State,
    permissions: &PermissionManager,
    actor_ignored_files: Option<&[compact_str::CompactString]>,
    master: &Server,
    split: &Server,
    update_existing: bool,
) -> Result<SyncResult, anyhow::Error> {
    use shared::models::UpdatableModel;
    use shared::models::server_subuser::UpdateServerSubuserOptions;

    let mut result = SyncResult::default();

    let master_subusers =
        ServerSubuser::by_server_uuid_with_pagination(&state.database, master.uuid, 1, 1000, None)
            .await?
            .data;
    let split_subusers =
        ServerSubuser::by_server_uuid_with_pagination(&state.database, split.uuid, 1, 1000, None)
            .await?
            .data;
    let custom: Vec<uuid::Uuid> = sqlx::query_scalar(
        "SELECT user_uuid FROM serversplitter_custom_subusers WHERE split_uuid = $1",
    )
    .bind(split.uuid)
    .fetch_all(state.database.read())
    .await?;
    let max_subusers = state.settings.get().await?.server.max_subuser_count as usize;
    let mut count = split_subusers.len();

    for subuser in master_subusers {
        if custom.contains(&subuser.user.uuid) {
            continue;
        }

        let grantable = permissions
            .missing_server_permission(&subuser.permissions)
            .is_none()
            && actor_ignored_files.is_none_or(|own| subuser.ignored_files.ends_with(own));
        if !grantable {
            result.skipped.push(subuser.user.email.clone());
            continue;
        }

        match split_subusers
            .iter()
            .find(|existing| existing.user.uuid == subuser.user.uuid)
        {
            Some(existing) => {
                if update_existing
                    && (existing.permissions != subuser.permissions
                        || existing.ignored_files != subuser.ignored_files)
                {
                    let mut existing = existing.clone();
                    existing
                        .update(
                            state,
                            UpdateServerSubuserOptions {
                                permissions: Some(subuser.permissions.clone()),
                                ignored_files: Some(subuser.ignored_files.clone()),
                            },
                        )
                        .await?;
                    result.updated += 1;
                }
            }
            None => {
                if count >= max_subusers {
                    result.skipped.push(subuser.user.email.clone());
                    continue;
                }
                ServerSubuser::create(
                    state,
                    CreateServerSubuserOptions {
                        server: split,
                        email: subuser.user.email.clone(),
                        permissions: subuser.permissions.clone(),
                        ignored_files: subuser.ignored_files.clone(),
                    },
                )
                .await?;
                count += 1;
                result.added += 1;
            }
        }
    }

    Ok(result)
}

mod post_split {
    use super::*;

    #[utoipa::path(post, path = "/", responses(
        (status = CREATED, body = inline(ApiServer)),
    ), request_body = inline(CreateSplitPayload))]
    pub async fn route(
        state: GetState,
        permissions: GetPermissionManager,
        user: GetUser,
        server: GetServer,
        activity_logger: GetServerActivityLogger,
        shared::Payload(data): shared::Payload<CreateSplitPayload>,
    ) -> ApiResponseResult {
        permissions.has_server_permission("splitter.create")?;
        if data.sync_subusers {
            permissions.has_server_permission("subusers.create")?;
        }

        if let Err(errors) = shared::utils::validate_data(&data) {
            return ApiResponse::new_serialized(ApiError::new_strings_value(errors))
                .with_status(StatusCode::BAD_REQUEST)
                .ok();
        }

        let master = server.0;
        if splitter_data(&master).parent_uuid.is_some() {
            return child_server_error();
        }

        let sync_subusers = data.sync_subusers;
        // "Start when done" starts the server, which needs the panel's own permission.
        let start = permissions.has_server_permission("control.start").is_ok();

        // Detached, so a dropped request can't stop it between creating the split and charging
        // the master.
        let (split, egg_name) =
            tokio::spawn(create_split(state.0.clone(), master.uuid, data, start)).await??;

        let mut synced = None;
        if sync_subusers {
            match copy_subusers(
                &state,
                &permissions,
                master.subuser_ignored_files.as_deref(),
                &master,
                &split,
                false,
            )
            .await
            {
                Ok(result) => synced = Some(result),
                Err(err) => tracing::warn!(split = %split.uuid, "failed to copy subusers to a new split: {err:?}"),
            }
        }

        activity_logger
            .log(
                "server:splitter.split",
                serde_json::json!({
                    "split_uuid": split.uuid,
                    "name": split.name,
                    "cpu": split.cpu,
                    "memory": split.memory,
                    "disk": split.disk,
                    "egg": egg_name,
                    "subusers_added": synced.as_ref().map(|s| s.added),
                    "subusers_skipped": synced.as_ref().map(|s| s.skipped.clone()),
                }),
            )
            .await;

        ApiResponse::new_serialized(split.into_api_object(&state, &user).await?)
            .with_status(StatusCode::CREATED)
            .ok()
    }

    /// Server variables for a split: every variable of the egg that isn't excluded by its rules, with
    /// the user's value where the user may edit it, else the egg's default. Validated like the panel's
    /// admin server create.
    async fn split_variables(
        state: &State,
        egg_uuid: uuid::Uuid,
        provided: &[VariableInput],
    ) -> Result<HashMap<uuid::Uuid, compact_str::CompactString>, anyhow::Error> {
        let variables = NestEggVariable::all_by_egg_uuid(&state.database, egg_uuid).await?;

        let value_of = |variable: &NestEggVariable| -> String {
            provided
                .iter()
                .find(|input| {
                    variable.user_editable
                        && variable.user_viewable
                        && input.env_variable == variable.env_variable
                })
                .map(|input| input.value.clone())
                .unwrap_or_else(|| variable.default_value.clone().unwrap_or_default())
        };
        let values: Vec<String> = variables.iter().map(value_of).collect();

        let mut validator_variables = HashMap::with_capacity(variables.len());
        for (variable, value) in variables.iter().zip(values.iter()) {
            validator_variables.insert(
                variable.env_variable.as_str(),
                (variable.rules.as_slice(), value.as_str()),
            );
        }

        let validator = rule_validator::Validator::new(validator_variables)
            .map_err(|error| rejected(error.to_string()))?;
        validator.validate().map_err(rejected)?;

        Ok(variables
            .iter()
            .zip(values.iter())
            .filter(|(variable, _)| !validator.is_excluded(&variable.env_variable))
            .map(|(variable, value)| (variable.uuid, value.as_str().into()))
            .collect())
    }

    /// Creates the split while holding the master's row lock, so the checks, the debit and the
    /// new server all see the same pool. The debit commits only once the split exists.
    async fn create_split(
        state: State,
        master_uuid: uuid::Uuid,
        data: CreateSplitPayload,
        start_on_completion: bool,
    ) -> Result<(Server, compact_str::CompactString), anyhow::Error> {
        let config = splitter_config(&state).await?;

        let mut transaction = state.database.write().begin().await?;
        let master = pool::lock_master(&mut transaction, master_uuid)
            .await?
            .ok_or_else(|| rejected_with(StatusCode::NOT_FOUND, "server not found"))?;
        let master_data = splitter_data(&master);
        if master_data.parent_uuid.is_some() {
            return Err(rejected_with(
                StatusCode::FORBIDDEN,
                "Splits can only be managed from the master server.",
            ));
        }

        let splits: i64 = sqlx::query_scalar("SELECT count(*) FROM servers WHERE parent_uuid = $1")
            .bind(master.uuid)
            .fetch_one(&mut *transaction)
            .await?;
        if splits >= master_data.splits as i64 {
            return Err(rejected(
                "Cannot create more splits than the server allows.",
            ));
        }

        let master_limits = Limits::of(&master);
        let mut requested = Limits {
            cpu: data.cpu,
            memory: data.memory,
            swap: 0,
            disk: data.disk,
            allocations: data.feature_limits.allocations,
            databases: data.feature_limits.databases,
            backups: data.feature_limits.backups,
            schedules: data.feature_limits.schedules,
        };
        if let Some(error) = resource_minimum_error(&config, &master_limits, &requested, None) {
            return Err(rejected(error));
        }

        let usage = pool::feature_usage(&mut transaction, master.uuid).await?;
        let transferable = pool::transferable_allocations(&mut transaction, master.uuid).await?;
        let taken = match data.port {
            Some(PortChoice::Master { allocation_uuid }) => Some(
                transferable
                    .iter()
                    .find(|allocation| allocation.server_allocation_uuid == allocation_uuid)
                    .cloned()
                    .ok_or_else(|| rejected("That port isn't one of this server's spare ports."))?,
            ),
            Some(PortChoice::Node) => None,
            None => transferable.first().cloned(),
        };

        let disk_usage_mb = pool::disk_usage_mb(&state, &master, &config)
            .await
            .ok_or_else(|| {
                rejected_with(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "Couldn't read this server's disk usage from its node. Try again in a moment.",
                )
            })?;
        let remaining = pool::remaining(
            &config,
            &master_limits,
            &usage,
            disk_usage_mb,
            None,
            taken.is_some(),
        );
        if let Some(error) = pool::exceeded(&requested, &remaining) {
            return Err(rejected(error));
        }

        let egg_uuid = split_egg_uuid(&config, master.egg.uuid, data.egg_uuid).map_err(rejected)?;
        let egg = NestEgg::by_uuid(&state.database, egg_uuid).await?;
        let variables = split_variables(&state, egg.uuid, &data.variables).await?;

        let allocation_uuid = match &taken {
            Some(allocation) => allocation.allocation_uuid,
            None => free_allocation(&state, &master).await?,
        };

        requested.swap = pool::split_swap(config.swap_mode, master.swap, requested.memory, 0);
        let (debit, charged) = pool::regrant(
            &master_limits,
            config.swap_mode == SwapMode::Pooled,
            &Limits::default(),
            &Charged::default(),
            &requested,
        );

        let startup = egg
            .startup_commands
            .first()
            .map(|(_, v)| v.clone())
            .unwrap_or_default();
        let image = egg
            .docker_images
            .first()
            .map(|(_, v)| v.clone())
            .unwrap_or_default();

        let timezone: Option<chrono_tz::Tz> = master
            .timezone
            .as_ref()
            .and_then(|tz| tz.parse::<chrono_tz::Tz>().ok());

        let create_options = CreateServerOptions {
            node_uuid: master.node.uuid,
            owner_uuid: master.owner.uuid,
            egg_uuid: egg.uuid,
            backup_configuration_uuid: master.backup_configuration.as_ref().map(|b| b.uuid),
            allocation_uuid: Some(allocation_uuid),
            allocation_uuids: Vec::new(),
            start_on_completion,
            skip_installer: false,
            external_id: None,
            name: data.name,
            description: data.description.filter(|d| !d.trim().is_empty()),
            limits: AdminApiServerLimits {
                cpu: requested.cpu,
                memory: requested.memory,
                memory_overhead: master.memory_overhead,
                swap: requested.swap,
                disk: requested.disk,
                io_weight: master.io_weight,
                // a rate cap, not a pooled resource: the split gets the master's cap
                bandwidth: Some(ApiServerBandwidth {
                    upload: master.bandwidth_upload as u64,
                    download: master.bandwidth_download as u64,
                }),
            },
            pinned_cpus: if config.inherit_pinned_cpus {
                master.pinned_cpus.clone()
            } else {
                Vec::new()
            },
            startup,
            image,
            // container labels are admin set per server (1.2.3); a split starts without the master's
            labels: Default::default(),
            timezone,
            trash: master.trash.clone(),
            // admin-granted device and kernel access stays with the server it was granted to
            hugepages_passthrough_enabled: false,
            kvm_passthrough_enabled: false,
            feature_limits: ApiServerFeatureLimits {
                allocations: requested.allocations,
                databases: requested.databases,
                backups: requested.backups,
                schedules: requested.schedules,
                __overlay: schema_extension_core::ExtensionOverlay::new(),
            },
            variables,
        };

        if !pool::debit_master(&mut transaction, master.uuid, &debit).await? {
            return Err(rejected(
                "The master server's resources changed, try again.",
            ));
        }

        let pending = pool::PendingSplit {
            master_uuid: master.uuid,
            transferred_server_allocation: taken
                .as_ref()
                .map(|allocation| allocation.server_allocation_uuid),
            granted: requested,
            charged,
        };
        let split = match pool::PENDING_SPLIT
            .scope(pending, Server::create(&state, create_options))
            .await
        {
            Ok(split) => split,
            Err(err) => {
                // Drops the debit. A split that failed on Wings was committed and deleted again,
                // taking the transferred allocation and its ledger entry with it.
                let _ = transaction.rollback().await;
                if let Some(allocation) = &taken {
                    pool::restore_allocation(&state, master.uuid, allocation).await?;
                }
                return Err(err.into());
            }
        };

        if let Err(err) = transaction.commit().await {
            // The split exists and is linked, but its debit was lost: charge the master again.
            tracing::error!(split = %split.uuid, master = %master.uuid, "failed to commit split debit: {err:?}");

            let mut transaction = state.database.write().begin().await?;
            pool::lock_master(&mut transaction, master.uuid).await?;
            if !pool::debit_master(&mut transaction, master.uuid, &debit).await? {
                tracing::error!(split = %split.uuid, master = %master.uuid, "could not charge the master for a new split");
            }
            transaction.commit().await?;
        }

        pool::refresh_and_sync(&state, &[master.uuid]).await;

        Ok((split, egg.name))
    }

    /// A free allocation on the master's node for the split's primary allocation, preferring the
    /// master's IP.
    async fn free_allocation(state: &State, master: &Server) -> Result<uuid::Uuid, anyhow::Error> {
        let node = master.node.fetch_cached(&state.database).await?;
        let exclude = NodeAllocation::used_by_node_any_ip(state, &node).await?;

        if let Some(master_allocation) = &master.allocation {
            let same_ip = sqlx::query_scalar::<_, uuid::Uuid>(
                r#"
                SELECT node_allocations.uuid
                FROM node_allocations
                LEFT JOIN server_allocations ON server_allocations.allocation_uuid = node_allocations.uuid
                WHERE node_allocations.node_uuid = $1
                  AND node_allocations.ip = $2
                  AND server_allocations.uuid IS NULL
                  AND NOT (node_allocations.uuid = ANY($3))
                ORDER BY RANDOM()
                LIMIT 1
                "#,
            )
            .bind(master.node.uuid)
            .bind(master_allocation.allocation.ip)
            .bind(&exclude)
            .fetch_optional(state.database.read())
            .await?;

            if let Some(uuid) = same_ip {
                return Ok(uuid);
            }
        }

        NodeAllocation::get_random(&state.database, master.node.uuid, 1, 65535, 1, &exclude, None)
            .await?
            .first()
            .copied()
            .ok_or_else(|| rejected("No free allocations are available on this node."))
    }
}

mod patch_split {
    use super::*;

    #[utoipa::path(patch, path = "/{subserver}", responses(
        (status = OK, body = inline(ApiServer)),
    ), request_body = inline(UpdateSplitPayload))]
    pub async fn route(
        state: GetState,
        permissions: GetPermissionManager,
        user: GetUser,
        server: GetServer,
        activity_logger: GetServerActivityLogger,
        Path((_server, subserver_uuid)): Path<(String, uuid::Uuid)>,
        shared::Payload(data): shared::Payload<UpdateSplitPayload>,
    ) -> ApiResponseResult {
        permissions.has_server_permission("splitter.update")?;

        if let Err(errors) = shared::utils::validate_data(&data) {
            return ApiResponse::new_serialized(ApiError::new_strings_value(errors))
                .with_status(StatusCode::BAD_REQUEST)
                .ok();
        }

        let master = server.0;
        if splitter_data(&master).parent_uuid.is_some() {
            return child_server_error();
        }

        // Detached, so a dropped request can't skip syncing the committed limits to Wings.
        let split = tokio::spawn(resize_split(
            state.0.clone(),
            master.uuid,
            subserver_uuid,
            data,
            false,
        ))
        .await??;

        activity_logger
            .log(
                "server:splitter.update",
                serde_json::json!({
                    "split_uuid": split.uuid,
                    "name": split.name,
                    "cpu": split.cpu,
                    "memory": split.memory,
                    "disk": split.disk,
                }),
            )
            .await;

        ApiResponse::new_serialized(split.into_api_object(&state, &user).await?).ok()
    }
}

/// Resizes (and renames) a split under its master's lock. With `admin`, the split may be in any state
/// and owned by anyone, and may go below the configured minimum split size (never to 0 = unlimited
/// where the master is limited); the pool bounds apply to everyone.
pub async fn resize_split(
    state: State,
    master_uuid: uuid::Uuid,
    split_uuid: uuid::Uuid,
    data: UpdateSplitPayload,
    admin: bool,
) -> Result<Server, anyhow::Error> {
    let config = splitter_config(&state).await?;

    let mut transaction = state.database.write().begin().await?;
    let master = pool::lock_master(&mut transaction, master_uuid)
        .await?
        .ok_or_else(|| rejected_with(StatusCode::NOT_FOUND, "server not found"))?;
    let split = Server::by_uuid_optional_with_transaction(&mut transaction, split_uuid).await?;
    let split = if admin {
        split
            .filter(|split| splitter_data(split).parent_uuid == Some(master.uuid))
            .ok_or_else(|| rejected_with(StatusCode::NOT_FOUND, "split not found"))?
    } else {
        managed_split(&master, split, Need::Available)?
    };
    let (entry, _) = entry_or_untracked(&mut transaction, &split, &master).await?;

    let master_limits = Limits::of(&master);
    let current = Limits::of(&split);
    let mut requested = Limits {
        cpu: data.cpu.unwrap_or(current.cpu),
        memory: data.memory.unwrap_or(current.memory),
        swap: current.swap,
        disk: data.disk.unwrap_or(current.disk),
        allocations: data
            .feature_limits
            .as_ref()
            .map_or(current.allocations, |f| f.allocations),
        databases: data
            .feature_limits
            .as_ref()
            .map_or(current.databases, |f| f.databases),
        backups: data
            .feature_limits
            .as_ref()
            .map_or(current.backups, |f| f.backups),
        schedules: data
            .feature_limits
            .as_ref()
            .map_or(current.schedules, |f| f.schedules),
    };

    let minimums = if admin {
        ServerSplitterSettingsData {
            minimum_cpu: Some(1),
            minimum_memory: Some(1),
            minimum_disk: Some(1),
            ..config.clone()
        }
    } else {
        config.clone()
    };
    if let Some(error) = resource_minimum_error(&minimums, &master_limits, &requested, Some(&current)) {
        return Err(rejected(error));
    }

    let split_usage = pool::feature_usage(&mut transaction, split.uuid).await?;
    if let Some(error) = pool::below_usage(&requested, &current, &split_usage) {
        return Err(rejected(error));
    }

    let master_usage = pool::feature_usage(&mut transaction, master.uuid).await?;
    let disk_usage_mb = pool::disk_usage_mb(&state, &master, &config)
        .await
        .ok_or_else(|| {
            rejected_with(
                StatusCode::SERVICE_UNAVAILABLE,
                "Couldn't read the master's disk usage from its node. Try again in a moment.",
            )
        })?;
    let remaining = pool::remaining(
        &config,
        &master_limits,
        &master_usage,
        disk_usage_mb,
        Some(&entry.granted),
        false,
    );
    if let Some(error) = pool::exceeded(&requested, &remaining) {
        return Err(rejected(error));
    }

    // swap follows memory: a rename or a cpu/disk change leaves it as it is
    if requested.memory != current.memory {
        requested.swap = pool::split_swap(
            config.swap_mode,
            master.swap,
            requested.memory,
            entry.charged.swap,
        );
    }
    let (debit, charged) = pool::regrant(
        &master_limits,
        config.swap_mode == SwapMode::Pooled,
        &entry.granted,
        &entry.charged,
        &requested,
    );

    let name = data.name.unwrap_or_else(|| split.name.clone());
    let description = match data.description {
        Some(description) if description.trim().is_empty() => None,
        Some(description) => Some(description),
        None => split.description.clone(),
    };

    sqlx::query(
        r#"
        UPDATE servers
        SET
            name = $1,
            description = $2,
            cpu = $3,
            memory = $4,
            disk = $5,
            swap = $6,
            allocation_limit = $7,
            database_limit = $8,
            backup_limit = $9,
            schedule_limit = $10
        WHERE uuid = $11
        "#,
    )
    .bind(name)
    .bind(description)
    .bind(requested.cpu)
    .bind(requested.memory)
    .bind(requested.disk)
    .bind(requested.swap)
    .bind(requested.allocations)
    .bind(requested.databases)
    .bind(requested.backups)
    .bind(requested.schedules)
    .bind(split.uuid)
    .execute(&mut *transaction)
    .await?;

    if !pool::debit_master(&mut transaction, master.uuid, &debit).await? {
        return Err(rejected(
            "The master server's resources changed, try again.",
        ));
    }
    pool::save_ledger(
        &mut transaction,
        &LedgerEntry {
            granted: requested,
            charged,
            ..entry
        },
    )
    .await?;

    transaction.commit().await?;

    pool::refresh_and_sync(&state, &[split.uuid, master.uuid]).await;

    Ok(Server::by_uuid(&state.database, split.uuid).await?)
}

#[derive(Deserialize, ToSchema)]
pub struct DeleteQuery {
    /// Give the split's ports back to the master (up to its allocation limit). Default true.
    #[serde(default)]
    pub return_port: Option<bool>,
}

mod delete_split {
    use super::*;

    #[utoipa::path(delete, path = "/{subserver}", responses(
        (status = NO_CONTENT, description = "Success"),
    ), params(("return_port" = Option<bool>, Query, description = "Give the split's ports back to the master (default true)")))]
    pub async fn route(
        state: GetState,
        permissions: GetPermissionManager,
        server: GetServer,
        activity_logger: GetServerActivityLogger,
        Path((_server, subserver_uuid)): Path<(String, uuid::Uuid)>,
        Query(query): Query<DeleteQuery>,
    ) -> ApiResponseResult {
        permissions.has_server_permission("splitter.delete")?;

        let master = server.0;
        if splitter_data(&master).parent_uuid.is_some() {
            return child_server_error();
        }

        let split = managed_split(
            &master,
            Server::by_uuid_optional(&state.database, subserver_uuid).await?,
            Need::Deletable,
        )?;

        let split_name = split.name.clone();
        let split_uuid = split.uuid;
        let return_ports = query.return_port.unwrap_or(true);

        // The delete handler returns the split's resources (and ports) to the master and syncs it.
        SPLIT_DELETE
            .scope(
                SplitDelete {
                    return_ports,
                    master_going: false,
                },
                split.delete(
                    &state,
                    shared::models::server::DeleteServerOptions { force: false },
                ),
            )
            .await?;
        Server::invalidate_cached(&state.database, master.uuid).await;

        activity_logger
            .log(
                "server:splitter.delete",
                serde_json::json!({
                    "split_uuid": split_uuid,
                    "name": split_name,
                    "return_ports": return_ports,
                }),
            )
            .await;

        ApiResponse::new_serialized(())
            .with_status(StatusCode::NO_CONTENT)
            .ok()
    }
}

mod sync_subusers {
    use super::*;

    #[utoipa::path(post, path = "/{subserver}/subusers-sync", responses(
        (status = OK, body = inline(SyncResult)),
    ))]
    pub async fn route(
        state: GetState,
        permissions: GetPermissionManager,
        server: GetServer,
        activity_logger: GetServerActivityLogger,
        Path((_server, subserver_uuid)): Path<(String, uuid::Uuid)>,
    ) -> ApiResponseResult {
        permissions.has_server_permission("splitter.update")?;
        permissions.has_server_permission("subusers.create")?;
        permissions.has_server_permission("subusers.update")?;

        let master = server.0;
        if splitter_data(&master).parent_uuid.is_some() {
            return child_server_error();
        }

        let split = managed_split(
            &master,
            Server::by_uuid_optional(&state.database, subserver_uuid).await?,
            Need::Available,
        )?;

        let result = copy_subusers(
            &state,
            &permissions,
            master.subuser_ignored_files.as_deref(),
            &master,
            &split,
            true,
        )
        .await?;

        activity_logger
            .log(
                "server:splitter.subusers-sync",
                serde_json::json!({
                    "split_uuid": split.uuid,
                    "added": result.added,
                    "updated": result.updated,
                    "skipped": result.skipped,
                }),
            )
            .await;

        ApiResponse::new_serialized(result).ok()
    }
}

#[derive(ToSchema, Serialize)]
pub struct SplitSubuserUser {
    pub uuid: uuid::Uuid,
    pub username: String,
    pub email: String,
}

#[derive(ToSchema, Serialize)]
pub struct SplitSubuser {
    pub user: SplitSubuserUser,
    pub on_master: bool,
    pub on_split: bool,
    pub custom: bool,
}

#[derive(ToSchema, Serialize)]
pub struct SplitSubusersResponse {
    pub subusers: Vec<SplitSubuser>,
}

#[derive(ToSchema, Deserialize)]
pub struct CustomPayload {
    pub custom: bool,
}

mod get_split_subusers {
    use super::*;

    #[utoipa::path(get, path = "/{subserver}/subusers", responses(
        (status = OK, body = inline(SplitSubusersResponse)),
    ))]
    pub async fn route(
        state: GetState,
        permissions: GetPermissionManager,
        server: GetServer,
        Path((_server, subserver_uuid)): Path<(String, uuid::Uuid)>,
    ) -> ApiResponseResult {
        permissions.has_server_permission("splitter.read")?;
        permissions.has_server_permission("subusers.read")?;

        let master = server.0;
        if splitter_data(&master).parent_uuid.is_some() {
            return child_server_error();
        }
        let split = managed_split(
            &master,
            Server::by_uuid_optional(&state.database, subserver_uuid).await?,
            Need::Deletable,
        )?;

        let subusers = sqlx::query(
            r#"
            SELECT
                u.uuid, u.username::text AS username, u.email::text AS email,
                (ms.user_uuid IS NOT NULL) AS on_master,
                (ss.user_uuid IS NOT NULL) AS on_split,
                (c.user_uuid IS NOT NULL) AS custom
            FROM users u
            LEFT JOIN server_subusers ms ON ms.server_uuid = $1 AND ms.user_uuid = u.uuid
            LEFT JOIN server_subusers ss ON ss.server_uuid = $2 AND ss.user_uuid = u.uuid
            LEFT JOIN serversplitter_custom_subusers c ON c.split_uuid = $2 AND c.user_uuid = u.uuid
            WHERE ms.user_uuid IS NOT NULL OR ss.user_uuid IS NOT NULL
            ORDER BY u.username ASC
            "#,
        )
        .bind(master.uuid)
        .bind(split.uuid)
        .fetch_all(state.database.read())
        .await?
        .into_iter()
        .map(|row| SplitSubuser {
            user: SplitSubuserUser {
                uuid: row.get("uuid"),
                username: row.get("username"),
                email: row.get("email"),
            },
            on_master: row.get("on_master"),
            on_split: row.get("on_split"),
            custom: row.get("custom"),
        })
        .collect();

        ApiResponse::new_serialized(SplitSubusersResponse { subusers }).ok()
    }
}

mod put_subuser_custom {
    use super::*;

    #[utoipa::path(put, path = "/{subserver}/subusers/{user}/custom", responses(
        (status = NO_CONTENT, description = "Success"),
    ), request_body = inline(CustomPayload))]
    pub async fn route(
        state: GetState,
        permissions: GetPermissionManager,
        server: GetServer,
        Path((_server, subserver_uuid, user_uuid)): Path<(String, uuid::Uuid, uuid::Uuid)>,
        shared::Payload(data): shared::Payload<CustomPayload>,
    ) -> ApiResponseResult {
        permissions.has_server_permission("splitter.update")?;
        permissions.has_server_permission("subusers.update")?;

        let master = server.0;
        if splitter_data(&master).parent_uuid.is_some() {
            return child_server_error();
        }
        let split = managed_split(
            &master,
            Server::by_uuid_optional(&state.database, subserver_uuid).await?,
            Need::Deletable,
        )?;

        let known: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM server_subusers WHERE server_uuid IN ($1, $2) AND user_uuid = $3)",
        )
        .bind(master.uuid)
        .bind(split.uuid)
        .bind(user_uuid)
        .fetch_one(state.database.read())
        .await?;
        if !known {
            return ApiResponse::error("subuser not found")
                .with_status(StatusCode::NOT_FOUND)
                .ok();
        }

        if data.custom {
            sqlx::query(
                "INSERT INTO serversplitter_custom_subusers (split_uuid, user_uuid) VALUES ($1, $2) ON CONFLICT DO NOTHING",
            )
        } else {
            sqlx::query(
                "DELETE FROM serversplitter_custom_subusers WHERE split_uuid = $1 AND user_uuid = $2",
            )
        }
        .bind(split.uuid)
        .bind(user_uuid)
        .execute(state.database.write())
        .await?;

        ApiResponse::new_serialized(())
            .with_status(StatusCode::NO_CONTENT)
            .ok()
    }
}

pub fn router(state: &State) -> OpenApiRouter<State> {
    OpenApiRouter::new()
        .routes(routes!(get_index::route))
        .routes(routes!(get_nests::route))
        .routes(routes!(post_split::route))
        .routes(routes!(patch_split::route))
        .routes(routes!(delete_split::route))
        .routes(routes!(sync_subusers::route))
        .routes(routes!(get_split_subusers::route))
        .routes(routes!(put_subuser_custom::route))
        .with_state(state.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::EggRule;

    fn create_payload(feature_limits: serde_json::Value) -> CreateSplitPayload {
        serde_json::from_value(serde_json::json!({
            "name": "split",
            "cpu": 100,
            "memory": 1024,
            "disk": 2048,
            "feature_limits": feature_limits,
        }))
        .unwrap()
    }

    fn update_payload(value: serde_json::Value) -> UpdateSplitPayload {
        serde_json::from_value(value).unwrap()
    }

    fn reserved(cpu: i32, memory: i64, disk: i64) -> ServerSplitterSettingsData {
        ServerSplitterSettingsData {
            reserved_cpu: cpu,
            reserved_memory: memory,
            reserved_disk: disk,
            ..Default::default()
        }
    }

    fn resources(cpu: i32, memory: i64, disk: i64) -> Limits {
        Limits {
            cpu,
            memory,
            disk,
            ..Default::default()
        }
    }

    // Negative limits on a split are subtracted from the master, raising its limits.
    #[test]
    fn payloads_reject_negative_feature_limits() {
        for field in ["databases", "backups", "schedules"] {
            let limits = serde_json::json!({ "allocations": 1, field: -100 });
            assert!(
                create_payload(limits.clone()).validate().is_err(),
                "create {field}"
            );
            assert!(
                update_payload(serde_json::json!({ "feature_limits": limits }))
                    .validate()
                    .is_err(),
                "update {field}"
            );
        }
    }

    #[test]
    fn payloads_require_an_allocation() {
        for allocations in [0, -5] {
            let limits = serde_json::json!({ "allocations": allocations });
            assert!(create_payload(limits.clone()).validate().is_err());
            assert!(
                update_payload(serde_json::json!({ "feature_limits": limits }))
                    .validate()
                    .is_err()
            );
        }
    }

    #[test]
    fn payloads_reject_negative_resources() {
        for field in ["cpu", "memory", "disk"] {
            assert!(
                update_payload(serde_json::json!({ field: -1 }))
                    .validate()
                    .is_err(),
                "{field}"
            );
        }
    }

    #[test]
    fn payloads_accept_valid_and_partial_updates() {
        assert!(
            create_payload(serde_json::json!({ "allocations": 1, "databases": 0 }))
                .validate()
                .is_ok()
        );
        assert!(update_payload(serde_json::json!({})).validate().is_ok());
        // an empty description clears it
        assert!(
            update_payload(serde_json::json!({ "description": "" }))
                .validate()
                .is_ok()
        );
    }

    // The panel's admin server create requires 3 characters.
    #[test]
    fn payloads_reject_short_or_oversized_names() {
        for name in ["", "ab"] {
            assert!(
                update_payload(serde_json::json!({ "name": name }))
                    .validate()
                    .is_err()
            );
        }
        assert!(
            update_payload(serde_json::json!({ "name": "x".repeat(256) }))
                .validate()
                .is_err()
        );
        assert!(
            update_payload(serde_json::json!({ "description": "x".repeat(1025) }))
                .validate()
                .is_err()
        );
    }

    #[test]
    fn port_choice_parses_both_kinds() {
        let payload: CreateSplitPayload = serde_json::from_value(serde_json::json!({
            "name": "split",
            "cpu": 100,
            "memory": 1024,
            "disk": 2048,
            "feature_limits": { "allocations": 1 },
            "port": { "type": "master", "allocation_uuid": "18a6ac64-df1b-4b2c-a327-8dd4e79a3ffb" },
        }))
        .unwrap();
        assert!(matches!(payload.port, Some(PortChoice::Master { .. })));

        let payload: CreateSplitPayload = serde_json::from_value(serde_json::json!({
            "name": "split",
            "cpu": 100,
            "memory": 1024,
            "disk": 2048,
            "feature_limits": { "allocations": 1 },
            "port": { "type": "node" },
        }))
        .unwrap();
        assert!(matches!(payload.port, Some(PortChoice::Node)));
        assert!(create_payload(serde_json::json!({ "allocations": 1 })).port.is_none());
    }

    // 0 means unlimited: a limited master must not hand out an unlimited split, even with no
    // reservation configured.
    #[test]
    fn limited_master_cannot_create_unlimited_split() {
        let config = reserved(0, 0, 0);
        let master = resources(400, 8192, 10_000);
        for split in [
            resources(0, 512, 512),
            resources(50, 0, 512),
            resources(50, 512, 0),
        ] {
            assert!(resource_minimum_error(&config, &master, &split, None).is_some());
        }
        assert!(resource_minimum_error(&config, &master, &resources(1, 1, 1), None).is_none());
    }

    #[test]
    fn unlimited_master_may_create_unlimited_splits() {
        let config = reserved(10, 128, 256);
        assert!(
            resource_minimum_error(&config, &resources(0, 0, 0), &resources(0, 0, 0), None)
                .is_none()
        );
    }

    #[test]
    fn reserved_amount_is_the_default_minimum_split_size() {
        let config = reserved(10, 128, 256);
        let master = resources(400, 8192, 10_000);
        for split in [
            resources(9, 128, 256),
            resources(10, 127, 256),
            resources(10, 128, 255),
        ] {
            assert!(resource_minimum_error(&config, &master, &split, None).is_some());
        }
        assert!(resource_minimum_error(&config, &master, &resources(10, 128, 256), None).is_none());
    }

    #[test]
    fn minimum_split_size_can_differ_from_the_reserve() {
        let config = ServerSplitterSettingsData {
            minimum_cpu: Some(25),
            minimum_memory: Some(512),
            minimum_disk: Some(1024),
            ..reserved(10, 128, 256)
        };
        let master = resources(400, 8192, 10_000);
        assert!(resource_minimum_error(&config, &master, &resources(10, 512, 1024), None).is_some());
        assert!(resource_minimum_error(&config, &master, &resources(25, 512, 1024), None).is_none());
    }

    // A split below a later-raised minimum can still be renamed or resized in other resources.
    #[test]
    fn unchanged_sizes_below_the_minimum_are_kept() {
        let config = reserved(10, 128, 256);
        let master = resources(400, 8192, 10_000);
        let current = resources(5, 64, 100);

        assert!(resource_minimum_error(&config, &master, &current, Some(&current)).is_none());
        assert!(
            resource_minimum_error(&config, &master, &resources(6, 64, 100), Some(&current))
                .is_some()
        );
    }

    #[test]
    fn split_egg_requires_a_rule_for_the_master_egg() {
        let (master, other, allowed) = (
            uuid::Uuid::new_v4(),
            uuid::Uuid::new_v4(),
            uuid::Uuid::new_v4(),
        );
        let config = ServerSplitterSettingsData {
            egg_rules: vec![EggRule {
                id: uuid::Uuid::new_v4(),
                eggs: vec![master],
                allowed_eggs: vec![allowed],
            }],
            ..Default::default()
        };

        // No rule for the master's egg: splitting is off, whatever egg is requested.
        assert!(split_egg_uuid(&config, other, Some(allowed)).is_err());
        assert!(split_egg_uuid(&config, other, None).is_err());
        // A rule exists: only its allowed eggs, and the default is not exempt.
        assert!(split_egg_uuid(&config, master, Some(other)).is_err());
        assert!(split_egg_uuid(&config, master, None).is_err());
        assert_eq!(split_egg_uuid(&config, master, Some(allowed)), Ok(allowed));
    }

    #[test]
    fn split_egg_defaults_to_the_master_egg_when_allowed() {
        let master = uuid::Uuid::new_v4();
        let config = ServerSplitterSettingsData {
            egg_rules: vec![EggRule {
                id: uuid::Uuid::new_v4(),
                eggs: vec![master],
                allowed_eggs: vec![master],
            }],
            ..Default::default()
        };
        assert_eq!(split_egg_uuid(&config, master, None), Ok(master));
    }
}
