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

/// How a split's swap relates to its master's.
#[derive(ToSchema, Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SwapMode {
    /// memory/4, taken from the master's swap and capped by what it has left.
    #[default]
    Pooled,
    /// No swap.
    None,
    /// The master's swap value, not taken from it.
    Copy,
}

fn default_true() -> bool {
    true
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
    /// Smallest split size where the master is limited. `None` = the reserve, as before 1.5.3.
    #[serde(default)]
    pub minimum_cpu: Option<i32>,
    #[serde(default)]
    pub minimum_memory: Option<i64>,
    #[serde(default)]
    pub minimum_disk: Option<i64>,
    /// Suspending or unsuspending a master does the same to its splits.
    #[serde(default = "default_true")]
    pub cascade_suspension: bool,
    /// Changing a master's owner moves its splits to the new owner.
    #[serde(default = "default_true")]
    pub cascade_owner: bool,
    /// Permission changes and removals of master subusers reach the splits they are synced to.
    #[serde(default = "default_true")]
    pub mirror_subusers: bool,
    /// Splits get the master's CPU pinning.
    #[serde(default = "default_true")]
    pub inherit_pinned_cpus: bool,
    #[serde(default)]
    pub swap_mode: SwapMode,
}

impl ServerSplitterSettingsData {
    pub fn min_cpu(&self) -> i32 {
        self.minimum_cpu.unwrap_or(self.reserved_cpu).max(1)
    }

    pub fn min_memory(&self) -> i64 {
        self.minimum_memory.unwrap_or(self.reserved_memory).max(1)
    }

    pub fn min_disk(&self) -> i64 {
        self.minimum_disk.unwrap_or(self.reserved_disk).max(1)
    }
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
            minimum_cpu: None,
            minimum_memory: None,
            minimum_disk: None,
            cascade_suspension: true,
            cascade_owner: true,
            mirror_subusers: true,
            inherit_pinned_cpus: true,
            swap_mode: SwapMode::Pooled,
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

        // 1.5.3 fields: the minimum split size follows the reserve, the cascades are on, swap is pooled
        assert_eq!(
            (config.min_cpu(), config.min_memory(), config.min_disk()),
            (10, 128, 256)
        );
        assert!(config.cascade_suspension && config.cascade_owner && config.mirror_subusers);
        assert!(config.inherit_pinned_cpus);
        assert_eq!(config.swap_mode, SwapMode::Pooled);
    }
}
