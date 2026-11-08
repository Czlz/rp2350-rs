//! `WiringTable` — bidirectional (device, port) ↔ GPIO pin mapping
//! `WiringTable` — 双向 (设备, 端口) ↔ GPIO 引脚映射

// 接线表文档注释中使用内部标识符来描述数据结构字段映射关系。
// Wiring table doc comments use internal identifiers to describe data
// structure field mapping relationships.
#![allow(clippy::doc_markdown, clippy::module_name_repetitions)]

use serde::{Deserialize, Serialize};

/// A single wiring entry connecting a device port to a GPIO pin
/// 连接设备端口到 GPIO 引脚的单个接线条目
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WiringEntry {
    /// Device instance ID / 设备实例 ID
    pub device_id: String,
    /// Port name on the device / 设备上的端口名
    pub port_name: String,
    /// Port index (cached for fast lookup) / 端口索引（缓存用于快速查找）
    pub port_index: usize,
    /// GPIO pin number / GPIO 引脚号
    pub gpio_pin: u8,
}

/// Bidirectional mapping between (device, port) and GPIO pins
/// 双向 (设备, 端口) ↔ GPIO 引脚的映射
///
/// Supports:
/// - Multiple ports connected to the same GPIO pin (shared bus)
/// - 多个端口连接到同一个 GPIO 引脚（共享总线）
/// - Lookup by (device_id, port_index) → gpio_pin
/// - Lookup by gpio_pin → all connected (device_id, port_index) pairs
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WiringTable {
    /// All wiring entries / 所有接线条目
    entries: Vec<WiringEntry>,
}

impl WiringTable {
    /// Create an empty wiring table / 创建空的接线表
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Add a wiring connection / 添加接线连接
    pub fn wire(&mut self, device_id: &str, port_name: &str, port_index: usize, gpio_pin: u8) {
        // Remove any existing entry for the same (device, port)
        self.entries.retain(|e| {
            !(e.device_id == device_id && e.port_name == port_name && e.port_index == port_index)
        });
        self.entries.push(WiringEntry {
            device_id: device_id.to_string(),
            port_name: port_name.to_string(),
            port_index,
            gpio_pin,
        });
    }

    /// Remove a specific wiring connection / 移除特定接线连接
    pub fn unwire(&mut self, device_id: &str, port_name: &str, port_index: usize) {
        self.entries.retain(|e| {
            !(e.device_id == device_id && e.port_name == port_name && e.port_index == port_index)
        });
    }

    /// Remove all wirings for a device / 移除设备的所有接线
    pub fn remove_device(&mut self, device_id: &str) {
        self.entries.retain(|e| e.device_id != device_id);
    }

    /// Find all wiring entries for a given GPIO pin
    /// 查找给定 GPIO 引脚的所有接线条目
    #[must_use]
    pub fn find_by_gpio(&self, gpio_pin: u8) -> Vec<&WiringEntry> {
        self.entries
            .iter()
            .filter(|e| e.gpio_pin == gpio_pin)
            .collect()
    }

    /// Find the GPIO pin for a given device port
    /// 查找给定设备端口连接的 GPIO 引脚
    #[must_use]
    pub fn find_by_device_port(&self, device_id: &str, port_index: usize) -> Option<u8> {
        self.entries
            .iter()
            .find(|e| e.device_id == device_id && e.port_index == port_index)
            .map(|e| e.gpio_pin)
    }

    /// Return all entries / 返回所有条目
    #[must_use]
    pub fn all(&self) -> &[WiringEntry] {
        &self.entries
    }

    /// Clear all wirings / 清除所有接线
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Number of wiring entries / 接线条目数量
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Check if the wiring table is empty / 检查接线表是否为空
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_and_find() {
        let mut table = WiringTable::new();
        table.wire("led-1", "LED", 0, 25);
        assert_eq!(table.len(), 1);
        assert_eq!(table.find_by_device_port("led-1", 0), Some(25));
    }

    #[test]
    fn same_port_rewired() {
        let mut table = WiringTable::new();
        table.wire("led-1", "LED", 0, 25);
        table.wire("led-1", "LED", 0, 26);
        assert_eq!(table.find_by_device_port("led-1", 0), Some(26));
        assert_eq!(table.len(), 1);
    }

    #[test]
    fn multiple_devices_on_same_gpio() {
        let mut table = WiringTable::new();
        table.wire("led-1", "LED", 0, 25);
        table.wire("btn-1", "BTN", 0, 25);
        let entries = table.find_by_gpio(25);
        assert_eq!(entries.len(), 2);
    }
}
