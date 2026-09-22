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
