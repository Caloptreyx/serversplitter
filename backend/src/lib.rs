use indexmap::IndexMap;
use shared::{
    State,
    extensions::{Extension, ExtensionPermissionsBuilder, ExtensionRouteBuilder},
    models::{
        BaseModel,
        server::{ApiServerFeatureLimits, Server},
    },
    permissions::PermissionGroup,
};
use std::sync::Arc;

pub mod health;
pub mod hooks;
pub mod model;
pub mod pool;
pub mod power;
pub mod routes;
pub mod settings;

#[derive(Default)]
pub struct ExtensionStruct;

#[async_trait::async_trait]
impl Extension for ExtensionStruct {
    async fn initialize(&mut self, state: State) {
        tracing::info!("Server Splitter extension initialized");

        Server::register_model_extension(model::ServerExtension);
        hooks::register();

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

        health::spawn_periodic_check(state);
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
                            "Allows resizing server splits on this server. Syncing subusers also needs the subuser permissions.",
                        ),
                        (
                            "delete",
                            "Allows deleting server splits on this server, with their files.",
                        ),
                    ]),
                },
            )
            .mutate_admin_permission_group("extensions", |group| {
                group.add_permission(
                    "splitter.read",
                    "Allows viewing server splitter settings, egg rules and split pools.",
                );
                group.add_permission(
                    "splitter.write",
                    "Allows modifying server splitter settings and egg rules, and reconciling split pools.",
                );
            })
    }
}
