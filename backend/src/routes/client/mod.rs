use crate::{
    model::{ServerExtension, ServerSplitterData},
    pool::{self, Limits},
    settings::ServerSplitterSettingsData,
};
use axum::{extract::Path, http::StatusCode};
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
            AdminApiServerLimits, ApiServer, ApiServerFeatureLimits, CreateServerOptions,
            DeleteServerOptions, GetServer, GetServerActivityLogger, Server,
        },
        server_subuser::{CreateServerSubuserOptions, ServerSubuser},
        user::{GetPermissionManager, GetUser},
    },
    response::{ApiResponse, ApiResponseResult, DisplayError},
};
use std::{borrow::Cow, collections::HashMap};
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};

#[derive(ToSchema, Serialize, Deserialize, Clone, Debug)]
pub struct SplitterFeatureLimits {
    pub allocations: i32,
    pub databases: i32,
    pub backups: i32,
    pub schedules: i32,
    pub splits: i32,
}

#[derive(ToSchema, Serialize, Deserialize, Clone, Debug)]
pub struct SplitterResourceLimits {
    pub cpu: i32,
    pub memory: i64,
    pub disk: i64,
    pub feature_limits: SplitterFeatureLimits,
}

#[derive(ToSchema, Serialize, Deserialize, Clone, Debug)]
pub struct ReservedLimits {
    pub cpu: i32,
    pub memory: i64,
    pub disk: i64,
}

#[derive(ToSchema, Serialize, Deserialize, Clone, Debug)]
pub struct ResourcesData {
    pub total: SplitterResourceLimits,
    /// The most a new split may take. cpu/memory/disk are -1 when the master is unlimited.
    pub remaining: SplitterResourceLimits,
    /// `remaining` for display: adds the reserve back unless reserved limits are displayed.
    pub remaining_display: SplitterResourceLimits,
    pub reserved: ReservedLimits,
    /// Whether a new split takes over one of the master's extra allocations. It then doesn't
    /// count against the master's free allocation slots, so a resize has one slot less.
    pub transferable_allocation: bool,
}

#[derive(ToSchema, Serialize)]
pub struct ParentServer {
    pub uuid: uuid::Uuid,
    pub name: compact_str::CompactString,
}

#[derive(ToSchema, Serialize)]
pub struct ClientIndexResponse {
    /// `None` when the server is itself a split: splits are managed from their master only.
    pub resources: Option<ResourcesData>,
    pub parent: Option<ParentServer>,
    pub servers: Vec<ApiServer>,
}

#[derive(ToSchema, Serialize)]
pub struct NestEggItem {
    pub uuid: uuid::Uuid,
    pub name: compact_str::CompactString,
    pub description: Option<compact_str::CompactString>,
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

fn default_true() -> bool {
    true
}

#[derive(ToSchema, Validate, Deserialize)]
pub struct CreateSplitPayload {
    #[garde(length(chars, min = 1, max = 255))]
    #[schema(min_length = 1, max_length = 255)]
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
    #[serde(default = "default_true")]
    #[garde(skip)]
    pub sync_subusers: bool,
}

#[derive(ToSchema, Validate, Deserialize)]
pub struct UpdateSplitPayload {
    #[garde(length(chars, min = 1, max = 255))]
    #[schema(min_length = 1, max_length = 255)]
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

    let min_cpu = config.reserved_cpu.max(1);
    if master.cpu != 0 && requested.cpu < min_cpu && changed(|l| l.cpu as i64) {
        return Some(format!("CPU must be at least {min_cpu}%."));
    }

    let min_memory = config.reserved_memory.max(1);
    if master.memory != 0 && requested.memory < min_memory && changed(|l| l.memory) {
        return Some(format!("Memory must be at least {min_memory}MB."));
    }

    let min_disk = config.reserved_disk.max(1);
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

/// A split `master` may manage: linked to it, owned by the same user (the link survives an admin
/// changing either server's owner), and not suspended or mid-transfer.
fn managed_split(master: &Server, split: Option<Server>) -> Result<Server, anyhow::Error> {
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
    if split.suspended {
        return Err(rejected_with(
            StatusCode::CONFLICT,
            "subserver is suspended",
        ));
    }
    if split.destination_node.is_some() {
        return Err(rejected_with(
            StatusCode::CONFLICT,
            "subserver is being transferred",
        ));
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

fn api_limits(limits: &Limits, splits: i32) -> SplitterResourceLimits {
    SplitterResourceLimits {
        cpu: limits.cpu,
        memory: limits.memory,
        disk: limits.disk,
        feature_limits: SplitterFeatureLimits {
            allocations: limits.allocations,
            databases: limits.databases,
            backups: limits.backups,
            schedules: limits.schedules,
            splits,
        },
    }
}

/// The master's pool as shown on the splitter page. `master` must be a master server.
pub async fn calculate_resources(
    state: &State,
    master: &Server,
    master_data: &ServerSplitterData,
) -> Result<ResourcesData, anyhow::Error> {
    let config = splitter_config(state).await?;

    let mut connection = state.database.read().acquire().await?;
    let usage = pool::feature_usage(&mut connection, master.uuid).await?;
    let transferable_allocation = pool::transferable_allocation(&mut connection, master.uuid)
        .await?
        .is_some();
    drop(connection);

    let disk_usage_mb = pool::disk_usage_mb(state, master, &config).await;
    let limits = Limits::of(master);
    let remaining = pool::remaining(
        &config,
        &limits,
        &usage,
        disk_usage_mb,
        None,
        transferable_allocation,
    );

    let mut total = api_limits(&limits, master_data.splits);
    total.feature_limits.allocations = limits.allocations.max(usage.allocations as i32);

    let remaining = api_limits(&remaining, 0);
    let mut remaining_display = remaining.clone();
    if !config.display_reserved_limits {
        if remaining_display.cpu != -1 {
            remaining_display.cpu += config.reserved_cpu;
        }
        if remaining_display.memory != -1 {
            remaining_display.memory += config.reserved_memory;
        }
        if remaining_display.disk != -1 {
            remaining_display.disk += config.reserved_disk;
        }
    }

    Ok(ResourcesData {
        total,
        remaining,
        remaining_display,
        reserved: ReservedLimits {
            cpu: config.reserved_cpu,
            memory: config.reserved_memory,
            disk: config.reserved_disk,
        },
        transferable_allocation,
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

        // A split only learns which master it belongs to, not the master's pool or siblings.
        if let Some(parent_uuid) = data.parent_uuid {
            let master = Server::by_uuid(&state.database, parent_uuid).await?;
            return ApiResponse::new_serialized(ClientIndexResponse {
                resources: None,
                parent: Some(ParentServer {
                    uuid: master.uuid,
                    name: master.name,
                }),
                servers: Vec::new(),
            })
            .ok();
        }

        let subservers = get_subservers(&state, server.uuid).await?;
        let resources = calculate_resources(&state, &server, &data).await?;

        let mut servers_api = Vec::with_capacity(subservers.len());
        for sub in subservers {
            // splits given to another user stay linked, but aren't this master's to show
            if sub.owner.uuid == server.owner.uuid {
                servers_api.push(sub.into_api_object(&state, &user).await?);
            }
        }

        ApiResponse::new_serialized(ClientIndexResponse {
            resources: Some(resources),
            parent: None,
            servers: servers_api,
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

        if allowed_egg_uuids.is_empty() {
            return ApiResponse::new_serialized(IndexMap::<
                compact_str::CompactString,
                Vec<NestEggItem>,
            >::new())
            .ok();
        }

        let nest_rows = sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT {} FROM nests ORDER BY nests.name ASC",
            Nest::columns_sql(None)
        )))
        .fetch_all(state.database.read())
        .await?;
        let mut result = IndexMap::<compact_str::CompactString, Vec<NestEggItem>>::new();

        for row in nest_rows {
            let nest = Nest::map(None, &row)?;
            let eggs = NestEgg::all_by_nest_uuid(&state.database, nest.uuid).await?;
            let filtered: Vec<NestEggItem> = eggs
                .into_iter()
                .filter(|e| allowed_egg_uuids.contains(&e.uuid))
                .map(|e| NestEggItem {
                    uuid: e.uuid,
                    name: e.name,
                    description: e.description,
                })
                .collect();

            if !filtered.is_empty() {
                result.insert(nest.name, filtered);
            }
        }

        ApiResponse::new_serialized(result).ok()
    }
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

        // Detached, so a dropped request can't stop it between creating the split and charging
        // the master.
        let (split, egg_name) =
            tokio::spawn(create_split(state.0.clone(), master.uuid, data)).await??;

        if sync_subusers
            && let Ok(master_subusers) = ServerSubuser::by_server_uuid_with_pagination(
                &state.database,
                master.uuid,
                1,
                1000,
                None,
            )
            .await
        {
            for subuser in master_subusers.data {
                let _ = ServerSubuser::create(
                    &state,
                    CreateServerSubuserOptions {
                        server: &split,
                        email: subuser.user.email.clone(),
                        permissions: subuser.permissions.clone(),
                        ignored_files: subuser.ignored_files.clone(),
                    },
                )
                .await;
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
                }),
            )
            .await;

        ApiResponse::new_serialized(split.into_api_object(&state, &user).await?)
            .with_status(StatusCode::CREATED)
            .ok()
    }

    /// Creates the split while holding the master's row lock, so the checks, the debit and the
    /// new server all see the same pool. The debit commits only once the split exists.
    async fn create_split(
        state: State,
        master_uuid: uuid::Uuid,
        data: CreateSplitPayload,
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
        let requested = Limits {
            cpu: data.cpu,
            memory: data.memory,
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
        let transferable = pool::transferable_allocation(&mut transaction, master.uuid).await?;
        let disk_usage_mb = pool::disk_usage_mb(&state, &master, &config).await;
        let remaining = pool::remaining(
            &config,
            &master_limits,
            &usage,
            disk_usage_mb,
            None,
            transferable.is_some(),
        );
        if let Some(error) = pool::exceeded(&requested, &remaining) {
            return Err(rejected(error));
        }

        let egg_uuid = split_egg_uuid(&config, master.egg.uuid, data.egg_uuid).map_err(rejected)?;
        let egg = NestEgg::by_uuid(&state.database, egg_uuid).await?;

        let allocation_uuid = match &transferable {
            Some(allocation) => allocation.allocation_uuid,
            None => free_allocation(&state, &master).await?,
        };

        let mut variables = HashMap::new();
        for variable in NestEggVariable::all_by_egg_uuid(&state.database, egg.uuid).await? {
            variables.insert(
                variable.uuid,
                variable.default_value.unwrap_or_default().into(),
            );
        }

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
            start_on_completion: true,
            skip_installer: false,
            external_id: None,
            name: data.name,
            description: data.description.filter(|d| !d.trim().is_empty()),
            limits: AdminApiServerLimits {
                cpu: data.cpu,
                memory: data.memory,
                memory_overhead: 0,
                swap: if master.swap > 0 || master.swap == -1 {
                    data.memory / 4
                } else {
                    0
                },
                disk: data.disk,
                io_weight: master.io_weight,
            },
            pinned_cpus: Vec::new(),
            startup,
            image,
            timezone,
            hugepages_passthrough_enabled: master.hugepages_passthrough_enabled,
            kvm_passthrough_enabled: master.kvm_passthrough_enabled,
            feature_limits: ApiServerFeatureLimits {
                allocations: data.feature_limits.allocations,
                databases: data.feature_limits.databases,
                backups: data.feature_limits.backups,
                schedules: data.feature_limits.schedules,
                __overlay: schema_extension_core::ExtensionOverlay::new(),
            },
            variables,
        };

        if !pool::debit_master(&mut transaction, master.uuid, &requested).await? {
            return Err(rejected(
                "The master server's resources changed, try again.",
            ));
        }

        let pending = pool::PendingSplit {
            master_uuid: master.uuid,
            transferred_server_allocation: transferable
                .as_ref()
                .map(|allocation| allocation.server_allocation_uuid),
        };
        let split = match pool::PENDING_SPLIT
            .scope(pending, Server::create(&state, create_options))
            .await
        {
            Ok(split) => split,
            Err(err) => {
                // Drops the debit. A split that failed on Wings was committed and deleted again,
                // taking the transferred allocation with it.
                let _ = transaction.rollback().await;
                if let Some(allocation) = &transferable {
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
            if !pool::debit_master(&mut transaction, master.uuid, &requested).await? {
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

        NodeAllocation::get_random(&state.database, master.node.uuid, 1, 65535, 1, &exclude)
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
        let split = tokio::spawn(update_split(
            state.0.clone(),
            master.uuid,
            subserver_uuid,
            data,
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

    async fn update_split(
        state: State,
        master_uuid: uuid::Uuid,
        split_uuid: uuid::Uuid,
        data: UpdateSplitPayload,
    ) -> Result<Server, anyhow::Error> {
        let config = splitter_config(&state).await?;

        let mut transaction = state.database.write().begin().await?;
        let master = pool::lock_master(&mut transaction, master_uuid)
            .await?
            .ok_or_else(|| rejected_with(StatusCode::NOT_FOUND, "server not found"))?;
        let split = managed_split(
            &master,
            Server::by_uuid_optional_with_transaction(&mut transaction, split_uuid).await?,
        )?;

        let master_limits = Limits::of(&master);
        let current = Limits::of(&split);
        let requested = Limits {
            cpu: data.cpu.unwrap_or(current.cpu),
            memory: data.memory.unwrap_or(current.memory),
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

        if let Some(error) =
            resource_minimum_error(&config, &master_limits, &requested, Some(&current))
        {
            return Err(rejected(error));
        }

        let split_usage = pool::feature_usage(&mut transaction, split.uuid).await?;
        if let Some(error) = pool::below_usage(&requested, &current, &split_usage) {
            return Err(rejected(error));
        }

        let master_usage = pool::feature_usage(&mut transaction, master.uuid).await?;
        let disk_usage_mb = pool::disk_usage_mb(&state, &master, &config).await;
        let remaining = pool::remaining(
            &config,
            &master_limits,
            &master_usage,
            disk_usage_mb,
            Some(&current),
            false,
        );
        if let Some(error) = pool::exceeded(&requested, &remaining) {
            return Err(rejected(error));
        }

        let name = data.name.unwrap_or_else(|| split.name.clone());
        let description = match data.description {
            Some(description) if description.trim().is_empty() => None,
            Some(description) => Some(description),
            None => split.description.clone(),
        };
        let swap = if master.swap > 0 || master.swap == -1 {
            requested.memory / 4
        } else {
            0
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
        .bind(swap)
        .bind(requested.allocations)
        .bind(requested.databases)
        .bind(requested.backups)
        .bind(requested.schedules)
        .bind(split.uuid)
        .execute(&mut *transaction)
        .await?;

        if !pool::debit_master(&mut transaction, master.uuid, &requested.minus(&current)).await? {
            return Err(rejected(
                "The master server's resources changed, try again.",
            ));
        }

        transaction.commit().await?;

        pool::refresh_and_sync(&state, &[split.uuid, master.uuid]).await;

        Ok(Server::by_uuid(&state.database, split.uuid).await?)
    }
}

mod delete_split {
    use super::*;

    #[utoipa::path(delete, path = "/{subserver}", responses(
        (status = NO_CONTENT, description = "Success"),
    ))]
    pub async fn route(
        state: GetState,
        permissions: GetPermissionManager,
        server: GetServer,
        activity_logger: GetServerActivityLogger,
        Path((_server, subserver_uuid)): Path<(String, uuid::Uuid)>,
    ) -> ApiResponseResult {
        permissions.has_server_permission("splitter.delete")?;

        let master = server.0;
        if splitter_data(&master).parent_uuid.is_some() {
            return child_server_error();
        }

        let split = managed_split(
            &master,
            Server::by_uuid_optional(&state.database, subserver_uuid).await?,
        )?;

        let split_name = split.name.clone();
        let split_uuid = split.uuid;

        // The delete handler returns the split's resources to the master and syncs it.
        split
            .delete(&state, DeleteServerOptions { force: false })
            .await?;
        Server::invalidate_cached(&state.database, master.uuid).await;

        activity_logger
            .log(
                "server:splitter.delete",
                serde_json::json!({
                    "split_uuid": split_uuid,
                    "name": split_name,
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
        (status = NO_CONTENT, description = "Success"),
    ))]
    pub async fn route(
        state: GetState,
        permissions: GetPermissionManager,
        server: GetServer,
        Path((_server, subserver_uuid)): Path<(String, uuid::Uuid)>,
    ) -> ApiResponseResult {
        permissions.has_server_permission("splitter.update")?;

        let master = server.0;
        if splitter_data(&master).parent_uuid.is_some() {
            return child_server_error();
        }

        let split = managed_split(
            &master,
            Server::by_uuid_optional(&state.database, subserver_uuid).await?,
        )?;

        let master_subusers = ServerSubuser::by_server_uuid_with_pagination(
            &state.database,
            master.uuid,
            1,
            1000,
            None,
        )
        .await?
        .data;

        let split_subusers = ServerSubuser::by_server_uuid_with_pagination(
            &state.database,
            split.uuid,
            1,
            1000,
            None,
        )
        .await?
        .data;

        for subuser in master_subusers {
            if split_subusers
                .iter()
                .any(|s| s.user.uuid == subuser.user.uuid)
            {
                continue;
            }

            let _ = ServerSubuser::create(
                &state,
                CreateServerSubuserOptions {
                    server: &split,
                    email: subuser.user.email.clone(),
                    permissions: subuser.permissions.clone(),
                    ignored_files: subuser.ignored_files.clone(),
                },
            )
            .await;
        }

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

    #[test]
    fn payloads_reject_empty_or_oversized_names() {
        assert!(
            update_payload(serde_json::json!({ "name": "" }))
                .validate()
                .is_err()
        );
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
    fn reserved_amount_is_the_minimum_split_size() {
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
