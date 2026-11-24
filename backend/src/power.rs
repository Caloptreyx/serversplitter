//! The power action a user may run on a master after one of its splits was created, resized or deleted.
//! Checked before the split change, sent through the master's node after it committed, the way the
//! panel's own power route sends it. A failed action never undoes the split change; it is reported.

use crate::settings::{MasterPower, ServerSplitterSettingsData};
use axum::http::StatusCode;
use serde::Serialize;
use shared::{
    State,
    models::{ByUuid, server::Server},
    response::DisplayError,
};
use utoipa::ToSchema;
use wings_api::client::ApiHttpError;

/// What a user may pick after a split change.
#[derive(ToSchema, Serialize, Clone, Debug)]
pub struct MasterPowerOptions {
    /// Enabled by the admin, permitted to the user and possible in the master's state.
    pub allowed: Vec<MasterPower>,
    /// Preselected: the admin's default when allowed, else none.
    pub default: MasterPower,
}

/// The outcome of the power action sent after a split change.
#[derive(ToSchema, Serialize, Clone, Debug)]
pub struct MasterPowerResult {
    pub action: MasterPower,
    pub ok: bool,
    pub error: Option<String>,
}

fn verb(action: MasterPower) -> &'static str {
    match action {
        MasterPower::None => "leave",
        MasterPower::Restart => "restart",
        MasterPower::Stop => "stop",
        MasterPower::Kill => "kill",
    }
}

/// The actions offered for `master`. `permitted` tells whether the acting user may run one.
pub fn options(
    config: &ServerSplitterSettingsData,
    master: &Server,
    permitted: impl Fn(MasterPower) -> bool,
) -> MasterPowerOptions {
    let allowed: Vec<MasterPower> = if master.unavailable_reason().is_some() {
        Vec::new()
    } else {
        MasterPower::ACTIONS
            .into_iter()
            .filter(|action| config.master_power_enabled(*action) && permitted(*action))
            .collect()
    };
    let default = if allowed.contains(&config.default_master_power) {
        config.default_master_power
    } else {
        MasterPower::None
    };

    MasterPowerOptions { allowed, default }
}

/// Why `action` can't run on `master` after the split change, checked before anything changes.
pub fn check(
    config: &ServerSplitterSettingsData,
    master: &Server,
    action: MasterPower,
    permitted: bool,
) -> Result<(), anyhow::Error> {
    if action == MasterPower::None {
        return Ok(());
    }

    let reject = |status: StatusCode, message: String| -> anyhow::Error {
        DisplayError::new(message).with_status(status).into()
    };
    if !config.master_power_enabled(action) {
        return Err(reject(
            StatusCode::BAD_REQUEST,
            format!("Choosing to {} the master isn't enabled.", verb(action)),
        ));
    }
    if !permitted {
        return Err(reject(
            StatusCode::FORBIDDEN,
            format!("You don't have permission to {} the master.", verb(action)),
        ));
    }
    if let Some(reason) = master.unavailable_reason() {
        return Err(reject(
            StatusCode::CONFLICT,
            format!("Can't {} the master: {reason}.", verb(action)),
        ));
    }

    Ok(())
}

fn wings_error(err: ApiHttpError) -> String {
    match err {
        ApiHttpError::Http(status, err) => format!("the node answered {status}: {}", err.error),
        err => format!("the node couldn't be reached: {}", anyhow::Error::from(err)),
    }
}

/// Pushes the master's committed limits to its node and then sends `action`, so a restart already
/// runs with the master's new share. `None` when no action was chosen.
pub async fn send(
    state: &State,
    master_uuid: uuid::Uuid,
    action: MasterPower,
) -> Option<MasterPowerResult> {
    let wings_action = action.wings()?;

    let result = async {
        Server::invalidate_cached(&state.database, master_uuid).await;
        let master = Server::by_uuid(&state.database, master_uuid)
            .await
            .map_err(|err| format!("the master couldn't be loaded: {err}"))?;
        let node = master
            .node
            .fetch_cached(&state.database)
            .await
            .map_err(|err| format!("the master's node couldn't be loaded: {err}"))?;
        let client = node
            .api_client(&state.database)
            .await
            .map_err(|err| format!("the master's node couldn't be loaded: {err}"))?;

        master.sync(state).await.map_err(|err| {
            format!("its new limits couldn't be sent to the node, so it was left as it is: {err}")
        })?;

        client
            .post_servers_server_power(
                master_uuid,
                &wings_api::servers_server_power::post::RequestBody {
                    action: wings_action,
                    wait_seconds: None,
                },
            )
            .await
            .map_err(wings_error)?;

        Ok::<_, String>(())
    }
    .await;

    if let Err(err) = &result {
        tracing::warn!(master = %master_uuid, "failed to {} the master after a split change: {err}", verb(action));
    }

    Some(MasterPowerResult {
        action,
        ok: result.is_ok(),
        error: result.err(),
    })
}

/// The master's power state on its node (cached by the panel for a few seconds), `None` when the node
/// couldn't be asked.
pub async fn state_of(state: &State, master: &Server) -> Option<wings_api::ServerState> {
    let node = master.node.fetch_cached(&state.database).await.ok()?;
    let resources = node.fetch_server_resources(&state.database).await.ok()?;
    resources.get(&master.uuid).map(|usage| usage.state)
}
