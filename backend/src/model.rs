use garde::Validate;
use serde::{Deserialize, Deserializer, Serialize, de::Error as _};
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
    #[serde(default, deserialize_with = "empty_as_none")]
    pub splits: Option<i32>,
}

/// Unset `splits` means "use the default split limit". Besides a missing field or `null`, the
/// admin create form sends `""` once its optional splits field has been typed in and cleared.
fn empty_as_none<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<i32>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Splits {
        Number(i32),
        Text(String),
    }

    match Option::<Splits>::deserialize(deserializer)? {
        None => Ok(None),
        Some(Splits::Number(splits)) => Ok(Some(splits)),
        Some(Splits::Text(text)) if text.trim().is_empty() => Ok(None),
        Some(Splits::Text(_)) => Err(D::Error::custom("splits must be a number")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn splits(json: &str) -> Result<Option<i32>, serde_json::Error> {
        serde_json::from_str::<ExtendedApiServerFeatureLimits>(json).map(|limits| limits.splits)
    }

    #[test]
    fn unset_splits_fall_back_to_the_default() {
        assert_eq!(splits("{}").unwrap(), None);
        assert_eq!(splits(r#"{"splits":null}"#).unwrap(), None);
        assert_eq!(splits(r#"{"splits":""}"#).unwrap(), None);
    }

    #[test]
    fn explicit_splits_are_kept() {
        assert_eq!(splits(r#"{"splits":0}"#).unwrap(), Some(0));
        assert_eq!(splits(r#"{"splits":3}"#).unwrap(), Some(3));
        assert!(splits(r#"{"splits":"three"}"#).is_err());
    }
}
