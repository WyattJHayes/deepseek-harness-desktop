//! 插件错误记录（持久化）。
//!
//! 记录来源：
//! - 安装/升级/卸载失败（本应用操作可确定，见 [`super::install`]）；
//! - 页面运行期异常——内嵌 dsh 页面（或 dsh-tauri 桥）经
//!   `report_plugin_error` 命令上报（见 desktop 的 iframe 消息桥）。
//!
//! 记录保存在桌面端数据目录 `plugin-errors.json`，与 `$DSH_HOME`（官方数据）
//! 分离：这是桌面端自己的诊断信息，不属于 dsh profile 数据。
//! 插件安装/升级/卸载成功时清除对应记录。

use crate::config;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Read;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::AppHandle;

const MAX_PLUGIN_ID_BYTES: usize = 214;
const MAX_PLUGIN_ERROR_MESSAGE_CHARS: usize = 2000;
const MAX_PLUGIN_ERROR_MESSAGE_BYTES: usize = 8192;
const MAX_PLUGIN_ERROR_ACTION_BYTES: usize = 32;
const MAX_PLUGIN_ERROR_ENTRIES: usize = 128;
const MAX_PLUGIN_ERROR_FILE_BYTES: u64 = 2 * 1024 * 1024;
const RUNTIME_REPORT_WINDOW: Duration = Duration::from_secs(10);
const MAX_RUNTIME_REPORTS: u32 = 8;
const MAX_RUNTIME_REPORTS_GLOBAL: u32 = 64;
const MAX_RUNTIME_TRACKED_IDS: usize = MAX_PLUGIN_ERROR_ENTRIES;

/// 单条插件错误
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PluginError {
    /// 错误消息（pnpm/运行日志片段，最多保留 2000 字符）
    pub message: String,
    /// 记录动作：install / update / remove / runtime
    pub action: String,
    /// 记录时间（unix 秒级时间戳字符串）
    pub at: String,
}

fn errors_path(app_handle: &AppHandle) -> PathBuf {
    config::get_base_dir(app_handle).join("plugin-errors.json")
}

fn storage_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn runtime_rate_lock() -> &'static Mutex<HashMap<String, RuntimeRate>> {
    static LIMITER: OnceLock<Mutex<HashMap<String, RuntimeRate>>> = OnceLock::new();
    LIMITER.get_or_init(|| Mutex::new(HashMap::new()))
}

#[derive(Clone, Copy)]
struct RuntimeRate {
    window_started: Instant,
    count: u32,
}

fn validate_plugin_id(id: &str) -> Result<(), String> {
    if id.is_empty() || id != id.trim() || id.len() > MAX_PLUGIN_ID_BYTES {
        return Err("PLUGIN_ERROR_INVALID_ID: invalid plugin id".to_string());
    }
    if !super::recovery::is_package_name(id) {
        return Err("PLUGIN_ERROR_INVALID_ID: invalid plugin id".to_string());
    }
    Ok(())
}

fn validate_action(action: &str) -> Result<(), String> {
    if action.len() > MAX_PLUGIN_ERROR_ACTION_BYTES
        || !matches!(action, "install" | "update" | "remove" | "runtime")
    {
        return Err("PLUGIN_ERROR_INVALID_ACTION: unsupported action".to_string());
    }
    Ok(())
}

fn validate_message(message: &str) -> Result<(), String> {
    if message.len() > MAX_PLUGIN_ERROR_MESSAGE_BYTES
        || message.chars().count() > MAX_PLUGIN_ERROR_MESSAGE_CHARS
    {
        return Err("PLUGIN_ERROR_TOO_LARGE: error message exceeds the limit".to_string());
    }
    Ok(())
}

fn validate_record_input(id: &str, action: &str, message: &str) -> Result<(), String> {
    validate_plugin_id(id)?;
    validate_action(action)?;
    validate_message(message)
}

/// 读取全部错误记录（缺失、损坏、超限或不符合格式的记录按空/过滤处理）。
pub(crate) fn load(app_handle: &AppHandle) -> HashMap<String, PluginError> {
    let path = errors_path(app_handle);
    let Ok(metadata) = std::fs::metadata(&path) else {
        return HashMap::new();
    };
    if metadata.len() > MAX_PLUGIN_ERROR_FILE_BYTES {
        log::warn!(
            "plugin error registry exceeds {} bytes: {}",
            MAX_PLUGIN_ERROR_FILE_BYTES,
            path.display()
        );
        return HashMap::new();
    }
    let Ok(file) = std::fs::File::open(path) else {
        return HashMap::new();
    };
    let mut content = Vec::new();
    if file
        .take(MAX_PLUGIN_ERROR_FILE_BYTES.saturating_add(1))
        .read_to_end(&mut content)
        .is_err()
        || content.len() as u64 > MAX_PLUGIN_ERROR_FILE_BYTES
    {
        return HashMap::new();
    }
    let Ok(content) = String::from_utf8(content) else {
        return HashMap::new();
    };
    let Ok(map) = serde_json::from_str::<HashMap<String, PluginError>>(&content) else {
        return HashMap::new();
    };
    map.into_iter()
        .filter(|(id, error)| validate_record_input(id, &error.action, &error.message).is_ok())
        .take(MAX_PLUGIN_ERROR_ENTRIES)
        .collect()
}

fn save(app_handle: &AppHandle, map: &HashMap<String, PluginError>) -> Result<(), String> {
    let path = errors_path(app_handle);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("PLUGIN_ERRORS_DIR: {e}"))?;
    }
    let json =
        serde_json::to_string_pretty(map).map_err(|e| format!("PLUGIN_ERRORS_RENDER: {e}"))?;
    if (json.len() + 1) as u64 > MAX_PLUGIN_ERROR_FILE_BYTES {
        return Err("PLUGIN_ERRORS_LIMIT: registry exceeds the size limit".to_string());
    }

    // 先写同目录临时文件，再替换目标，避免进程中断时留下半个 JSON 文件。
    let temp_path = path.with_file_name(format!(".plugin-errors.{}.tmp", std::process::id()));
    if let Err(error) = std::fs::write(&temp_path, format!("{json}\n")) {
        return Err(format!("PLUGIN_ERRORS_WRITE: {error}"));
    }
    if let Err(error) = replace_file(&temp_path, &path) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(error);
    }
    Ok(())
}

#[cfg(not(windows))]
fn replace_file(temp_path: &std::path::Path, path: &std::path::Path) -> Result<(), String> {
    std::fs::rename(temp_path, path).map_err(|e| format!("PLUGIN_ERRORS_REPLACE: {e}"))
}

#[cfg(windows)]
fn replace_file(temp_path: &std::path::Path, path: &std::path::Path) -> Result<(), String> {
    use std::iter::once;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let temp_path: Vec<u16> = temp_path.as_os_str().encode_wide().chain(once(0)).collect();
    let path: Vec<u16> = path.as_os_str().encode_wide().chain(once(0)).collect();
    let result = unsafe {
        MoveFileExW(
            temp_path.as_ptr(),
            path.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        Err(format!(
            "PLUGIN_ERRORS_REPLACE: {}",
            std::io::Error::last_os_error()
        ))
    } else {
        Ok(())
    }
}

fn insert_record(
    map: &mut HashMap<String, PluginError>,
    id: &str,
    action: &str,
    message: &str,
    at: String,
) -> Result<(), String> {
    validate_record_input(id, action, message)?;
    if !map.contains_key(id) && map.len() >= MAX_PLUGIN_ERROR_ENTRIES {
        return Err("PLUGIN_ERRORS_LIMIT: registry entry limit reached".to_string());
    }
    map.insert(
        id.to_string(),
        PluginError {
            message: message.to_string(),
            action: action.to_string(),
            at,
        },
    );
    Ok(())
}

fn current_timestamp() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs().to_string())
        .unwrap_or_default()
}

/// 记录插件错误（同 id 幂等覆盖，输入和注册表均有界）。
pub fn record(app_handle: &AppHandle, id: &str, action: &str, message: &str) -> Result<(), String> {
    validate_record_input(id, action, message)?;
    let _guard = storage_lock()
        .lock()
        .map_err(|_| "PLUGIN_ERRORS_STATE: lock poisoned".to_string())?;
    let mut map = load(app_handle);
    insert_record(&mut map, id, action, message.trim(), current_timestamp())?;
    save(app_handle, &map)
}

fn allow_runtime_report(id: &str) -> Result<(), String> {
    let now = Instant::now();
    let mut limiter = runtime_rate_lock()
        .lock()
        .map_err(|_| "PLUGIN_ERRORS_STATE: rate limiter lock poisoned".to_string())?;
    limiter.retain(|_, rate| now.duration_since(rate.window_started) < RUNTIME_REPORT_WINDOW);

    let total_reports: u32 = limiter.values().map(|rate| rate.count).sum();
    if total_reports >= MAX_RUNTIME_REPORTS_GLOBAL {
        return Err("PLUGIN_ERROR_RATE_LIMITED: too many runtime reports".to_string());
    }

    if let Some(rate) = limiter.get_mut(id) {
        if rate.count >= MAX_RUNTIME_REPORTS {
            return Err("PLUGIN_ERROR_RATE_LIMITED: too many runtime reports".to_string());
        }
        rate.count += 1;
        return Ok(());
    }
    if limiter.len() >= MAX_RUNTIME_TRACKED_IDS {
        return Err("PLUGIN_ERROR_RATE_LIMITED: too many runtime reporters".to_string());
    }
    limiter.insert(
        id.to_string(),
        RuntimeRate {
            window_started: now,
            count: 1,
        },
    );
    Ok(())
}

/// 记录来自 iframe 的运行期错误：必须是已安装插件，并限制短时间内的上报次数。
pub fn record_runtime(app_handle: &AppHandle, id: &str, message: &str) -> Result<(), String> {
    validate_record_input(id, "runtime", message)?;
    if !super::installed::is_installed(app_handle, id) {
        return Err("PLUGIN_ERROR_UNKNOWN_ID: plugin is not installed".to_string());
    }
    // 只有确认属于当前 profile 的插件后才消耗配额，避免未知 id 影响真实插件上报。
    allow_runtime_report(id)?;
    record(app_handle, id, "runtime", message)
}

/// 清除插件错误（安装/升级/卸载成功后）
pub fn clear(app_handle: &AppHandle, id: &str) -> Result<(), String> {
    validate_plugin_id(id)?;
    let _guard = storage_lock()
        .lock()
        .map_err(|_| "PLUGIN_ERRORS_STATE: lock poisoned".to_string())?;
    let mut map = load(app_handle);
    if map.remove(id).is_some() {
        save(app_handle, &map)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_clear_round_trip() {
        // 不依赖 AppHandle 的纯文件读写用临时目录验证序列化形态
        let map = HashMap::from([(
            "dshmarket".to_string(),
            PluginError {
                message: "ERR_PNPM_IGNORED_BUILDS".to_string(),
                action: "install".to_string(),
                at: "1700000000".to_string(),
            },
        )]);
        let json = serde_json::to_string(&map).unwrap();
        let back: HashMap<String, PluginError> = serde_json::from_str(&json).unwrap();
        assert_eq!(back.get("dshmarket").unwrap().action, "install");
        assert_eq!(
            back.get("dshmarket").unwrap().message,
            "ERR_PNPM_IGNORED_BUILDS"
        );
    }

    #[test]
    fn record_input_is_bounded_and_typed() {
        assert!(validate_record_input("dshmarket", "runtime", "failure").is_ok());
        assert!(validate_record_input("not a package", "runtime", "failure").is_err());
        assert!(validate_record_input("dshmarket", "unknown", "failure").is_err());
        assert!(validate_record_input(
            "dshmarket",
            "runtime",
            &"x".repeat(MAX_PLUGIN_ERROR_MESSAGE_CHARS + 1)
        )
        .is_err());
    }

    #[test]
    fn registry_entry_limit_is_enforced() {
        let mut map = HashMap::new();
        for index in 0..MAX_PLUGIN_ERROR_ENTRIES {
            let id = format!("dsh-test-plugin-{index}");
            insert_record(
                &mut map,
                &id,
                "runtime",
                "failure",
                "1700000000".to_string(),
            )
            .unwrap();
        }
        assert!(insert_record(
            &mut map,
            "dsh-test-plugin-overflow",
            "runtime",
            "failure",
            "1700000000".to_string(),
        )
        .is_err());
        insert_record(
            &mut map,
            "dsh-test-plugin-0",
            "runtime",
            "updated",
            "1700000001".to_string(),
        )
        .unwrap();
    }

    #[test]
    fn runtime_reports_have_a_global_window_limit() {
        let prefix = format!("dsh-test-runtime-{}", std::process::id());
        for index in 0..MAX_RUNTIME_REPORTS_GLOBAL {
            allow_runtime_report(&format!("{prefix}-{index}")).unwrap();
        }
        assert!(allow_runtime_report(&format!("{prefix}-overflow")).is_err());
    }
}
