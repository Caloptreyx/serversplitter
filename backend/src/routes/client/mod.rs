use crate::settings::{EggRule, ServerSplitterSettingsData};
use axum::{extract::Path, http::StatusCode};
use serde::{Deserialize, Serialize};
use shared::{
    GetState, State,
    models::{
        BaseModel, admin_activity::GetAdminActivityLogger, nest::Nest, nest_egg::NestEgg,
        user::GetPermissionManager,
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
pub struct SettingsResponse {
    pub reserved_cpu: i32,
    pub reserved_memory: i64,
    pub reserved_disk: i64,
    pub include_disk_usage: bool,
    pub display_reserved_limits: bool,
    pub egg_rules: Vec<EggRule>,
    pub eggs: Vec<EggItem>,
}

#[derive(ToSchema, Deserialize)]
pub struct UpdateSettingsPayload {
    pub reserved_cpu: i32,
    pub reserved_memory: i64,
    pub reserved_disk: i64,
    pub include_disk_usage: bool,
    pub display_reserved_limits: bool,
}

#[derive(ToSchema, Deserialize)]
pub struct CreateEggRulePayload {
    pub eggs: Vec<uuid::Uuid>,
    pub allowed_eggs: Vec<uuid::Uuid>,
}

#[derive(ToSchema, Deserialize)]
pub struct UpdateEggRulePayload {
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

mod get_settings {
    use super::*;

    #[utoipa::path(get, path = "/settings", responses(
        (status = OK, body = inline(SettingsResponse)),
    ))]
    pub async fn route(state: GetState, permissions: GetPermissionManager) -> ApiResponseResult {
        permissions.has_admin_permission("extensions.splitter.read")?;

        let settings = state.settings.get().await?;
        let ext_settings: &ServerSplitterSettingsData = settings.find_extension_settings()?;

        let eggs = get_all_eggs(&state).await?;

        ApiResponse::new_serialized(SettingsResponse {
            reserved_cpu: ext_settings.reserved_cpu,
            reserved_memory: ext_settings.reserved_memory,
            reserved_disk: ext_settings.reserved_disk,
            include_disk_usage: ext_settings.include_disk_usage,
            display_reserved_limits: ext_settings.display_reserved_limits,
            egg_rules: ext_settings.egg_rules.clone(),
            eggs,
        })
        .ok()
    }
}

mod put_settings {
    use super::*;

    #[utoipa::path(put, path = "/settings", responses(
        (status = OK, body = inline(EmptyResponse)),
    ), request_body = inline(UpdateSettingsPayload))]
    pub async fn route(
        state: GetState,
        permissions: GetPermissionManager,
        activity_logger: GetAdminActivityLogger,
        shared::Payload(data): shared::Payload<UpdateSettingsPayload>,
    ) -> ApiResponseResult {
        permissions.has_admin_permission("extensions.splitter.write")?;

        let mut settings = state.settings.get_mut().await?;
        let ext_settings: &mut ServerSplitterSettingsData =
            settings.find_mut_extension_settings()?;

        ext_settings.reserved_cpu = data.reserved_cpu;
        ext_settings.reserved_memory = data.reserved_memory;
        ext_settings.reserved_disk = data.reserved_disk;
        ext_settings.include_disk_usage = data.include_disk_usage;
        ext_settings.display_reserved_limits = data.display_reserved_limits;

        settings.save().await?;

        activity_logger
            .log(
                "settings:splitter:update",
                serde_json::json!({
                    "reserved_cpu": data.reserved_cpu,
                    "reserved_memory": data.reserved_memory,
                    "reserved_disk": data.reserved_disk,
                    "include_disk_usage": data.include_disk_usage,
                    "display_reserved_limits": data.display_reserved_limits,
                }),
            )
            .await;

        ApiResponse::new_serialized(EmptyResponse {}).ok()
    }
}

mod post_egg_rule {
    use super::*;

    #[utoipa::path(post, path = "/egg-rules", responses(
        (status = CREATED, body = inline(EggRule)),
    ), request_body = inline(CreateEggRulePayload))]
    pub async fn route(
        state: GetState,
        permissions: GetPermissionManager,
        activity_logger: GetAdminActivityLogger,
        shared::Payload(data): shared::Payload<CreateEggRulePayload>,
    ) -> ApiResponseResult {
        permissions.has_admin_permission("extensions.splitter.write")?;

        let new_rule = EggRule {
            id: uuid::Uuid::new_v4(),
            eggs: data.eggs,
            allowed_eggs: data.allowed_eggs,
        };

        let mut settings = state.settings.get_mut().await?;
        let ext_settings: &mut ServerSplitterSettingsData =
            settings.find_mut_extension_settings()?;

        ext_settings.egg_rules.push(new_rule.clone());
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
    ), request_body = inline(UpdateEggRulePayload))]
    pub async fn route(
        state: GetState,
        permissions: GetPermissionManager,
        activity_logger: GetAdminActivityLogger,
        Path(rule_id): Path<uuid::Uuid>,
        shared::Payload(data): shared::Payload<UpdateEggRulePayload>,
    ) -> ApiResponseResult {
        permissions.has_admin_permission("extensions.splitter.write")?;

        let mut settings = state.settings.get_mut().await?;
        let ext_settings: &mut ServerSplitterSettingsData =
            settings.find_mut_extension_settings()?;

        let Some(rule) = ext_settings.egg_rules.iter_mut().find(|r| r.id == rule_id) else {
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
        let ext_settings: &mut ServerSplitterSettingsData =
            settings.find_mut_extension_settings()?;

        let initial_len = ext_settings.egg_rules.len();
        ext_settings.egg_rules.retain(|r| r.id != rule_id);

        if ext_settings.egg_rules.len() == initial_len {
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

pub fn router(state: &State) -> OpenApiRouter<State> {
    OpenApiRouter::new()
        .routes(routes!(get_settings::route))
        .routes(routes!(put_settings::route))
        .routes(routes!(post_egg_rule::route))
        .routes(routes!(put_egg_rule::route))
        .routes(routes!(delete_egg_rule::route))
        .with_state(state.clone())
}
