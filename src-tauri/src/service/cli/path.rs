//! 用户 PATH 注册与路径计算：bin 目录定位、Windows 注册表读写与
//! `WM_SETTINGCHANGE` 广播、Unix shell rc 幂等块更新（备份 + 失败回滚），以及用户 pnpm 探测。

#[cfg(not(windows))]
use std::fs;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

#[cfg(windows)]
use super::shim::SHIM_CMD_NAME;
#[cfg(unix)]
use super::shim::SHIM_SH_NAME;

/// Windows 下 shim 根目录名（`%LOCALAPPDATA%\<此目录>\bin`）
#[cfg(windows)]
const CLI_ROOT_DIR_NAME: &str = "deepseek-harness";

/// Unix 下 shim 所在目录（XDG 约定）
#[cfg(unix)]
const UNIX_BIN_DIR: &str = ".local/bin";

/// shell rc 注入标记（用于幂等增删；Windows 无 rc 逻辑，仅测试引用）
#[cfg_attr(windows, allow(dead_code))]
const RC_MARK_START: &str = "# >>> deepseek-harness dsh >>>";
#[cfg_attr(windows, allow(dead_code))]
const RC_MARK_END: &str = "# <<< deepseek-harness dsh <<<";

/// Unix 下需要写入 PATH 导出的 rc 文件（按顺序处理；同上，Windows 仅测试引用）
#[cfg_attr(windows, allow(dead_code))]
const RC_FILES: [&str; 2] = [".zshrc", ".bashrc"];

// ---------------------------------------------------------------------------
// 路径计算
// ---------------------------------------------------------------------------

/// bin 目录：
/// - Windows：`%LOCALAPPDATA%\deepseek-harness\bin`（用户级、不随应用数据目录变动）
/// - Unix：`~/.local/bin`（XDG 约定，通常已在 PATH 中）
pub fn get_bin_dir(app_handle: &AppHandle) -> PathBuf {
    #[cfg(windows)]
    {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .or_else(|| {
                app_handle
                    .path()
                    .local_data_dir()
                    .ok()
                    .and_then(|d| d.parent().map(|p| p.to_path_buf()))
            })
            .unwrap_or_else(std::env::temp_dir)
            .join(CLI_ROOT_DIR_NAME)
            .join("bin")
    }
    #[cfg(not(windows))]
    {
        app_handle
            .path()
            .home_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(UNIX_BIN_DIR)
    }
}

/// 主 shim 文件路径（状态展示用）
pub fn get_shim_path(app_handle: &AppHandle) -> PathBuf {
    let bin_dir = get_bin_dir(app_handle);
    #[cfg(windows)]
    {
        bin_dir.join(SHIM_CMD_NAME)
    }
    #[cfg(not(windows))]
    {
        bin_dir.join(SHIM_SH_NAME)
    }
}

/// 当前用户 PATH 中是否已包含 bin 目录（Windows 以注册表为准，
/// 因为进程内 PATH 在广播 WM_SETTINGCHANGE 后不会自动更新）
pub fn path_registered(app_handle: &AppHandle) -> bool {
    #[cfg(windows)]
    {
        let bin_dir = get_bin_dir(app_handle);
        let Some(bin_str) = bin_dir.to_str() else {
            return false;
        };
        read_user_path()
            .map(|value| path_contains_token(&value, bin_str))
            .unwrap_or(false)
    }
    #[cfg(not(windows))]
    {
        let bin_dir = get_bin_dir(app_handle);
        // 1. 当前进程 PATH 已包含（新终端直接可用）
        if std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
            .any(|p| p == bin_dir)
        {
            return true;
        }
        // 2. rc 文件中已注入标记块（重启 shell 后可用）
        let home = app_handle
            .path()
            .home_dir()
            .unwrap_or_else(|_| PathBuf::from("."));
        RC_FILES.iter().any(|name| {
            fs::read_to_string(home.join(name))
                .map(|content| content.contains(RC_MARK_START))
                .unwrap_or(false)
        })
    }
}

/// 在 PATH 中查找用户自己安装的 pnpm（排除应用注册的 shim 目录）。
///
/// "用户优先"策略：安装时（`Pnpm::check_installed`）用户已有 pnpm 则跳过
/// 捆绑安装；`pnpm` shim 运行时也会优先转发到用户的 pnpm。
pub fn find_user_pnpm(app_handle: &AppHandle) -> Option<PathBuf> {
    let dirs: Vec<PathBuf> =
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()).collect();
    #[cfg(windows)]
    let dirs = {
        let mut dirs = dirs;
        append_windows_pnpm_dirs(&mut dirs);
        dirs
    };
    find_pnpm_in_dirs(&get_bin_dir(app_handle), &dirs)
}

/// 在给定目录中按顺序查找用户 pnpm，便于不依赖全局环境测试探测规则。
fn find_pnpm_in_dirs(bin_dir: &Path, dirs: &[PathBuf]) -> Option<PathBuf> {
    let candidates: &[&str] = if cfg!(windows) {
        &["pnpm.cmd", "pnpm.exe", "pnpm.bat"]
    } else {
        &["pnpm"]
    };
    let bin_dir = dunce::canonicalize(bin_dir).unwrap_or_else(|_| bin_dir.to_path_buf());
    for dir in dirs {
        if dir.as_os_str().is_empty() {
            continue;
        }
        let normalized = dunce::canonicalize(dir).unwrap_or_else(|_| dir.clone());
        if normalized == bin_dir {
            continue;
        }
        for name in candidates {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// Windows GUI 进程的 PATH 可能早于 npm/pnpm 安装，补充这些工具的标准用户目录。
#[cfg(windows)]
fn append_windows_pnpm_dirs(dirs: &mut Vec<PathBuf>) {
    let mut append = |dir: PathBuf| {
        if dir.is_absolute() && !dirs.iter().any(|existing| existing == &dir) {
            dirs.push(dir);
        }
    };
    if let Some(appdata) = std::env::var_os("APPDATA") {
        append(PathBuf::from(appdata).join("npm"));
    }
    for variable in ["PNPM_HOME", "NPM_CONFIG_PREFIX", "npm_config_prefix"] {
        if let Some(value) = std::env::var_os(variable) {
            append(PathBuf::from(value));
        }
    }
    if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
        append(PathBuf::from(local_app_data).join("pnpm"));
    }
}

// ---------------------------------------------------------------------------
// PATH 注册 / 注销（Windows：注册表 + WM_SETTINGCHANGE；Unix：shell rc）
// ---------------------------------------------------------------------------

/// 注册 bin 目录到用户 PATH（幂等）
pub fn register_path(app_handle: &AppHandle) -> Result<(), String> {
    if path_registered(app_handle) {
        return Ok(());
    }
    #[cfg(windows)]
    {
        let bin_dir = get_bin_dir(app_handle);
        let bin_str = bin_dir
            .to_str()
            .ok_or_else(|| "bin dir is not valid UTF-8".to_string())?;
        let current = read_user_path().unwrap_or_default();
        let new_value = if current.trim().is_empty() {
            bin_str.to_string()
        } else {
            format!("{};{}", current.trim_end_matches(';'), bin_str)
        };
        write_user_path(&new_value)?;
        notify_environment_change();
        log::info!("Registered dsh bin dir in user PATH: {bin_str}");
    }
    #[cfg(not(windows))]
    {
        inject_shell_rc(app_handle)?;
    }
    Ok(())
}

/// 从用户 PATH 中移除 bin 目录（幂等）
pub fn unregister_path(app_handle: &AppHandle) -> Result<(), String> {
    #[cfg(windows)]
    {
        let bin_dir = get_bin_dir(app_handle);
        let Some(bin_str) = bin_dir.to_str() else {
            return Ok(());
        };
        if let Some(current) = read_user_path() {
            if !path_contains_token(&current, bin_str) {
                return Ok(());
            }
            let new_value = remove_path_token(&current, bin_str);
            write_user_path(&new_value)?;
            notify_environment_change();
            log::info!("Removed dsh bin dir from user PATH");
        }
    }
    #[cfg(not(windows))]
    {
        strip_shell_rc(app_handle)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Windows 注册表辅助
// ---------------------------------------------------------------------------

#[cfg(windows)]
#[inline]
fn to_wide_null(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
fn read_user_path() -> Option<String> {
    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_MORE_DATA};
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE,
    };

    unsafe {
        let mut hkey: HKEY = std::ptr::null_mut();
        let key_name = to_wide_null("Environment");
        let ret = RegOpenKeyExW(
            HKEY_CURRENT_USER,
            key_name.as_ptr(),
            0,
            KEY_QUERY_VALUE,
            &mut hkey,
        );
        if ret != 0 {
            log::warn!("failed to open HKCU\\Environment (error {ret})");
            return None;
        }

        let value_name = to_wide_null("Path");
        let mut value_type: u32 = 0;
        let mut size: u32 = 0;
        let mut ret = RegQueryValueExW(
            hkey,
            value_name.as_ptr(),
            std::ptr::null(),
            &mut value_type,
            std::ptr::null_mut(),
            &mut size,
        );

        if ret == ERROR_FILE_NOT_FOUND {
            RegCloseKey(hkey);
            return Some(String::new());
        }
        if ret != ERROR_MORE_DATA && ret != 0 {
            RegCloseKey(hkey);
            log::warn!("failed to query HKCU\\Environment\\Path (error {ret})");
            return None;
        }

        let mut buf = vec![0u16; (size as usize / 2).max(1) + 1];
        ret = RegQueryValueExW(
            hkey,
            value_name.as_ptr(),
            std::ptr::null(),
            &mut value_type,
            buf.as_mut_ptr() as *mut u8,
            &mut size,
        );
        RegCloseKey(hkey);

        if ret != 0 {
            log::warn!("failed to read HKCU\\Environment\\Path (error {ret})");
            return None;
        }
        let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Some(String::from_utf16_lossy(&buf[..end]))
    }
}

#[cfg(windows)]
fn write_user_path(new_value: &str) -> Result<(), String> {
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER,
        KEY_QUERY_VALUE, KEY_SET_VALUE, REG_EXPAND_SZ, REG_SZ,
    };

    unsafe {
        let mut hkey: HKEY = std::ptr::null_mut();
        let key_name = to_wide_null("Environment");
        let ret = RegOpenKeyExW(
            HKEY_CURRENT_USER,
            key_name.as_ptr(),
            0,
            KEY_QUERY_VALUE | KEY_SET_VALUE,
            &mut hkey,
        );
        if ret != 0 {
            return Err(format!("failed to open HKCU\\Environment (error {ret})"));
        }

        let value_name = to_wide_null("Path");
        let mut value_type: u32 = REG_EXPAND_SZ;
        let mut size: u32 = 0;
        RegQueryValueExW(
            hkey,
            value_name.as_ptr(),
            std::ptr::null(),
            &mut value_type,
            std::ptr::null_mut(),
            &mut size,
        );
        if value_type != REG_SZ && value_type != REG_EXPAND_SZ {
            value_type = REG_EXPAND_SZ;
        }

        let wide_value = to_wide_null(new_value);
        let bytes = (wide_value.len() * 2) as u32;
        let ret = RegSetValueExW(
            hkey,
            value_name.as_ptr(),
            0,
            value_type,
            wide_value.as_ptr() as *const u8,
            bytes,
        );
        RegCloseKey(hkey);

        if ret != 0 {
            return Err(format!(
                "failed to write HKCU\\Environment\\Path (error {ret})"
            ));
        }
        Ok(())
    }
}

#[cfg(windows)]
fn notify_environment_change() {
    use windows_sys::Win32::Foundation::{LPARAM, WPARAM};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SendMessageTimeoutW, HWND_BROADCAST, SMTO_ABORTIFHUNG, WM_SETTINGCHANGE,
    };
    let wide = to_wide_null("Environment");
    unsafe {
        SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            0 as WPARAM,
            wide.as_ptr() as LPARAM,
            SMTO_ABORTIFHUNG,
            5000,
            std::ptr::null_mut(),
        );
    }
}

/// 展开字符串中的 `%VAR%`（Windows）
#[cfg(windows)]
fn expand_env(value: &str) -> String {
    use windows_sys::Win32::System::Environment::ExpandEnvironmentStringsW;
    let wide = to_wide_null(value);
    let mut buf = vec![0u16; 32768];
    let n = unsafe { ExpandEnvironmentStringsW(wide.as_ptr(), buf.as_mut_ptr(), buf.len() as u32) };
    if n == 0 || n > buf.len() as u32 {
        return value.to_string();
    }
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

/// PATH 值（`;` 分隔）中是否已包含指定目录（大小写不敏感，先展开 %VAR%）
#[cfg(windows)]
fn path_contains_token(path_value: &str, token: &str) -> bool {
    let expanded = expand_env(path_value);
    let token_lower = token.to_lowercase();
    expanded
        .split(';')
        .any(|p| !p.is_empty() && p.trim_end_matches('\\').to_lowercase() == token_lower)
}

/// 从 PATH 值中移除指定目录 token（同时处理 `%LOCALAPPDATA%` 未展开形式）
#[cfg(windows)]
fn remove_path_token(path_value: &str, token: &str) -> String {
    let token_lower = token.to_lowercase();
    let unexpanded_lower = token_lower.replace(
        &std::env::var("LOCALAPPDATA")
            .unwrap_or_default()
            .to_lowercase(),
        "%localappdata%",
    );
    let kept: Vec<&str> = path_value
        .split(';')
        .filter(|p| {
            if p.is_empty() {
                return false;
            }
            let norm = p.trim_end_matches('\\').to_lowercase();
            norm != token_lower && norm != unexpanded_lower
        })
        .collect();
    kept.join(";")
}

// ---------------------------------------------------------------------------
// Unix shell rc 辅助
// ---------------------------------------------------------------------------

/// Unix：向 `~/.zshrc` / `~/.bashrc` 幂等注入 `~/.local/bin` 的 PATH 导出。
///
/// 只更新自身标记块：读取原文件 → 移除旧块 → 末尾追加新块；仅当文件不存在时
/// 才新建。读失败（非"不存在"）直接报错退出，绝不把"读不到"当作空文件去
/// 全量覆盖用户配置；写入前先备份，写失败自动回滚（见 `write_rc_with_backup`）。
#[cfg(not(windows))]
fn inject_shell_rc(app_handle: &AppHandle) -> Result<(), String> {
    let home = app_handle
        .path()
        .home_dir()
        .map_err(|_| "failed to resolve home directory".to_string())?;
    let block = format!("{RC_MARK_START}\nexport PATH=\"$HOME/.local/bin:$PATH\"\n{RC_MARK_END}\n");

    for name in RC_FILES {
        let rc_path = home.join(name);
        let original = match fs::read_to_string(&rc_path) {
            Ok(content) => content,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => {
                return Err(format!(
                    "READ_RC_FAILED: read {} failed: {e}",
                    rc_path.display()
                ))
            }
        };
        let next = upsert_rc_block(&original, &block);
        if next == original {
            continue;
        }
        write_rc_with_backup(&rc_path, &next)?;
        log::info!("Injected PATH export into {}", rc_path.display());
    }
    Ok(())
}

/// Unix：从 rc 文件中移除注入块（保留用户其余配置，同样走备份 + 回滚写入）
#[cfg(not(windows))]
fn strip_shell_rc(app_handle: &AppHandle) -> Result<(), String> {
    let home = app_handle
        .path()
        .home_dir()
        .map_err(|_| "failed to resolve home directory".to_string())?;
    for name in RC_FILES {
        let rc_path = home.join(name);
        let original = match fs::read_to_string(&rc_path) {
            Ok(content) => content,
            // 文件不存在则无需清理
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                return Err(format!(
                    "READ_RC_FAILED: read {} failed: {e}",
                    rc_path.display()
                ))
            }
        };
        let cleaned = strip_rc_block(&original);
        if cleaned != original {
            write_rc_with_backup(&rc_path, &cleaned)?;
            log::info!("Removed PATH export from {}", rc_path.display());
        }
    }
    Ok(())
}

/// 将 PATH 导出块并入 rc 内容：先移除已有标记块，再在文件末尾追加新块，
/// 只更新自身块、保留用户其余配置，且块始终落在文件末尾。
#[cfg_attr(windows, allow(dead_code))]
fn upsert_rc_block(content: &str, block: &str) -> String {
    let mut out = strip_rc_block(content);
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(block);
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// 原子写回 rc 文件：写入前先备份为 `<file>.dsh-backup`，再通过同目录
/// 临时文件 + rename 原子替换；写失败时删除临时文件并回滚备份内容，
/// 保证任何异常路径下用户原文件都不会被半写/被清空。
#[cfg_attr(windows, allow(dead_code))]
fn write_rc_with_backup(rc_path: &std::path::Path, new_content: &str) -> Result<(), String> {
    use std::fs;

    let backup_path = rc_path.with_extension("dsh-backup");
    let had_original = rc_path.exists();
    if had_original {
        fs::copy(rc_path, &backup_path).map_err(|e| {
            format!(
                "BACKUP_RC_FAILED: backup {} to {} failed: {e}",
                rc_path.display(),
                backup_path.display()
            )
        })?;
    }

    let tmp_path = rc_path.with_extension("dsh-rc-tmp");
    fs::write(&tmp_path, new_content)
        .map_err(|e| format!("WRITE_RC_FAILED: write {} failed: {e}", tmp_path.display()))?;
    let rename_res = match fs::rename(&tmp_path, rc_path) {
        Ok(()) => Ok(()),
        // Windows 下 rename 不覆盖已存在目标（仅测试环境会走到）：删旧文件后重试
        Err(_) => {
            let _ = fs::remove_file(rc_path);
            fs::rename(&tmp_path, rc_path)
        }
    };
    if let Err(e) = rename_res {
        let _ = fs::remove_file(&tmp_path);
        if had_original {
            let _ = fs::copy(&backup_path, rc_path);
        }
        return Err(format!(
            "RENAME_RC_FAILED: rename into {} failed: {e}",
            rc_path.display()
        ));
    }
    Ok(())
}

/// 移除 rc 文件中的标记块（含标记行本身）。
/// 同时被注入（`upsert_rc_block`）与移除路径使用；Windows 仅测试引用。
#[cfg_attr(windows, allow(dead_code))]
fn strip_rc_block(content: &str) -> String {
    let mut lines = content.lines().peekable();
    let mut out = String::with_capacity(content.len());
    let mut skipping = false;
    while let Some(line) = lines.next() {
        if line.trim() == RC_MARK_START {
            skipping = true;
            continue;
        }
        if skipping {
            if line.trim() == RC_MARK_END {
                skipping = false;
            }
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const RC_BLOCK: &str =
        "# >>> deepseek-harness dsh >>>\nexport PATH=\"$HOME/.local/bin:$PATH\"\n# <<< deepseek-harness dsh <<<\n";

    /// 独立的临时目录，避免测试间互相干扰
    #[test]
    fn find_pnpm_prefers_path_order_and_skips_shim_dir() {
        let root = temp_dir("pnpm-discovery");
        let shim_dir = root.join("shim");
        let first = root.join("first");
        let second = root.join("second");
        std::fs::create_dir_all(&shim_dir).unwrap();
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        let name = if cfg!(windows) { "pnpm.cmd" } else { "pnpm" };
        std::fs::write(shim_dir.join(name), "shim").unwrap();
        std::fs::write(first.join(name), "first").unwrap();
        std::fs::write(second.join(name), "second").unwrap();
        let found = find_pnpm_in_dirs(&shim_dir, &[shim_dir.clone(), first.clone(), second]);
        assert_eq!(found, Some(first.join(name)));
        let _ = std::fs::remove_dir_all(root);
    }

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "dsh-rc-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// issue #57：目标 rc 文件已存在且包含用户自定义内容 → 只追加块，原内容保留
    #[test]
    fn upsert_keeps_user_content_and_appends_block() {
        let content = "# oh-my-zsh\nplugins=(git)\nalias ll='ls -alF'\n";
        let next = upsert_rc_block(content, RC_BLOCK);
        assert!(next.starts_with(content));
        assert!(next.ends_with(RC_BLOCK));
        assert_eq!(next.matches(RC_MARK_START).count(), 1);
    }

    /// issue #57：旧块位于文件中间时 → 移除并移动到末尾，周围用户内容保留
    #[test]
    fn upsert_moves_stale_block_to_end() {
        let stale = format!("alias ll='ls -alF'\n{RC_BLOCK}export NVM_DIR=\"$HOME/.nvm\"\n");
        let next = upsert_rc_block(&stale, RC_BLOCK);
        let stripped = strip_rc_block(&next);
        assert_eq!(
            stripped,
            "alias ll='ls -alF'\nexport NVM_DIR=\"$HOME/.nvm\"\n"
        );
        assert!(next.ends_with(RC_BLOCK));
        assert_eq!(next.matches(RC_MARK_START).count(), 1);
    }

    /// 幂等：重复注入不产生第二块
    #[test]
    fn upsert_is_idempotent() {
        let content = "user content\n";
        let once = upsert_rc_block(content, RC_BLOCK);
        assert_eq!(upsert_rc_block(&once, RC_BLOCK), once);
    }

    /// 空内容（文件不存在时的新建场景）→ 仅注入块，没有多余空行
    #[test]
    fn upsert_from_missing_file_creates_block_only() {
        assert_eq!(upsert_rc_block("", RC_BLOCK), RC_BLOCK.to_string());
    }

    /// 无末尾换行的内容 → 补换行后再追加块
    #[test]
    fn upsert_handles_missing_trailing_newline() {
        let next = upsert_rc_block("no trailing nl", RC_BLOCK);
        assert_eq!(next, "no trailing nl\n".to_string() + RC_BLOCK);
    }

    /// strip 原语：移除标记块且幂等
    #[test]
    fn strip_rc_block_removes_block_and_is_idempotent() {
        let content = format!("keep\n{RC_BLOCK}tail\n");
        let cleaned = strip_rc_block(&content);
        assert_eq!(cleaned, "keep\ntail\n");
        assert_eq!(strip_rc_block(&cleaned), cleaned);
    }

    /// 写回：备份保留原内容、目标被替换为 new_content
    #[test]
    fn write_rc_with_backup_preserves_backup() {
        let dir = temp_dir("backup");
        let rc_path = dir.join(".zshrc");
        std::fs::write(&rc_path, "original\n").unwrap();

        write_rc_with_backup(&rc_path, "original\n# block\n").unwrap();

        assert_eq!(
            std::fs::read_to_string(&rc_path).unwrap(),
            "original\n# block\n"
        );
        assert_eq!(
            std::fs::read_to_string(rc_path.with_extension("dsh-backup")).unwrap(),
            "original\n"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 写回：文件原本不存在 → 新建成功且不产生备份
    #[test]
    fn write_rc_with_backup_creates_missing_file() {
        let dir = temp_dir("create");
        let rc_path = dir.join(".bashrc");
        write_rc_with_backup(&rc_path, "# block\n").unwrap();
        assert_eq!(std::fs::read_to_string(&rc_path).unwrap(), "# block\n");
        assert!(!rc_path.with_extension("dsh-backup").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
