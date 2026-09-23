use indexmap::IndexMap;
use shared::{
    Extendible, State,
    extensions::{Extension, ExtensionPermissionsBuilder, ExtensionRouteBuilder},
    models::{
        BaseModel, ByUuid, CreatableModel, DeletableModel, ListenerPriority, UpdatableModel,
        server::{ApiServerFeatureLimits, Server},
    },
    permissions::PermissionGroup,
};
use std::sync::Arc;

pub mod model;
pub mod routes;
pub mod settings;

#[derive(Default)]
pub struct ExtensionStruct;

#[async_trait::async_trait]
impl Extension for ExtensionStruct {
    async fn initialize(&mut self, state: State) {
        tracing::info!("Server Splitter extension initialized");

        // Safe DDL initialization fallback
        let _ = sqlx::query(
            r#"
            ALTER TABLE "servers" ADD COLUMN IF NOT EXISTS "parent_uuid" UUID REFERENCES "servers"("uuid") ON DELETE CASCADE;
            ALTER TABLE "servers" ADD COLUMN IF NOT EXISTS "splits" INTEGER NOT NULL DEFAULT 0;
            CREATE INDEX IF NOT EXISTS "servers_parent_uuid_idx" ON "servers"("parent_uuid");
            "#,
        )
        .execute(state.database.write())
        .await;

        // 1. Register ModelExtension
        Server::register_model_extension(model::ServerExtension);

        // 2. Register Server CREATE handler: `feature_limits.splits` from the payload, else the
        //    configured default split limit
        Server::register_create_handler(
            ListenerPriority::Normal,
            |options, query_builder, state, _transaction| {
                Box::pin(async move {
                    let splits = match options
                        .feature_limits
                        .parse_extended::<model::ExtendedApiServerFeatureLimits>()
                        .ok()
                        .and_then(|extended| extended.splits)
                    {
                        Some(splits) => splits,
                        None => state
                            .settings
                            .get()
                            .await
                            .and_then(|settings| {
                                settings
                                    .find_extension_settings::<settings::ServerSplitterSettingsData>()
                                    .map(|config| config.default_splits)
                            })
                            .unwrap_or(0),
                    };
                    query_builder.set("splits", splits.max(0));
                    Ok(())
                })
            },
        );

        // 3. Register Server UPDATE handler (for splits feature limit)
        Server::register_update_handler(
            ListenerPriority::Normal,
            |_server, options, query_builder, _state, _transaction| {
                Box::pin(async move {
                    if let Some(feature_limits) = &options.feature_limits
                        && let Ok(extended) =
                            feature_limits.parse_extended::<model::ExtendedApiServerFeatureLimits>()
                        && let Some(value) = extended.splits
                    {
                        query_builder.set("splits", Some(value));
                    }
                    Ok(())
                })
            },
        );

        // 4. Register Server DELETE handlers
        Server::register_delete_handler(
            ListenerPriority::Normal,
            |server, _options, _state, transaction| {
                Box::pin(async move {
                    if let Ok(ext) = server.parse_model_extension::<model::ServerExtension>()
                        && let Some(parent_uuid) = ext.parent_uuid
                    {
                        let _ = sqlx::query(
                                r#"
                                UPDATE servers
                                SET
                                    cpu = CASE WHEN cpu > 0 AND $1 > 0 THEN cpu + $1 ELSE cpu END,
                                    memory = memory + $2,
                                    disk = CASE WHEN disk > 0 AND $3 > 0 THEN disk + $3 ELSE disk END,
                                    allocation_limit = allocation_limit + $4,
                                    database_limit = database_limit + $5,
                                    backup_limit = backup_limit + $6,
                                    schedule_limit = schedule_limit + $7
                                WHERE uuid = $8
                                "#,
                            )
                            .bind(server.cpu)
                            .bind(server.memory)
                            .bind(server.disk)
                            .bind(server.allocation_limit)
                            .bind(server.database_limit)
                            .bind(server.backup_limit)
                            .bind(server.schedule_limit)
                            .bind(parent_uuid)
                            .execute(&mut **transaction)
                            .await;
                    }
                    Ok(())
                })
            },
        );

        Server::register_after_delete_handler(
            ListenerPriority::Normal,
            |server, _options, state, _transaction| {
                Box::pin(async move {
                    if let Ok(ext) = server.parse_model_extension::<model::ServerExtension>()
                        && let Some(parent_uuid) = ext.parent_uuid
                        && let Ok(parent) = Server::by_uuid(&state.database, parent_uuid).await
                    {
                        let database_arc = std::sync::Arc::new(state.database.clone());
                        parent.batch_sync(&database_arc).await;
                    }
                    Ok(())
                })
            },
        );

        // 5. Extend ApiServerFeatureLimits
        ApiServerFeatureLimits::extend_validated(
            |server, _state| {
                Box::pin(
                    async move { Ok(server.parse_model_extension::<model::ServerExtension>()?) },
                )
            },
            |_limits, extension, _state| model::ExtendedApiServerFeatureLimits {
                splits: Some(extension.splits),
            },
        );
    }

    async fn settings_deserializer(
        &self,
        _state: State,
    ) -> shared::extensions::settings::ExtensionSettingsDeserializer {
        Arc::new(settings::ServerSplitterSettingsDeserializer)
    }

    async fn initialize_router(
        &mut self,
        state: State,
        builder: ExtensionRouteBuilder,
    ) -> ExtensionRouteBuilder {
        builder
            .add_admin_api_router(|router| {
                router.nest(
                    "/extensions/com.caloptreyx.serversplitter",
                    routes::admin::router(&state),
                )
            })
            .add_client_server_api_router(|router| {
                router.nest("/splitter", routes::client::router(&state))
            })
    }

    async fn initialize_permissions(
        &mut self,
        _state: State,
        builder: ExtensionPermissionsBuilder,
    ) -> ExtensionPermissionsBuilder {
        builder
            .add_server_permission_group(
                "splitter",
                PermissionGroup {
                    description: "Permissions for the Server Splitter extension.",
                    permissions: IndexMap::from([
                        ("read", "Allows viewing server splits on this server."),
                        ("create", "Allows creating server splits on this server."),
                        (
                            "update",
                            "Allows updating and syncing server splits on this server.",
                        ),
                        ("delete", "Allows deleting server splits on this server."),
                    ]),
                },
            )
            .mutate_admin_permission_group("extensions", |group| {
                group.add_permission(
                    "splitter.read",
                    "Allows viewing server splitter settings and egg rules.",
                );
                group.add_permission(
                    "splitter.write",
                    "Allows modifying server splitter settings and egg rules.",
                );
            })
    }
}
