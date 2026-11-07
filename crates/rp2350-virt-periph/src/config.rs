//! Configuration schema for plugin parameters (JSON Schema-based)
//! 插件参数的配置模式（基于 JSON Schema）

use serde::{Deserialize, Serialize};

use super::error::ConfigError;

/// A JSON Schema describing the configuration shape accepted by a plugin.
/// 描述插件接受的配置形状的 JSON Schema。
///
/// Plugins return this via `DevicePlugin::config_schema()` to enable
/// structured configuration editing (both GUI and MCP).
/// 插件通过 `DevicePlugin::config_schema()` 返回此结构体，
/// 使能结构化配置编辑（GUI 和 MCP 均可用）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigSchema {
    /// Schema title / Schema 标题
    pub title: String,
    /// Schema description / Schema 描述
    pub description: String,
    /// JSON Schema properties / JSON Schema 属性
    #[serde(rename = "type")]
    pub schema_type: String,
    /// Properties map / 属性映射
    pub properties: serde_json::Value,
    /// Required field names / 必填字段名
    pub required: Vec<String>,
}

impl ConfigSchema {
    /// Create a new ConfigSchema / 创建新的 ConfigSchema
    #[must_use]
    pub fn new(title: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            description: description.into(),
            schema_type: "object".to_string(),
            properties: serde_json::json!({}),
            required: Vec::new(),
        }
    }

    /// Add a property to the schema / 向 schema 添加属性
    #[must_use]
    pub fn with_property(mut self, name: &str, prop: serde_json::Value, required: bool) -> Self {
        if let Some(obj) = self.properties.as_object_mut() {
            obj.insert(name.to_string(), prop);
        }
        if required {
            self.required.push(name.to_string());
        }
        self
    }

    /// Validate a JSON config value against this schema (stub)
    /// 根据此 schema 验证 JSON 配置值（桩实现）
    ///
    /// # Errors
    /// Returns `ConfigError` if required fields are missing
    pub fn validate(&self, config: &serde_json::Value) -> Result<(), ConfigError> {
        for field in &self.required {
            if config.get(field).is_none() {
                return Err(ConfigError::MissingField(field.clone()));
            }
        }
        Ok(())
    }
}