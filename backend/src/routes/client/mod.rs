use crate::{
    model::{ServerExtension, ServerSplitterData},
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
    response::{ApiResponse, ApiResponseResult},
};
use sqlx::Row;
use std::collections::HashMap;
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
    pub remaining: SplitterResourceLimits,
    pub remaining_display: SplitterResourceLimits,
    pub reserved: ReservedLimits,
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
    #[garde(skip)]
    pub name: compact_str::CompactString,
    #[garde(skip)]
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
    #[garde(skip)]
    pub name: Option<compact_str::CompactString>,
    #[garde(skip)]
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

/// Rejects split sizes below the configured minimums. A limit of 0 means unlimited, so a split of
/// a limited master gets at least 1 of that resource.
fn resource_minimum_error(
    config: &ServerSplitterSettingsData,
    master_cpu: i32,
    master_disk: i64,
    cpu: i32,
    memory: i64,
    disk: i64,
) -> Option<String> {
    let min_cpu = config.reserved_cpu.max(1);
    if master_cpu != 0 && cpu < min_cpu {
        return Some(format!("CPU must be at least {min_cpu}%."));
    }

    let min_memory = config.reserved_memory.max(1);
    if memory < min_memory {
        return Some(format!("Memory must be at least {min_memory}MB."));
    }

    let min_disk = config.reserved_disk.max(1);
    if master_disk != 0 && disk < min_disk {
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

/// `parent` must be a master server.
pub async fn calculate_resources(
    state: &State,
    parent: &Server,
    parent_data: &ServerSplitterData,
    subserver: Option<&Server>,
) -> Result<ResourcesData, anyhow::Error> {
    let settings = state.settings.get().await?;
    let config: ServerSplitterSettingsData = settings
        .find_extension_settings::<ServerSplitterSettingsData>()
        .cloned()
        .unwrap_or_default();

    let disk_utilization_mb: i64 = if config.include_disk_usage {
        if let Ok(node) = parent.node.fetch_cached(&state.database).await {
            if let Ok(resources_map) = node.fetch_server_resources(&state.database).await {
                resources_map
                    .get(&parent.uuid)
                    .map(|r| (r.disk_bytes / 1024 / 1024) as i64)
                    .unwrap_or(0)
            } else {
                0
            }
        } else {
            0
        }
    } else {
        0
    };

    let sub_cpu = subserver.map(|s| s.cpu).unwrap_or(0);
    let sub_memory = subserver.map(|s| s.memory).unwrap_or(0);
    let sub_disk = subserver.map(|s| s.disk).unwrap_or(0);

    let base_cpu = parent.cpu - config.reserved_cpu + sub_cpu;
    let base_disk = parent.disk - config.reserved_disk - disk_utilization_mb + sub_disk;

    let (used_allocations, used_databases, used_backups, used_schedules) = tokio::try_join!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM server_allocations WHERE server_uuid = $1"
        )
        .bind(parent.uuid)
        .fetch_one(state.database.read()),
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM server_databases WHERE server_uuid = $1"
        )
        .bind(parent.uuid)
        .fetch_one(state.database.read()),
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM server_backups WHERE server_uuid = $1")
            .bind(parent.uuid)
            .fetch_one(state.database.read()),
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM server_schedules WHERE server_uuid = $1"
        )
        .bind(parent.uuid)
        .fetch_one(state.database.read()),
    )?;

    let sub_alloc = subserver.map(|s| s.allocation_limit).unwrap_or(0);
    let sub_db = subserver.map(|s| s.database_limit).unwrap_or(0);
    let sub_backup = subserver.map(|s| s.backup_limit).unwrap_or(0);
    let sub_sched = subserver.map(|s| s.schedule_limit).unwrap_or(0);

    // A server can allocate:
    // 1) Any extra allocations it already holds beyond its primary allocation: (used_allocations - 1)
    // 2) Any unused allocation quota from its limit: (parent.allocation_limit - used_allocations)
    // Combined: max(parent.allocation_limit, used_allocations) - 1 + sub_alloc
    let rem_alloc = (((parent.allocation_limit as i64).max(used_allocations) - 1).max(0)
        + sub_alloc as i64) as i32;
    let rem_db = (parent.database_limit as i64 - used_databases + sub_db as i64).max(0) as i32;
    let rem_backup = (parent.backup_limit as i64 - used_backups + sub_backup as i64).max(0) as i32;
    let rem_sched =
        (parent.schedule_limit as i64 - used_schedules + sub_sched as i64).max(0) as i32;

    let total = SplitterResourceLimits {
        cpu: parent.cpu,
        memory: parent.memory,
        disk: parent.disk,
        feature_limits: SplitterFeatureLimits {
            allocations: parent.allocation_limit.max(used_allocations as i32),
            databases: parent.database_limit,
            backups: parent.backup_limit,
            schedules: parent.schedule_limit,
            splits: parent_data.splits,
        },
    };

    let remaining = SplitterResourceLimits {
        cpu: if parent.cpu > 0 { base_cpu.max(0) } else { -1 },
        memory: (parent.memory - config.reserved_memory + sub_memory).max(0),
        disk: if parent.disk > 0 {
            base_disk.max(0)
        } else {
            -1
        },
        feature_limits: SplitterFeatureLimits {
            allocations: rem_alloc,
            databases: rem_db,
            backups: rem_backup,
            schedules: rem_sched,
            splits: 0,
        },
    };

    let mut remaining_display = remaining.clone();
    if !config.display_reserved_limits {
        if remaining_display.cpu != -1 {
            remaining_display.cpu += config.reserved_cpu;
        }
        remaining_display.memory += config.reserved_memory;
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
        let resources = calculate_resources(&state, &server, &data, None).await?;

        let mut servers_api = Vec::with_capacity(subservers.len());
        for sub in subservers {
            servers_api.push(sub.into_api_object(&state, &user).await?);
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
        let settings = state.settings.get().await?;
        let config: ServerSplitterSettingsData = settings
            .find_extension_settings::<ServerSplitterSettingsData>()
            .cloned()
            .unwrap_or_default();

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

        let parent = server.0;
        let parent_data = splitter_data(&parent);
        if parent_data.parent_uuid.is_some() {
            return child_server_error();
        }

        // 1. Check split count limit
        let current_splits_count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM servers WHERE parent_uuid = $1")
                .bind(parent.uuid)
                .fetch_one(state.database.read())
                .await?;

        if current_splits_count >= parent_data.splits as i64 {
            return ApiResponse::error("Cannot create more splits than the server allows.")
                .with_status(StatusCode::BAD_REQUEST)
                .ok();
        }

        // 2. Check resources against config minimums
        let settings = state.settings.get().await?;
        let config: ServerSplitterSettingsData = settings
            .find_extension_settings::<ServerSplitterSettingsData>()
            .cloned()
            .unwrap_or_default();

        if let Some(error) = resource_minimum_error(
            &config,
            parent.cpu,
            parent.disk,
            data.cpu,
            data.memory,
            data.disk,
        ) {
            return ApiResponse::error(error)
                .with_status(StatusCode::BAD_REQUEST)
                .ok();
        }

        // 3. Check against remaining resources
        let remaining = calculate_resources(&state, &parent, &parent_data, None).await?;

        if parent.cpu != 0 && data.cpu > remaining.remaining.cpu {
            return ApiResponse::error("CPU limit exceeded.")
                .with_status(StatusCode::BAD_REQUEST)
                .ok();
        }

        if data.memory > remaining.remaining.memory {
            return ApiResponse::error("Memory limit exceeded.")
                .with_status(StatusCode::BAD_REQUEST)
                .ok();
        }

        if parent.disk != 0 && data.disk > remaining.remaining.disk {
            return ApiResponse::error("Disk limit exceeded.")
                .with_status(StatusCode::BAD_REQUEST)
                .ok();
        }

        if data.feature_limits.allocations > remaining.remaining.feature_limits.allocations {
            return ApiResponse::error("Allocation limit exceeded.")
                .with_status(StatusCode::BAD_REQUEST)
                .ok();
        }

        if data.feature_limits.databases > remaining.remaining.feature_limits.databases {
            return ApiResponse::error("Database limit exceeded.")
                .with_status(StatusCode::BAD_REQUEST)
                .ok();
        }

        if data.feature_limits.backups > remaining.remaining.feature_limits.backups {
            return ApiResponse::error("Backup limit exceeded.")
                .with_status(StatusCode::BAD_REQUEST)
                .ok();
        }

        if data.feature_limits.schedules > remaining.remaining.feature_limits.schedules {
            return ApiResponse::error("Schedule limit exceeded.")
                .with_status(StatusCode::BAD_REQUEST)
                .ok();
        }

        // 4. Resolve egg
        let egg_uuid = match split_egg_uuid(&config, parent.egg.uuid, data.egg_uuid) {
            Ok(egg_uuid) => egg_uuid,
            Err(error) => {
                return ApiResponse::error(error)
                    .with_status(StatusCode::BAD_REQUEST)
                    .ok();
            }
        };
        let egg = NestEgg::by_uuid(&state.database, egg_uuid).await?;

        // 5. Select allocation on parent's node
        let node_model = parent.node.fetch_cached(&state.database).await?;
        let exclude = NodeAllocation::used_by_node_any_ip(&state, &node_model).await?;

        // Check if parent has extra non-primary allocations in server_allocations that can be transferred
        let extra_alloc_row = sqlx::query(
            r#"
            SELECT sa.uuid as sa_uuid, sa.allocation_uuid
            FROM server_allocations sa
            JOIN servers s ON s.uuid = sa.server_uuid
            WHERE sa.server_uuid = $1
              AND (s.allocation_uuid IS NULL OR sa.uuid != s.allocation_uuid)
            LIMIT 1
            "#,
        )
        .bind(parent.uuid)
        .fetch_optional(state.database.read())
        .await?;

        let (allocation_uuid, transferred_sa_info) = if let Some(row) = extra_alloc_row {
            let sa_uuid: uuid::Uuid = row.get("sa_uuid");
            let alloc_uuid: uuid::Uuid = row.get("allocation_uuid");

            sqlx::query("DELETE FROM server_allocations WHERE uuid = $1")
                .bind(sa_uuid)
                .execute(state.database.write())
                .await?;

            (alloc_uuid, Some((sa_uuid, alloc_uuid)))
        } else if let Some(parent_alloc) = &parent.allocation {
            let row_opt = sqlx::query_scalar::<_, uuid::Uuid>(
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
            .bind(parent.node.uuid)
            .bind(parent_alloc.allocation.ip)
            .bind(&exclude)
            .fetch_optional(state.database.read())
            .await?;

            let u = match row_opt {
                Some(u) => u,
                None => {
                    let randoms = NodeAllocation::get_random(
                        &state.database,
                        parent.node.uuid,
                        1,
                        65535,
                        1,
                        &exclude,
                    )
                    .await?;
                    *randoms
                        .first()
                        .ok_or_else(|| anyhow::anyhow!("No available allocations"))?
                }
            };
            (u, None)
        } else {
            let randoms = NodeAllocation::get_random(
                &state.database,
                parent.node.uuid,
                1,
                65535,
                1,
                &exclude,
            )
            .await?;
            let u = *randoms
                .first()
                .ok_or_else(|| anyhow::anyhow!("No available allocations"))?;
            (u, None)
        };

        // 6. Build default egg variables
        let egg_vars = NestEggVariable::all_by_egg_uuid(&state.database, egg.uuid).await?;
        let mut server_variables = HashMap::new();
        for var in egg_vars {
            server_variables.insert(var.uuid, var.default_value.unwrap_or_default().into());
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

        let timezone: Option<chrono_tz::Tz> = parent
            .timezone
            .as_ref()
            .and_then(|tz| tz.parse::<chrono_tz::Tz>().ok());

        let create_options = CreateServerOptions {
            node_uuid: parent.node.uuid,
            owner_uuid: parent.owner.uuid,
            egg_uuid: egg.uuid,
            backup_configuration_uuid: parent.backup_configuration.as_ref().map(|b| b.uuid),
            allocation_uuid: Some(allocation_uuid),
            allocation_uuids: Vec::new(),
            start_on_completion: true,
            skip_installer: false,
            external_id: None,
            name: data.name.clone(),
            description: data.description.clone(),
            limits: AdminApiServerLimits {
                cpu: data.cpu,
                memory: data.memory,
                memory_overhead: 0,
                swap: if parent.swap > 0 || parent.swap == -1 {
                    data.memory / 4
                } else {
                    0
                },
                disk: data.disk,
                io_weight: parent.io_weight,
            },
            pinned_cpus: Vec::new(),
            startup,
            image,
            timezone,
            hugepages_passthrough_enabled: parent.hugepages_passthrough_enabled,
            kvm_passthrough_enabled: parent.kvm_passthrough_enabled,
            feature_limits: ApiServerFeatureLimits {
                allocations: data.feature_limits.allocations,
                databases: data.feature_limits.databases,
                backups: data.feature_limits.backups,
                schedules: data.feature_limits.schedules,
                __overlay: schema_extension_core::ExtensionOverlay::new(),
            },
            variables: server_variables,
        };

        let split = match Server::create(&state, create_options).await {
            Ok(s) => s,
            Err(err) => {
                if let Some((sa_uuid, alloc_uuid)) = transferred_sa_info {
                    let _ = sqlx::query(
                        "INSERT INTO server_allocations (uuid, server_uuid, allocation_uuid) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
                    )
                    .bind(sa_uuid)
                    .bind(parent.uuid)
                    .bind(alloc_uuid)
                    .execute(state.database.write())
                    .await;
                }
                return ApiResponse::from(err).ok();
            }
        };

        // 7. Mark parent_uuid on split server and decrement parent resources
        let mut transaction = state.database.write().begin().await?;

        // a split never gets splits of its own: they are managed from the master only
        sqlx::query("UPDATE servers SET parent_uuid = $1, splits = 0 WHERE uuid = $2")
            .bind(parent.uuid)
            .bind(split.uuid)
            .execute(&mut *transaction)
            .await?;

        sqlx::query(
            r#"
            UPDATE servers
            SET
                cpu = CASE WHEN cpu > 0 THEN GREATEST(0, cpu - $1) ELSE cpu END,
                memory = GREATEST(0, memory - $2),
                disk = CASE WHEN disk > 0 THEN GREATEST(0, disk - $3) ELSE disk END,
                allocation_limit = GREATEST(0, allocation_limit - $4),
                database_limit = GREATEST(0, database_limit - $5),
                backup_limit = GREATEST(0, backup_limit - $6),
                schedule_limit = GREATEST(0, schedule_limit - $7)
            WHERE uuid = $8
            "#,
        )
        .bind(data.cpu)
        .bind(data.memory)
        .bind(data.disk)
        .bind(data.feature_limits.allocations)
        .bind(data.feature_limits.databases)
        .bind(data.feature_limits.backups)
        .bind(data.feature_limits.schedules)
        .bind(parent.uuid)
        .execute(&mut *transaction)
        .await?;

        transaction.commit().await?;

        // 8. Synchronize parent to node
        let database_arc = std::sync::Arc::new(state.database.clone());
        parent.clone().batch_sync(&database_arc).await;

        // 9. Sync subusers if requested
        if data.sync_subusers
            && let Ok(parent_subusers) = ServerSubuser::by_server_uuid_with_pagination(
                &state.database,
                parent.uuid,
                1,
                1000,
                None,
            )
            .await
        {
            for subuser in parent_subusers.data {
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

        // 10. Audit log
        activity_logger
            .log(
                "server:splitter.split",
                serde_json::json!({
                    "split_uuid": split.uuid,
                    "name": split.name,
                    "cpu": split.cpu,
                    "memory": split.memory,
                    "disk": split.disk,
                    "egg": egg.name,
                }),
            )
            .await;

        ApiResponse::new_serialized(split.into_api_object(&state, &user).await?)
            .with_status(StatusCode::CREATED)
            .ok()
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

        let parent = server.0;
        let parent_data = splitter_data(&parent);
        if parent_data.parent_uuid.is_some() {
            return child_server_error();
        }

        let split = match Server::by_uuid(&state.database, subserver_uuid).await {
            Ok(s) => s,
            Err(_) => {
                return ApiResponse::error("subserver not found")
                    .with_status(StatusCode::NOT_FOUND)
                    .ok();
            }
        };

        if splitter_data(&split).parent_uuid != Some(parent.uuid) {
            return ApiResponse::error("subserver does not belong to this parent server")
                .with_status(StatusCode::BAD_REQUEST)
                .ok();
        }

        let new_cpu = data.cpu.unwrap_or(split.cpu);
        let new_memory = data.memory.unwrap_or(split.memory);
        let new_disk = data.disk.unwrap_or(split.disk);

        let new_allocations = data
            .feature_limits
            .as_ref()
            .map(|f| f.allocations)
            .unwrap_or(split.allocation_limit);
        let new_databases = data
            .feature_limits
            .as_ref()
            .map(|f| f.databases)
            .unwrap_or(split.database_limit);
        let new_backups = data
            .feature_limits
            .as_ref()
            .map(|f| f.backups)
            .unwrap_or(split.backup_limit);
        let new_schedules = data
            .feature_limits
            .as_ref()
            .map(|f| f.schedules)
            .unwrap_or(split.schedule_limit);

        let settings = state.settings.get().await?;
        let config: ServerSplitterSettingsData = settings
            .find_extension_settings::<ServerSplitterSettingsData>()
            .cloned()
            .unwrap_or_default();

        if let Some(error) = resource_minimum_error(
            &config,
            parent.cpu,
            parent.disk,
            new_cpu,
            new_memory,
            new_disk,
        ) {
            return ApiResponse::error(error)
                .with_status(StatusCode::BAD_REQUEST)
                .ok();
        }

        let remaining = calculate_resources(&state, &parent, &parent_data, Some(&split)).await?;

        if parent.cpu != 0 && new_cpu > remaining.remaining.cpu {
            return ApiResponse::error("CPU limit exceeded.")
                .with_status(StatusCode::BAD_REQUEST)
                .ok();
        }

        if new_memory > remaining.remaining.memory {
            return ApiResponse::error("Memory limit exceeded.")
                .with_status(StatusCode::BAD_REQUEST)
                .ok();
        }

        if parent.disk != 0 && new_disk > remaining.remaining.disk {
            return ApiResponse::error("Disk limit exceeded.")
                .with_status(StatusCode::BAD_REQUEST)
                .ok();
        }

        if new_allocations > remaining.remaining.feature_limits.allocations {
            return ApiResponse::error("Allocation limit exceeded.")
                .with_status(StatusCode::BAD_REQUEST)
                .ok();
        }

        if new_databases > remaining.remaining.feature_limits.databases {
            return ApiResponse::error("Database limit exceeded.")
                .with_status(StatusCode::BAD_REQUEST)
                .ok();
        }

        if new_backups > remaining.remaining.feature_limits.backups {
            return ApiResponse::error("Backup limit exceeded.")
                .with_status(StatusCode::BAD_REQUEST)
                .ok();
        }

        if new_schedules > remaining.remaining.feature_limits.schedules {
            return ApiResponse::error("Schedule limit exceeded.")
                .with_status(StatusCode::BAD_REQUEST)
                .ok();
        }

        let delta_cpu = new_cpu - split.cpu;
        let delta_memory = new_memory - split.memory;
        let delta_disk = new_disk - split.disk;
        let delta_alloc = new_allocations - split.allocation_limit;
        let delta_db = new_databases - split.database_limit;
        let delta_backup = new_backups - split.backup_limit;
        let delta_sched = new_schedules - split.schedule_limit;

        let new_name = data.name.unwrap_or_else(|| split.name.clone());
        let new_description = match data.description {
            Some(d) => Some(d),
            None => split.description.clone(),
        };

        let new_swap = if parent.swap > 0 || parent.swap == -1 {
            new_memory / 4
        } else {
            0
        };

        let mut transaction = state.database.write().begin().await?;

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
        .bind(new_name)
        .bind(new_description)
        .bind(new_cpu)
        .bind(new_memory)
        .bind(new_disk)
        .bind(new_swap)
        .bind(new_allocations)
        .bind(new_databases)
        .bind(new_backups)
        .bind(new_schedules)
        .bind(split.uuid)
        .execute(&mut *transaction)
        .await?;

        sqlx::query(
            r#"
            UPDATE servers
            SET
                cpu = CASE WHEN cpu > 0 THEN GREATEST(0, cpu - $1) ELSE cpu END,
                memory = GREATEST(0, memory - $2),
                disk = CASE WHEN disk > 0 THEN GREATEST(0, disk - $3) ELSE disk END,
                allocation_limit = GREATEST(0, allocation_limit - $4),
                database_limit = GREATEST(0, database_limit - $5),
                backup_limit = GREATEST(0, backup_limit - $6),
                schedule_limit = GREATEST(0, schedule_limit - $7)
            WHERE uuid = $8
            "#,
        )
        .bind(delta_cpu)
        .bind(delta_memory)
        .bind(delta_disk)
        .bind(delta_alloc)
        .bind(delta_db)
        .bind(delta_backup)
        .bind(delta_sched)
        .bind(parent.uuid)
        .execute(&mut *transaction)
        .await?;

        transaction.commit().await?;

        let updated_split = Server::by_uuid(&state.database, split.uuid).await?;
        let database_arc = std::sync::Arc::new(state.database.clone());
        updated_split.clone().batch_sync(&database_arc).await;
        parent.batch_sync(&database_arc).await;

        activity_logger
            .log(
                "server:splitter.update",
                serde_json::json!({
                    "split_uuid": updated_split.uuid,
                    "name": updated_split.name,
                    "cpu": updated_split.cpu,
                    "memory": updated_split.memory,
                    "disk": updated_split.disk,
                }),
            )
            .await;

        ApiResponse::new_serialized(updated_split.into_api_object(&state, &user).await?).ok()
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

        let parent = server.0;
        if splitter_data(&parent).parent_uuid.is_some() {
            return child_server_error();
        }

        let split = match Server::by_uuid(&state.database, subserver_uuid).await {
            Ok(s) => s,
            Err(_) => {
                return ApiResponse::error("subserver not found")
                    .with_status(StatusCode::NOT_FOUND)
                    .ok();
            }
        };

        if splitter_data(&split).parent_uuid != Some(parent.uuid) {
            return ApiResponse::error("subserver does not belong to this parent server")
                .with_status(StatusCode::BAD_REQUEST)
                .ok();
        }

        let split_name = split.name.clone();
        let split_uuid = split.uuid;

        split
            .delete(&state, DeleteServerOptions { force: false })
            .await?;

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

        let parent = server.0;
        if splitter_data(&parent).parent_uuid.is_some() {
            return child_server_error();
        }

        let split = match Server::by_uuid(&state.database, subserver_uuid).await {
            Ok(s) => s,
            Err(_) => {
                return ApiResponse::error("subserver not found")
                    .with_status(StatusCode::NOT_FOUND)
                    .ok();
            }
        };

        if splitter_data(&split).parent_uuid != Some(parent.uuid) {
            return ApiResponse::error("subserver does not belong to this parent server")
                .with_status(StatusCode::BAD_REQUEST)
                .ok();
        }

        let parent_subusers = ServerSubuser::by_server_uuid_with_pagination(
            &state.database,
            parent.uuid,
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

        for subuser in parent_subusers {
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
    }

    // 0 means unlimited: a limited master must not hand out an unlimited split, even with no
    // reservation configured.
    #[test]
    fn limited_master_cannot_create_unlimited_split() {
        let config = reserved(0, 0, 0);
        assert!(resource_minimum_error(&config, 400, 10_000, 0, 512, 512).is_some());
        assert!(resource_minimum_error(&config, 400, 10_000, 50, 0, 512).is_some());
        assert!(resource_minimum_error(&config, 400, 10_000, 50, 512, 0).is_some());
        assert!(resource_minimum_error(&config, 400, 10_000, 1, 1, 1).is_none());
    }

    #[test]
    fn unlimited_master_may_create_unlimited_cpu_and_disk() {
        let config = reserved(0, 0, 0);
        assert!(resource_minimum_error(&config, 0, 0, 0, 512, 0).is_none());
    }

    #[test]
    fn reserved_amount_is_the_minimum_split_size() {
        let config = reserved(10, 128, 256);
        assert!(resource_minimum_error(&config, 400, 10_000, 9, 128, 256).is_some());
        assert!(resource_minimum_error(&config, 400, 10_000, 10, 127, 256).is_some());
        assert!(resource_minimum_error(&config, 400, 10_000, 10, 128, 255).is_some());
        assert!(resource_minimum_error(&config, 400, 10_000, 10, 128, 256).is_none());
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
