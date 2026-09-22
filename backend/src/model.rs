use garde::Validate;
use serde::{Deserialize, Serialize};
use shared::models::{ModelExtension, SafeModelExtension};
use sqlx::{Row, postgres::PgRow};
use std::collections::BTreeMap;
use utoipa::ToSchema;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ServerSplitterData {
    pub parent_uuid: Option<uuid::Uuid>,
    pub splits: i32,
}

pub struct ServerExtension;

impl SafeModelExtension for ServerExtension {
    type Value = ServerSplitterData;

    fn name() -> &'static str {
        ServerExtension.extension_name()
    }
}

impl ModelExtension for ServerExtension {
    fn extension_name(&self) -> &'static str {
        "com.caloptreyx.serversplitter"
    }

    fn extended_columns(&self, prefix: &str) -> BTreeMap<&'static str, compact_str::CompactString> {
        BTreeMap::from([
            (
                "servers.parent_uuid",
                compact_str::format_compact!("{prefix}parent_uuid"),
            ),
            (
                "servers.splits",
                compact_str::format_compact!("{prefix}splits"),
            ),
        ])
    }

    fn map_extended(
        &self,
        prefix: &str,
        row: &PgRow,
    ) -> Result<shared::models::ModelExtensionMapType, shared::database::DatabaseError> {
        Ok(Box::new(ServerSplitterData {
            parent_uuid: row
                .try_get(compact_str::format_compact!("{prefix}parent_uuid").as_str())
                .unwrap_or(None),
            splits: row
                .try_get(compact_str::format_compact!("{prefix}splits").as_str())
                .unwrap_or(0),
        }))
    }
}

#[derive(ToSchema, Validate, Serialize, Deserialize, Clone, Debug, Default)]
pub struct ExtendedApiServerFeatureLimits {
    #[garde(range(min = 0))]
    #[schema(minimum = 0)]
    pub splits: Option<i32>,
}
