use indexmap::IndexMap;
use shared::{
    Extendible, State,
    extensions::{Extension, ExtensionPermissionsBuilder, ExtensionRouteBuilder},
    models::{
        BaseModel, CreatableModel, DeletableModel, ListenerPriority, UpdatableModel,
        server::{ApiServerFeatureLimits, DeleteServerOptions, Server},
    },
    permissions::PermissionGroup,
};
use std::sync::Arc;

pub mod model;
pub mod pool;
pub mod routes;
pub mod settings;

#[derive(Default)]
pub struct ExtensionStruct;

#[async_trait::async_trait]
impl Extension for ExtensionStruct {
    async fn initialize(&mut self, _state: State) {
        tracing::info!("Server Splitter extension initialized");

        // 1. Register ModelExtension
        Server::register_model_extension(model::ServerExtension);

        // 2. Register Server CREATE handler. A split is inserted already linked to its master,
        //    taking over the master's allocation in the same transaction. Other servers get
        //    `feature_limits.splits` from the payload, else the configured default split limit.
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

        // 4. Register Server DELETE handler. Runs inside the panel's delete transaction, which
        //    commits only once Wings has deleted the server.
        Server::register_delete_handler(
            ListenerPriority::Normal,
            |server, options, state, transaction| {
                Box::pin(async move {
                    let data = routes::client::splitter_data(server);

                    // A split: return what it holds to its master, under the master's lock so a
                    // concurrent resize is credited at its committed size, then push the master's
                    // new limits to Wings once this transaction ends.
                    if let Some(parent_uuid) = data.parent_uuid {
                        if pool::lock_master(transaction, parent_uuid).await?.is_some() {
                            pool::credit_master(transaction, server.uuid).await?;
                            pool::refresh_and_sync_after_unlock(state.clone(), parent_uuid);
                        }
                        return Ok(());
                    }

                    // A master: delete its splits properly first. Removing their rows along with
                    // the master would leave their containers running on Wings and their
                    // databases on the database hosts.
                    for split in routes::client::get_subservers(state, server.uuid).await? {
                        split
                            .delete(
                                state,
                                DeleteServerOptions {
                                    force: options.force,
                                },
                            )
                            .await?;
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
