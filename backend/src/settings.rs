use serde::{Deserialize, Serialize};
use shared::extensions::settings::{
    ExtensionSettings, SettingsDeserializeExt, SettingsDeserializer, SettingsSerializeExt,
    SettingsSerializer,
};
use utoipa::ToSchema;

#[derive(ToSchema, Serialize, Deserialize, Clone, Debug)]
pub struct EggRule {
    pub id: uuid::Uuid,
    pub eggs: Vec<uuid::Uuid>,
    pub allowed_eggs: Vec<uuid::Uuid>,
}

#[derive(ToSchema, Serialize, Deserialize, Clone, Debug)]
pub struct ServerSplitterSettingsData {
    pub reserved_cpu: i32,
    pub reserved_memory: i64,
    pub reserved_disk: i64,
    pub include_disk_usage: bool,
    pub display_reserved_limits: bool,
    pub egg_rules: Vec<EggRule>,
    /// Split limit given to new servers created without an explicit `splits` feature limit.
    /// Defaulted so configs saved before this field existed still load.
    #[serde(default)]
    pub default_splits: i32,
}

impl Default for ServerSplitterSettingsData {
    fn default() -> Self {
        Self {
            reserved_cpu: 10,
            reserved_memory: 128,
            reserved_disk: 256,
            include_disk_usage: true,
            display_reserved_limits: true,
            egg_rules: Vec::new(),
            default_splits: 0,
        }
    }
}

#[async_trait::async_trait]
impl SettingsSerializeExt for ServerSplitterSettingsData {
    async fn serialize(
        &self,
        serializer: SettingsSerializer,
    ) -> Result<SettingsSerializer, anyhow::Error> {
        Ok(serializer.write_serde_setting("config", self)?)
    }
}

pub struct ServerSplitterSettingsDeserializer;

#[async_trait::async_trait]
impl SettingsDeserializeExt for ServerSplitterSettingsDeserializer {
    async fn deserialize_boxed(
        &self,
        deserializer: SettingsDeserializer<'_>,
    ) -> Result<ExtensionSettings, anyhow::Error> {
        let data: ServerSplitterSettingsData = deserializer
            .read_serde_setting("config")
            .unwrap_or_default();
        Ok(Box::new(data))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A config saved before `default_splits` existed must still load: a failed load falls back
    // to the defaults, which would silently drop every egg rule.
    #[test]
    fn config_saved_before_default_splits_keeps_its_values() {
        let stored = r#"{"reserved_cpu":10,"reserved_memory":128,"reserved_disk":256,"include_disk_usage":true,"display_reserved_limits":true,"egg_rules":[{"id":"18a6ac64-df1b-4b2c-a327-8dd4e79a3ffb","eggs":["10664c3a-30a1-402b-a7f9-b2c45fb7a58f"],"allowed_eggs":["10664c3a-30a1-402b-a7f9-b2c45fb7a58f"]}]}"#;

        let config: ServerSplitterSettingsData = serde_json::from_str(stored).unwrap();
        assert_eq!(config.egg_rules.len(), 1);
        assert_eq!(config.default_splits, 0);
    }
}
