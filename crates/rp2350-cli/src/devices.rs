//! Plugin subcommands — `list-devices` and `wire` / 插件子命令
//!
//! Both subcommands scan a plugin directory, instantiate every device plugin
//! through [`DeviceManager::scan_and_load`] and operate on the resulting
//! manager. The manager lives only for the duration of the process, so every
//! invocation is a **one-shot** operation: the devices (and any wiring) are
//! released when the command returns.
//! 两个子命令都会扫描插件目录、经 [`DeviceManager::scan_and_load`] 实例化其中的
//! 设备插件，并对产出的管理器进行操作。管理器仅存活于进程运行期间，因此每次
//! 调用都是**一次性**操作：命令返回时设备（以及接线）即被释放。
//!
//! `list-devices` additionally supports hot reload: `--reload` performs one
//! rescan and prints the change report, `--watch` keeps polling the directory.
//! `list-devices` 另支持热重载：`--reload` 重新扫描一次并打印变更报告，
//! `--watch` 持续轮询插件目录。

use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use rp2350_virt_periph::{DeviceManager, ReloadReport, WiringEntry};

/// Default plugin directory scanned by the plugin subcommands
/// 插件子命令默认扫描的插件目录
pub const DEFAULT_PLUGIN_DIR: &str = "plugins";

/// Polling interval of `list-devices --watch` / `list-devices --watch` 的轮询间隔
pub const WATCH_INTERVAL: Duration = Duration::from_secs(1);

/// One device as printed by `list-devices` / `list-devices` 打印的单台设备
#[derive(Debug, Clone, Copy)]
pub struct DeviceLine<'a> {
    /// Device ID / 设备 ID
    pub id: &'a str,
    /// Human-readable name / 可读名称
    pub name: &'a str,
    /// Whether the device has a GUI panel / 设备是否有 GUI 面板
    pub has_ui: bool,
    /// Port names in wiring order / 按接线顺序排列的端口名
    pub ports: &'a [&'a str],
}

impl DeviceLine<'_> {
    /// Render the single-line `id / name / has_ui / ports` form
    /// 渲染单行 `id / name / has_ui / ports` 形式
    #[must_use]
    pub fn render(&self) -> String {
        let ports = if self.ports.is_empty() {
            "-".to_string()
        } else {
            self.ports.join(",")
        };
        format!(
            "{} / {} / has_ui={} / ports=[{}]",
            self.id, self.name, self.has_ui, ports
        )
    }
}

/// Render one wiring-table entry / 渲染单个接线条目
#[must_use]
pub fn format_wiring(entry: &WiringEntry) -> String {
    format!(
        "{} / {} / port {} / GPIO{}",
        entry.device_id, entry.port_name, entry.port_index, entry.gpio_pin
    )
}

/// Scan `plugin_dir` and build a fully populated device manager
/// 扫描插件目录并构建包含全部设备的设备管理器
///
/// # Errors
///
/// Returns an error when the plugin directory does not exist. Individual
/// plugin load failures are only logged by [`DeviceManager::scan_and_load`].
/// 插件目录不存在时返回错误；单个插件加载失败仅由
/// [`DeviceManager::scan_and_load`] 记录日志。
fn scan_manager(plugin_dir: &Path) -> Result<DeviceManager> {
    if !plugin_dir.is_dir() {
        bail!(
            "plugin directory not found: {} / 插件目录不存在: {}",
            plugin_dir.display(),
            plugin_dir.display()
        );
    }
    let mut manager = DeviceManager::new();
    let loaded = manager.scan_and_load(plugin_dir);
    log::info!(
        "loaded {} device plugin(s) from {} / 已从插件目录加载设备插件",
        loaded.len(),
        plugin_dir.display()
    );
    Ok(manager)
}

/// `list-devices` — scan the plugin directory and print one line per device
/// `list-devices` — 扫描插件目录并为每台设备打印一行
///
/// # Errors
///
/// Propagates errors from [`scan_manager`] / 透传 [`scan_manager`] 的错误
pub fn list_devices(plugin_dir: &Path) -> Result<()> {
    let manager = scan_manager(plugin_dir)?;
    print_devices(&manager, plugin_dir);
    Ok(())
}

/// Print every device known to `manager`, or the empty-directory notice
/// 打印管理器中的全部设备，或在目录为空时打印提示
fn print_devices(manager: &DeviceManager, plugin_dir: &Path) {
    let devices = manager.mcp_list_devices();
    if devices.is_empty() {
        println!(
            "no device plugins found in {} / 未在插件目录发现设备插件",
            plugin_dir.display()
        );
        return;
    }

    for entry in devices {
        let id = entry["id"].as_str().unwrap_or("<unknown>");
        let name = entry["name"].as_str().unwrap_or("<unnamed>");
        let has_ui = entry["has_ui"].as_bool().unwrap_or(false);
        let port_names: Vec<&str> = entry["ports"]
            .as_array()
            .map(|ports| ports.iter().filter_map(|port| port.as_str()).collect())
            .unwrap_or_default();
        println!(
            "{}",
            DeviceLine {
                id,
                name,
                has_ui,
                ports: &port_names,
            }
            .render()
        );
    }
}

/// Print a reload report / 打印热重载报告
fn print_reload_report(report: &ReloadReport) {
    for line in report.render_lines() {
        println!("{line}");
    }
}

/// `list-devices --reload` — rescan once and print the change report
/// `list-devices --reload` — 重新扫描一次并打印变更报告
///
/// The initial scan is printed like the plain `list-devices` output, followed by
/// the `[reload] …` report; the command then exits.
/// 先按普通 `list-devices` 打印初始扫描结果，随后打印 `[reload] …` 报告并退出。
///
/// # Errors
///
/// Propagates errors from [`scan_manager`] / 透传 [`scan_manager`] 的错误
pub fn list_devices_reload(plugin_dir: &Path) -> Result<()> {
    let mut manager = scan_manager(plugin_dir)?;
    print_devices(&manager, plugin_dir);
    let report = manager.reload(plugin_dir);
    print_reload_report(&report);
    Ok(())
}

/// Dispatch `list-devices` according to its `--reload` / `--watch` flags
/// 根据 `--reload` / `--watch` 标志分发 `list-devices`
///
/// `watch` wins when both flags are somehow set; the CLI rejects that
/// combination at parse time. With both `false` the plain one-shot listing runs.
/// 两个标志同时为真时 `watch` 优先（CLI 在解析阶段已拒绝该组合）；均为 `false`
/// 时执行普通的一次性列表。
///
/// # Errors
///
/// Propagates errors from the selected listing mode / 透传所选模式的错误
pub fn list_devices_with(plugin_dir: &Path, reload: bool, watch: bool) -> Result<()> {
    if watch {
        watch_devices(plugin_dir)
    } else if reload {
        list_devices_reload(plugin_dir)
    } else {
        list_devices(plugin_dir)
    }
}

/// `list-devices --watch` — poll the directory forever until Ctrl-C
/// `list-devices --watch` — 持续轮询插件目录直至 Ctrl-C
///
/// Every change prints its report and the refreshed device list; unchanged
/// polls stay silent.
/// 每次变更会打印报告与刷新后的设备列表；无变更的轮询保持静默。
///
/// # Errors
///
/// Propagates errors from [`scan_manager`] / 透传 [`scan_manager`] 的错误
pub fn watch_devices(plugin_dir: &Path) -> Result<()> {
    watch_devices_with(plugin_dir, WATCH_INTERVAL, None)
}

/// Watch loop with an injectable interval and iteration limit
/// 可注入间隔与迭代上限的监视循环
///
/// `limit = None` polls until the process is interrupted (Ctrl-C terminates the
/// process itself). A finite limit exists so tests never hang.
/// `limit = None` 表示轮询到进程被中断（Ctrl-C 由进程默认处理）；有限上限仅
/// 用于测试，避免挂起。
///
/// # Errors
///
/// Propagates errors from [`scan_manager`] / 透传 [`scan_manager`] 的错误
pub fn watch_devices_with(
    plugin_dir: &Path,
    interval: Duration,
    limit: Option<usize>,
) -> Result<()> {
    let mut manager = scan_manager(plugin_dir)?;
    print_devices(&manager, plugin_dir);
    println!(
        "watching {} every {} ms; press Ctrl-C to stop / 正在监视插件目录（每 {} ms），按 Ctrl-C 停止",
        plugin_dir.display(),
        interval.as_millis(),
        interval.as_millis()
    );

    for _ in 0..limit.unwrap_or(usize::MAX) {
        std::thread::sleep(interval);
        let report = manager.reload(plugin_dir);
        if report.is_empty() {
            continue;
        }
        print_reload_report(&report);
        print_devices(&manager, plugin_dir);
    }
    Ok(())
}

/// `wire` — scan the plugin directory, wire one port and print the wiring table
/// `wire` — 扫描插件目录、连接一个端口并打印接线表
///
/// # Errors
///
/// Returns an error when the plugin directory is missing, the device ID is not
/// present in the directory, or the port index is out of bounds.
/// 插件目录缺失、设备 ID 不在目录中或端口索引越界时返回错误。
pub fn wire(plugin_dir: &Path, device: &str, port: usize, gpio: u8) -> Result<()> {
    let mut manager = scan_manager(plugin_dir)?;
    if manager.mcp_query_device(device).is_none() {
        let available: Vec<String> = manager
            .mcp_list_devices()
            .iter()
            .filter_map(|entry| entry["id"].as_str().map(str::to_string))
            .collect();
        let available = if available.is_empty() {
            "none / 无".to_string()
        } else {
            available.join(", ")
        };
        bail!(
            "device '{}' not found in {} (available: {}) / 设备未找到",
            device,
            plugin_dir.display(),
            available
        );
    }

    manager.wire(device, port, gpio).with_context(|| {
        format!(
            "failed to wire '{device}' port {port} to GPIO{gpio} / 接线失败: '{device}' 端口 {port} → GPIO{gpio}"
        )
    })?;

    for entry in manager.wiring().all() {
        println!("{}", format_wiring(entry));
    }
    println!(
        "note: devices and wiring live only in this process and are released on exit / \
         提示：设备与接线仅存在于本进程，退出后释放"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Create a fresh temporary directory for plugin-directory tests
    /// 为插件目录测试创建全新的临时目录
    fn temp_dir(tag: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("rp2350-cli-devices-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create temp dir");
        path
    }

    #[test]
    fn device_line_renders_all_fields() {
        let ports = ["LED"];
        let line = DeviceLine {
            id: "com.rp2350-rs.led",
            name: "LED",
            has_ui: false,
            ports: &ports,
        };
        assert_eq!(
            line.render(),
            "com.rp2350-rs.led / LED / has_ui=false / ports=[LED]"
        );
    }

    #[test]
    fn device_line_marks_missing_ports() {
        let line = DeviceLine {
            id: "dev",
            name: "NoPorts",
            has_ui: true,
            ports: &[],
        };
        assert_eq!(line.render(), "dev / NoPorts / has_ui=true / ports=[-]");
    }

    #[test]
    fn wiring_line_renders_entry() {
        let entry = WiringEntry {
            device_id: "com.rp2350-rs.led".to_string(),
            port_name: "LED".to_string(),
            port_index: 0,
            gpio_pin: 25,
        };
        assert_eq!(
            format_wiring(&entry),
            "com.rp2350-rs.led / LED / port 0 / GPIO25"
        );
    }

    #[test]
    fn list_devices_rejects_missing_directory() {
        let dir = temp_dir("list-missing");
        let error = list_devices(&dir.join("absent")).expect_err("missing dir must fail");
        assert!(error.to_string().contains("plugin directory not found"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_devices_accepts_empty_directory() {
        let dir = temp_dir("list-empty");
        list_devices(&dir).expect("empty directory is not an error");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn wire_reports_unknown_device() {
        let dir = temp_dir("wire-unknown");
        let error = wire(&dir, "com.rp2350-rs.led", 0, 25).expect_err("unknown device must fail");
        assert!(error.to_string().contains("not found"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_devices_reload_accepts_empty_directory() {
        let dir = temp_dir("reload-empty");
        list_devices_reload(&dir).expect("empty directory is not an error");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_devices_reload_rejects_missing_directory() {
        let dir = temp_dir("reload-missing");
        let error = list_devices_reload(&dir.join("absent")).expect_err("missing dir must fail");
        assert!(error.to_string().contains("plugin directory not found"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn watch_devices_with_bounded_iterations_returns() {
        let dir = temp_dir("watch-bounded");
        watch_devices_with(&dir, Duration::from_millis(1), Some(2))
            .expect("bounded watch loop must return");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_devices_with_dispatches_on_flags() {
        let dir = temp_dir("dispatch");
        list_devices_with(&dir, false, false).expect("plain listing");
        list_devices_with(&dir, true, false).expect("one reload pass");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn watch_devices_with_zero_iterations_returns_immediately() {
        let dir = temp_dir("watch-zero");
        watch_devices_with(&dir, Duration::from_millis(1), Some(0))
            .expect("zero iterations must not poll");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
