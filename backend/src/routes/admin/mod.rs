use crate::{
    health::{self, PoolIssue, ReconcileResult, SplitHealth},
    pool::{self, Charged, Limits},
    power::{self, MasterPowerOptions, MasterPowerResult},
    routes::client::{
        Amounts, ParentServer, ResourceAmounts, UpdateSplitPayload, entry_or_untracked,
        get_subservers, overcommit_amounts, resize_bounds, resize_split, splitter_data,
    },
    settings::{EggRule, ExternalResize, MasterPower, ServerSplitterSettingsData, SwapMode},
};
use axum::{
    extract::{Path, Query},
    http::StatusCode,
};
use garde::Validate;
use serde::{Deserialize, Serialize};
use shared::{
    ApiError, GetState, State,
    models::{
        BaseModel, ByUuid, admin_activity::GetAdminActivityLogger, nest::Nest, nest_egg::NestEgg,
        server::Server, user::GetPermissionManager,
    },
    response::{ApiResponse, ApiResponseResult},
};
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};

#[derive(ToSchema, Serialize)]
pub struct EmptyResponse {}

#[derive(ToSchema, Serialize)]
pub struct EggItem {
    pub uuid: uuid::Uuid,
    pub name: compact_str::CompactString,
    pub nest_uuid: uuid::Uuid,
    pub nest_name: compact_str::CompactString,
}

#[derive(ToSchema, Serialize)]
pub struct AdminSplitSummary {
    pub uuid: uuid::Uuid,
    pub name: compact_str::CompactString,
    pub limits: Amounts,
    pub ledger: Option<Amounts>,
    pub min: Amounts,
    pub max: Amounts,
    pub health: SplitHealth,
    pub suspended: bool,
}

#[derive(ToSchema, Serialize)]
pub struct ServerSplitsResponse {
    /// The master server, when this server is itself a split.
    pub parent: Option<ParentServer>,
    /// This server's splits, when it is a master server.
    pub splits: Vec<AdminSplitSummary>,
    pub splits_limit: i32,
    /// How this master's limits relate to its plan, when it isn't a split.
    pub plan: Option<AdminPlan>,
    /// The power actions an admin may run on this master after resizing one of its splits.
    pub master_power: Option<AdminMasterPower>,
}

/// A master's limits read against its plan: `master + splits - overcommit = total`.
#[derive(ToSchema, Serialize)]
pub struct AdminPlan {
    pub mode: ExternalResize,
    /// The master's own limits (cpu/memory/disk 0 and swap -1 = unlimited).
    pub master: ResourceAmounts,
    /// What its splits were charged.
    pub splits: ResourceAmounts,
    /// The plan total (0 where the master is unlimited, swap -1 = unlimited).
    pub total: ResourceAmounts,
    /// What its splits must free; `None` when nothing is owed.
    pub overcommit: Option<ResourceAmounts>,
    /// Swap counts towards the plan only when splits take it from the master.
    pub swap_pooled: bool,
}

#[derive(ToSchema, Serialize)]
pub struct AdminMasterPower {
    #[serde(flatten)]
    pub options: MasterPowerOptions,
    /// The master's power state on its node; `None` when the node couldn't be asked.
    pub state: Option<wings_api::ServerState>,
}

#[derive(ToSchema, Serialize)]
pub struct ResizeResponse {
    pub master_power: Option<MasterPowerResult>,
}

#[derive(ToSchema, Serialize)]
pub struct SettingsResponse {
    #[serde(flatten)]
    pub settings: SettingsFields,
    pub egg_rules: Vec<EggRule>,
    pub eggs: Vec<EggItem>,
}

#[derive(ToSchema, Serialize, Deserialize, Validate)]
pub struct SettingsFields {
    #[garde(range(min = 0))]
    pub reserved_cpu: i32,
    #[garde(range(min = 0))]
    pub reserved_memory: i64,
    #[garde(range(min = 0))]
    pub reserved_disk: i64,
    #[garde(inner(range(min = 1)))]
    pub minimum_cpu: Option<i32>,
    #[garde(inner(range(min = 1)))]
    pub minimum_memory: Option<i64>,
    #[garde(inner(range(min = 1)))]
    pub minimum_disk: Option<i64>,
    #[garde(range(min = 0))]
    pub default_splits: i32,
    #[garde(skip)]
    pub include_disk_usage: bool,
    #[garde(skip)]
    pub display_reserved_limits: bool,
    #[garde(skip)]
    pub cascade_suspension: bool,
    #[garde(skip)]
    pub cascade_owner: bool,
    #[garde(skip)]
    pub mirror_subusers: bool,
    #[garde(skip)]
    pub inherit_pinned_cpus: bool,
    #[garde(skip)]
    pub swap_mode: SwapMode,
    #[garde(skip)]
    pub external_resize: ExternalResize,
    #[garde(skip)]
    pub master_power_actions: Vec<MasterPower>,
    #[garde(skip)]
    pub default_master_power: MasterPower,
}

impl SettingsFields {
    fn of(config: &ServerSplitterSettingsData) -> Self {
        Self {
            reserved_cpu: config.reserved_cpu,
            reserved_memory: config.reserved_memory,
            reserved_disk: config.reserved_disk,
            minimum_cpu: config.minimum_cpu,
            minimum_memory: config.minimum_memory,
            minimum_disk: config.minimum_disk,
            default_splits: config.default_splits,
            include_disk_usage: config.include_disk_usage,
            display_reserved_limits: config.display_reserved_limits,
            cascade_suspension: config.cascade_suspension,
            cascade_owner: config.cascade_owner,
            mirror_subusers: config.mirror_subusers,
            inherit_pinned_cpus: config.inherit_pinned_cpus,
            swap_mode: config.swap_mode,
            external_resize: config.external_resize,
            master_power_actions: config.master_power_actions.clone(),
            default_master_power: config.default_master_power,
        }
    }
}

#[derive(ToSchema, Deserialize)]
pub struct EggRulePayload {
    pub eggs: Vec<uuid::Uuid>,
    pub allowed_eggs: Vec<uuid::Uuid>,
}

async fn get_all_eggs(state: &State) -> Result<Vec<EggItem>, anyhow::Error> {
    let nest_rows = sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT {} FROM nests ORDER BY nests.name ASC",
        Nest::columns_sql(None)
    )))
    .fetch_all(state.database.read())
    .await?;

    let mut eggs = Vec::new();

    for row in nest_rows {
        let nest = Nest::map(None, &row)?;
        let nest_eggs = NestEgg::all_by_nest_uuid(&state.database, nest.uuid).await?;
        for egg in nest_eggs {
            eggs.push(EggItem {
                uuid: egg.uuid,
                name: egg.name,
                nest_uuid: nest.uuid,
                nest_name: nest.name.clone(),
            });
        }
    }

    Ok(eggs)
}

/// Why an egg rule can't be saved: it needs eggs on both sides, and a master egg may be in one rule
/// only (the first matching rule would win, silently ignoring the other).
fn egg_rule_error(rules: &[EggRule], rule_id: Option<uuid::Uuid>, payload: &EggRulePayload) -> Option<String> {
    if payload.eggs.is_empty() {
        return Some("Pick at least one master egg.".into());
    }
    if payload.allowed_eggs.is_empty() {
        return Some("Pick at least one allowed child egg.".into());
    }

    rules
        .iter()
        .filter(|rule| Some(rule.id) != rule_id)
        .find(|rule| rule.eggs.iter().any(|egg| payload.eggs.contains(egg)))
        .map(|_| "A master egg in this rule is already a master egg of another rule. Each master egg can be in one rule only.".into())
}

mod get_settings {
    use super::*;

    #[utoipa::path(get, path = "/settings", responses(
        (status = OK, body = inline(SettingsResponse)),
    ))]
    pub async fn route(state: GetState, permissions: GetPermissionManager) -> ApiResponseResult {
        permissions.has_admin_permission("extensions.splitter.read")?;

        let settings = state.settings.get().await?;
        let config: &ServerSplitterSettingsData = settings.find_extension_settings()?;
        let response = SettingsResponse {
            settings: SettingsFields::of(config),
            egg_rules: config.egg_rules.clone(),
            eggs: Vec::new(),
        };
        drop(settings);

        ApiResponse::new_serialized(SettingsResponse {
            eggs: get_all_eggs(&state).await?,
            ..response
        })
        .ok()
    }
}

mod put_settings {
    use super::*;

    #[utoipa::path(put, path = "/settings", responses(
        (status = OK, body = inline(EmptyResponse)),
    ), request_body = inline(SettingsFields))]
    pub async fn route(
        state: GetState,
        permissions: GetPermissionManager,
        activity_logger: GetAdminActivityLogger,
        shared::Payload(mut data): shared::Payload<SettingsFields>,
    ) -> ApiResponseResult {
        permissions.has_admin_permission("extensions.splitter.write")?;

        if let Err(errors) = shared::utils::validate_data(&data) {
            return ApiResponse::new_serialized(ApiError::new_strings_value(errors))
                .with_status(StatusCode::BAD_REQUEST)
                .ok();
        }

        // in a fixed order, without none (always allowed) or repeats
        data.master_power_actions = MasterPower::ACTIONS
            .into_iter()
            .filter(|action| data.master_power_actions.contains(action))
            .collect();
        if data.default_master_power != MasterPower::None
            && !data.master_power_actions.contains(&data.default_master_power)
        {
            return ApiResponse::error(
                "The preselected power action must be one of the actions users may choose.",
            )
            .with_status(StatusCode::BAD_REQUEST)
            .ok();
        }

        let mut settings = state.settings.get_mut().await?;
        let config: &mut ServerSplitterSettingsData = settings.find_mut_extension_settings()?;

        config.reserved_cpu = data.reserved_cpu;
        config.reserved_memory = data.reserved_memory;
        config.reserved_disk = data.reserved_disk;
        config.minimum_cpu = data.minimum_cpu;
        config.minimum_memory = data.minimum_memory;
        config.minimum_disk = data.minimum_disk;
        config.default_splits = data.default_splits;
        config.include_disk_usage = data.include_disk_usage;
        config.display_reserved_limits = data.display_reserved_limits;
        config.cascade_suspension = data.cascade_suspension;
        config.cascade_owner = data.cascade_owner;
        config.mirror_subusers = data.mirror_subusers;
        config.inherit_pinned_cpus = data.inherit_pinned_cpus;
        config.swap_mode = data.swap_mode;
        config.external_resize = data.external_resize;
        config.master_power_actions = data.master_power_actions.clone();
        config.default_master_power = data.default_master_power;

        settings.save().await?;

        activity_logger
            .log("settings:splitter:update", serde_json::to_value(&data)?)
            .await;

        ApiResponse::new_serialized(EmptyResponse {}).ok()
    }
}

mod post_egg_rule {
    use super::*;

    #[utoipa::path(post, path = "/egg-rules", responses(
        (status = CREATED, body = inline(EggRule)),
    ), request_body = inline(EggRulePayload))]
    pub async fn route(
        state: GetState,
        permissions: GetPermissionManager,
        activity_logger: GetAdminActivityLogger,
        shared::Payload(data): shared::Payload<EggRulePayload>,
    ) -> ApiResponseResult {
        permissions.has_admin_permission("extensions.splitter.write")?;

        let mut settings = state.settings.get_mut().await?;
        let config: &mut ServerSplitterSettingsData = settings.find_mut_extension_settings()?;

        if let Some(error) = egg_rule_error(&config.egg_rules, None, &data) {
            return ApiResponse::error(&error)
                .with_status(StatusCode::BAD_REQUEST)
                .ok();
        }

        let new_rule = EggRule {
            id: uuid::Uuid::new_v4(),
            eggs: data.eggs,
            allowed_eggs: data.allowed_eggs,
        };
        config.egg_rules.push(new_rule.clone());
        settings.save().await?;

        activity_logger
            .log(
                "settings:splitter.egg_rule.create",
                serde_json::json!({
                    "rule_id": new_rule.id,
                    "eggs": new_rule.eggs,
                    "allowed_eggs": new_rule.allowed_eggs,
                }),
            )
            .await;

        ApiResponse::new_serialized(new_rule)
            .with_status(StatusCode::CREATED)
            .ok()
    }
}

mod put_egg_rule {
    use super::*;

    #[utoipa::path(put, path = "/egg-rules/{id}", responses(
        (status = OK, body = inline(EmptyResponse)),
    ), request_body = inline(EggRulePayload))]
    pub async fn route(
        state: GetState,
        permissions: GetPermissionManager,
        activity_logger: GetAdminActivityLogger,
        Path(rule_id): Path<uuid::Uuid>,
        shared::Payload(data): shared::Payload<EggRulePayload>,
    ) -> ApiResponseResult {
        permissions.has_admin_permission("extensions.splitter.write")?;

        let mut settings = state.settings.get_mut().await?;
        let config: &mut ServerSplitterSettingsData = settings.find_mut_extension_settings()?;

        if let Some(error) = egg_rule_error(&config.egg_rules, Some(rule_id), &data) {
            return ApiResponse::error(&error)
                .with_status(StatusCode::BAD_REQUEST)
                .ok();
        }

        let Some(rule) = config.egg_rules.iter_mut().find(|r| r.id == rule_id) else {
            return ApiResponse::error("egg rule not found")
                .with_status(StatusCode::NOT_FOUND)
                .ok();
        };

        rule.eggs = data.eggs.clone();
        rule.allowed_eggs = data.allowed_eggs.clone();

        settings.save().await?;

        activity_logger
            .log(
                "settings:splitter.egg_rule.update",
                serde_json::json!({
                    "rule_id": rule_id,
                    "eggs": data.eggs,
                    "allowed_eggs": data.allowed_eggs,
                }),
            )
            .await;

        ApiResponse::new_serialized(EmptyResponse {}).ok()
    }
}

mod delete_egg_rule {
    use super::*;

    #[utoipa::path(delete, path = "/egg-rules/{id}", responses(
        (status = OK, body = inline(EmptyResponse)),
    ))]
    pub async fn route(
        state: GetState,
        permissions: GetPermissionManager,
        activity_logger: GetAdminActivityLogger,
        Path(rule_id): Path<uuid::Uuid>,
    ) -> ApiResponseResult {
        permissions.has_admin_permission("extensions.splitter.write")?;

        let mut settings = state.settings.get_mut().await?;
        let config: &mut ServerSplitterSettingsData = settings.find_mut_extension_settings()?;

        let initial_len = config.egg_rules.len();
        config.egg_rules.retain(|r| r.id != rule_id);

        if config.egg_rules.len() == initial_len {
            return ApiResponse::error("egg rule not found")
                .with_status(StatusCode::NOT_FOUND)
                .ok();
        }

        settings.save().await?;

        activity_logger
            .log(
                "settings:splitter.egg_rule.delete",
                serde_json::json!({ "rule_id": rule_id }),
            )
            .await;

        ApiResponse::new_serialized(EmptyResponse {}).ok()
    }
}

async fn splitter_config(state: &State) -> Result<ServerSplitterSettingsData, anyhow::Error> {
    let settings = state.settings.get().await?;
    Ok(settings
        .find_extension_settings::<ServerSplitterSettingsData>()
        .cloned()
        .unwrap_or_default())
}

mod get_server_splits {
    use super::*;

    #[utoipa::path(get, path = "/servers/{server}", responses(
        (status = OK, body = inline(ServerSplitsResponse)),
    ))]
    pub async fn route(
        state: GetState,
        permissions: GetPermissionManager,
        Path(server_identifier): Path<String>,
    ) -> ApiResponseResult {
        permissions.has_admin_permission("servers.read")?;

        // admin server pages are reached by full uuid or short id
        let Some(server) = Server::by_identifier(&state.database, &server_identifier).await? else {
            return ApiResponse::error("server not found")
                .with_status(StatusCode::NOT_FOUND)
                .ok();
        };
        let data = splitter_data(&server);

        if let Some(parent_uuid) = data.parent_uuid {
            let master = Server::by_uuid(&state.database, parent_uuid).await?;
            return ApiResponse::new_serialized(ServerSplitsResponse {
                parent: Some(ParentServer {
                    uuid: master.uuid,
                    name: master.name,
                }),
                splits: Vec::new(),
                splits_limit: 0,
                plan: None,
                master_power: None,
            })
            .ok();
        }

        let config = splitter_config(&state).await?;
        let mut conn = state.database.read().acquire().await?;
        let usage = pool::feature_usage(&mut conn, server.uuid).await?;
        let disk_usage_mb = pool::disk_usage_mb(&state, &server, &config).await.unwrap_or(0);

        let mut splits = Vec::new();
        for split in get_subservers(&state, server.uuid).await? {
            let (entry, tracked) = entry_or_untracked(&mut conn, &split, &server).await?;
            let (min, max) =
                resize_bounds(&mut conn, &config, &server, &usage, disk_usage_mb, &split, &entry)
                    .await?;
            let limits = Limits::of(&split);
            splits.push(AdminSplitSummary {
                uuid: split.uuid,
                name: split.name.clone(),
                limits: limits.into(),
                ledger: tracked.then(|| entry.granted.into()),
                // admins may go below the minimum split size, never to 0 (unlimited) where the
                // master is limited, and never below what the split uses
                min: Limits {
                    cpu: (server.cpu > 0) as i32,
                    memory: (server.memory > 0) as i64,
                    disk: (server.disk > 0) as i64,
                    ..min
                }
                .into(),
                max: max.into(),
                health: if !tracked {
                    SplitHealth::Untracked
                } else if entry.granted != limits {
                    SplitHealth::Drift
                } else {
                    SplitHealth::Balanced
                },
                suspended: split.suspended,
            });
        }

        let ledger = pool::ledger_by_master(&mut conn, server.uuid).await?;
        let owed = pool::overcommit(&mut conn, server.uuid).await?;
        let charged = ledger
            .iter()
            .fold(Charged::default(), |sum, entry| sum.plus(&entry.charged));
        let master_limits = Limits::of(&server);
        let plan = AdminPlan {
            mode: config.external_resize,
            master: master_limits.into(),
            splits: charged.into(),
            total: pool::plan_total(&master_limits, &charged, &owed).into(),
            overcommit: overcommit_amounts(owed),
            swap_pooled: config.swap_mode == SwapMode::Pooled,
        };

        let may_power = permissions.has_admin_permission("nodes.power").is_ok();
        let options = power::options(&config, &server, |_| may_power);
        let master_state = if options.allowed.is_empty() {
            None
        } else {
            power::state_of(&state, &server).await
        };

        ApiResponse::new_serialized(ServerSplitsResponse {
            parent: None,
            splits,
            splits_limit: data.splits,
            plan: Some(plan),
            master_power: Some(AdminMasterPower {
                options,
                state: master_state,
            }),
        })
        .ok()
    }
}

mod patch_resize {
    use super::*;

    #[utoipa::path(patch, path = "/servers/{split}/resize", responses(
        (status = OK, body = inline(ResizeResponse)),
    ), request_body = inline(UpdateSplitPayload))]
    pub async fn route(
        state: GetState,
        permissions: GetPermissionManager,
        activity_logger: GetAdminActivityLogger,
        Path(split_uuid): Path<uuid::Uuid>,
        shared::Payload(data): shared::Payload<UpdateSplitPayload>,
    ) -> ApiResponseResult {
        permissions.has_admin_permission("servers.update")?;

        if let Err(errors) = shared::utils::validate_data(&data) {
            return ApiResponse::new_serialized(ApiError::new_strings_value(errors))
                .with_status(StatusCode::BAD_REQUEST)
                .ok();
        }

        let Some(split) = Server::by_uuid_optional(&state.database, split_uuid).await? else {
            return ApiResponse::error("server not found")
                .with_status(StatusCode::NOT_FOUND)
                .ok();
        };
        let Some(master_uuid) = splitter_data(&split).parent_uuid else {
            return ApiResponse::error("server is not a split")
                .with_status(StatusCode::BAD_REQUEST)
                .ok();
        };

        // the master power action needs the panel's admin power permission (`nodes.power`)
        let master_power = data.master_power;
        let config = splitter_config(&state).await?;
        let master = Server::by_uuid(&state.database, master_uuid).await?;
        power::check(
            &config,
            &master,
            master_power,
            permissions.has_admin_permission("nodes.power").is_ok(),
        )?;

        let resized = tokio::spawn(resize_split(
            state.0.clone(),
            master_uuid,
            split_uuid,
            UpdateSplitPayload {
                name: None,
                description: None,
                ..data
            },
            true,
        ))
        .await??;

        let master_power = {
            let state = state.0.clone();
            tokio::spawn(async move { power::send(&state, master_uuid, master_power).await })
                .await
                .ok()
                .flatten()
        };

        activity_logger
            .log(
                "server:splitter.resize",
                serde_json::json!({
                    "split_uuid": resized.uuid,
                    "master_uuid": master_uuid,
                    "cpu": resized.cpu,
                    "memory": resized.memory,
                    "disk": resized.disk,
                    "allocations": resized.allocation_limit,
                    "databases": resized.database_limit,
                    "backups": resized.backup_limit,
                    "schedules": resized.schedule_limit,
                    "master_power": master_power.as_ref().map(|result| result.action),
                    "master_power_error": master_power.as_ref().and_then(|result| result.error.clone()),
                }),
            )
            .await;

        ApiResponse::new_serialized(ResizeResponse { master_power }).ok()
    }
}

#[derive(ToSchema, Serialize)]
pub struct MasterOwner {
    pub uuid: uuid::Uuid,
    pub username: compact_str::CompactString,
}

#[derive(ToSchema, Serialize)]
pub struct MasterNode {
    pub uuid: uuid::Uuid,
    pub name: compact_str::CompactString,
}

#[derive(ToSchema, Serialize)]
pub struct MasterRow {
    pub uuid: uuid::Uuid,
    pub uuid_short: String,
    pub name: compact_str::CompactString,
    pub owner: MasterOwner,
    pub node: MasterNode,
    pub splits_used: i64,
    pub splits_limit: i32,
    pub master: Amounts,
    pub held: Amounts,
    /// `overcommitted` (its splits must free resources), else `drift` (issues) or `balanced`.
    pub status: &'static str,
    pub issues: Vec<PoolIssue>,
    /// What its splits must free; `None` when nothing is owed.
    pub overcommit: Option<ResourceAmounts>,
}

#[derive(ToSchema, Serialize)]
pub struct MastersPage {
    pub masters: Vec<MasterRow>,
    pub total: usize,
    pub page: usize,
    pub per_page: usize,
}

#[derive(Deserialize, ToSchema)]
pub struct MastersQuery {
    #[serde(default)]
    pub page: Option<usize>,
    #[serde(default)]
    pub per_page: Option<usize>,
    #[serde(default)]
    pub search: Option<String>,
    /// `drift`: only masters whose pool needs attention (issues or an overcommit).
    #[serde(default)]
    pub status: Option<String>,
}

mod get_masters {
    use super::*;

    #[utoipa::path(get, path = "/splits", responses(
        (status = OK, body = inline(MastersPage)),
    ), params(
        ("page" = Option<usize>, Query, description = "1-based page"),
        ("per_page" = Option<usize>, Query, description = "rows per page (max 100)"),
        ("search" = Option<String>, Query, description = "master name, owner username or node name"),
        ("status" = Option<String>, Query, description = "`drift` for masters that need attention (issues or an overcommit)"),
    ))]
    pub async fn route(
        state: GetState,
        permissions: GetPermissionManager,
        Query(query): Query<MastersQuery>,
    ) -> ApiResponseResult {
        permissions.has_admin_permission("extensions.splitter.read")?;
        permissions.has_admin_permission("servers.read")?;

        let page = query.page.unwrap_or(1).max(1);
        let per_page = query.per_page.unwrap_or(25).clamp(1, 100);
        let search = query
            .search
            .as_deref()
            .map(str::trim)
            .filter(|search| !search.is_empty());

        let mut conn = state.database.read().acquire().await?;
        let candidates: Vec<uuid::Uuid> = sqlx::query_scalar(
            r#"
            SELECT m.uuid
            FROM servers m
            JOIN users u ON u.uuid = m.owner_uuid
            JOIN nodes n ON n.uuid = m.node_uuid
            WHERE (
                EXISTS (SELECT 1 FROM servers s WHERE s.parent_uuid = m.uuid)
                OR EXISTS (SELECT 1 FROM serversplitter_splits l WHERE l.master_uuid = m.uuid)
                OR EXISTS (SELECT 1 FROM serversplitter_overcommit o WHERE o.master_uuid = m.uuid)
            )
            AND (
                $1::text IS NULL
                OR m.name ILIKE '%' || $1 || '%'
                OR u.username ILIKE '%' || $1 || '%'
                OR n.name ILIKE '%' || $1 || '%'
            )
            ORDER BY m.name ASC
            "#,
        )
        .bind(search)
        .fetch_all(&mut *conn)
        .await?;

        let mut rows = Vec::new();
        for master_uuid in candidates {
            let issues = health::issues(&mut conn, master_uuid).await?;
            let owed = pool::overcommit(&mut conn, master_uuid).await?;
            if query.status.as_deref() == Some("drift") && issues.is_empty() && owed.is_zero() {
                continue;
            }
            rows.push((master_uuid, issues, owed));
        }

        let total = rows.len();
        let mut masters = Vec::new();
        for (master_uuid, issues, owed) in rows.into_iter().skip((page - 1) * per_page).take(per_page) {
            let Some(master) = Server::by_uuid_optional(&state.database, master_uuid).await? else {
                continue;
            };
            let node = master.node.fetch_cached(&state.database).await?;
            let held = pool::ledger_by_master(&mut conn, master_uuid)
                .await?
                .iter()
                .fold(Limits::default(), |sum, entry| sum.plus(&entry.granted));
            let splits_used: i64 =
                sqlx::query_scalar("SELECT count(*) FROM servers WHERE parent_uuid = $1")
                    .bind(master_uuid)
                    .fetch_one(&mut *conn)
                    .await?;

            masters.push(MasterRow {
                uuid: master.uuid,
                uuid_short: format!("{:08x}", master.uuid_short),
                name: master.name.clone(),
                owner: MasterOwner {
                    uuid: master.owner.uuid,
                    username: master.owner.username.clone(),
                },
                node: MasterNode {
                    uuid: node.uuid,
                    name: node.name.clone(),
                },
                splits_used,
                splits_limit: splitter_data(&master).splits,
                master: Limits::of(&master).into(),
                held: held.into(),
                status: if !owed.is_zero() {
                    "overcommitted"
                } else if issues.is_empty() {
                    "balanced"
                } else {
                    "drift"
                },
                issues,
                overcommit: overcommit_amounts(owed),
            });
        }

        ApiResponse::new_serialized(MastersPage {
            masters,
            total,
            page,
            per_page,
        })
        .ok()
    }
}

mod post_reconcile {
    use super::*;

    #[utoipa::path(post, path = "/splits/{master}/reconcile", responses(
        (status = OK, body = inline(ReconcileResult)),
    ))]
    pub async fn route(
        state: GetState,
        permissions: GetPermissionManager,
        activity_logger: GetAdminActivityLogger,
        Path(master_uuid): Path<uuid::Uuid>,
    ) -> ApiResponseResult {
        permissions.has_admin_permission("extensions.splitter.write")?;

        let result = health::reconcile(&state, master_uuid).await?;

        activity_logger
            .log(
                "server:splitter.reconcile",
                serde_json::json!({
                    "master_uuid": master_uuid,
                    "fixed": result.fixed,
                    "failed": result.failed.len(),
                }),
            )
            .await;

        ApiResponse::new_serialized(result).ok()
    }
}

mod post_reconcile_all {
    use super::*;

    #[utoipa::path(post, path = "/splits/reconcile", responses(
        (status = OK, body = inline(ReconcileResult)),
    ))]
    pub async fn route(
        state: GetState,
        permissions: GetPermissionManager,
        activity_logger: GetAdminActivityLogger,
    ) -> ApiResponseResult {
        permissions.has_admin_permission("extensions.splitter.write")?;

        let masters = {
            let mut conn = state.database.read().acquire().await?;
            let mut masters = Vec::new();
            for master_uuid in health::masters_with_splits(&mut conn).await? {
                if !health::issues(&mut conn, master_uuid).await?.is_empty() {
                    masters.push(master_uuid);
                }
            }
            masters
        };

        let mut result = ReconcileResult::default();
        for master_uuid in &masters {
            let one = health::reconcile(&state, *master_uuid).await?;
            result.fixed += one.fixed;
            result.failed.extend(one.failed);
        }

        activity_logger
            .log(
                "server:splitter.reconcile",
                serde_json::json!({
                    "masters": masters.len(),
                    "fixed": result.fixed,
                    "failed": result.failed.len(),
                }),
            )
            .await;

        ApiResponse::new_serialized(result).ok()
    }
}

pub fn router(state: &State) -> OpenApiRouter<State> {
    OpenApiRouter::new()
        .routes(routes!(get_settings::route))
        .routes(routes!(put_settings::route))
        .routes(routes!(post_egg_rule::route))
        .routes(routes!(put_egg_rule::route))
        .routes(routes!(delete_egg_rule::route))
        .routes(routes!(get_server_splits::route))
        .routes(routes!(patch_resize::route))
        .routes(routes!(get_masters::route))
        .routes(routes!(post_reconcile::route))
        .routes(routes!(post_reconcile_all::route))
        .with_state(state.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(eggs: &[uuid::Uuid]) -> EggRule {
        EggRule {
            id: uuid::Uuid::new_v4(),
            eggs: eggs.to_vec(),
            allowed_eggs: eggs.to_vec(),
        }
    }

    #[test]
    fn a_master_egg_belongs_to_one_rule() {
        let (a, b) = (uuid::Uuid::new_v4(), uuid::Uuid::new_v4());
        let rules = vec![rule(&[a])];

        let overlapping = EggRulePayload {
            eggs: vec![a, b],
            allowed_eggs: vec![b],
        };
        assert!(egg_rule_error(&rules, None, &overlapping).is_some());
        // editing the rule that already holds the egg is fine
        assert!(egg_rule_error(&rules, Some(rules[0].id), &overlapping).is_none());

        let separate = EggRulePayload {
            eggs: vec![b],
            allowed_eggs: vec![a],
        };
        assert!(egg_rule_error(&rules, None, &separate).is_none());
    }

    #[test]
    fn egg_rules_need_eggs_on_both_sides() {
        let egg = uuid::Uuid::new_v4();
        let empty_master = EggRulePayload {
            eggs: vec![],
            allowed_eggs: vec![egg],
        };
        let empty_children = EggRulePayload {
            eggs: vec![egg],
            allowed_eggs: vec![],
        };
        assert!(egg_rule_error(&[], None, &empty_master).is_some());
        assert!(egg_rule_error(&[], None, &empty_children).is_some());
    }
}
