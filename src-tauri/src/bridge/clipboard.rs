//! 原生剪贴板图片读取（Linux/WebKitGTK 贴图回退）。
//!
//! WebKitGTK 不通过 Web API（`ClipboardEvent.clipboardData.items/files`）暴露
//! `image/*` 剪贴板条目，导致桌面端内嵌的 dsh iframe 中输入框「贴图」无效
//! （浏览器里却正常）。本命令在 Rust 侧用 `arboard` 读取系统剪贴板图片、编码为
//! PNG data URL 返回；注入到 iframe 的桥脚本（`desktop::paste::paste_shim_js`）
//! 拿到该 data URL 后重新派发 `paste` 事件，让 dsh 聊天框按正常贴图路径处理。

use base64::{engine::general_purpose::STANDARD, Engine};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::Manager;

type HmacSha256 = Hmac<Sha256>;

const BRIDGE_SECRET_BYTES: usize = 32;
const NONCE_BYTES: usize = 16;
const NONCE_HEX_LEN: usize = NONCE_BYTES * 2;
const PROOF_BYTES: usize = 32;
const NONCE_TTL: Duration = Duration::from_secs(5);
const MAX_USED_NONCES: usize = 256;
const SIGNING_CONTEXT: &[u8] = b"dsh-clipboard-image-v1:";

/// 剪贴板桥的会话认证状态。
///
/// 密钥由宿主进程生成，并只通过初始化脚本交给受控闭包；注入脚本通过 HMAC
/// 证明请求来自受控粘贴流程。nonce 在 Rust 侧只允许消费一次，避免脚本截获合法
/// 请求后重复读取剪贴板。
pub(crate) struct ClipboardBridgeState {
    secret: [u8; BRIDGE_SECRET_BYTES],
    used_nonces: Mutex<HashMap<[u8; NONCE_BYTES], Instant>>,
}

impl ClipboardBridgeState {
    /// 使用操作系统随机源创建一次会话密钥；随机源不可用时拒绝启动桥接能力。
    pub(crate) fn new() -> Result<Self, String> {
        let mut secret = [0u8; BRIDGE_SECRET_BYTES];
        getrandom::fill(&mut secret)
            .map_err(|e| format!("CLIPBOARD_BRIDGE_INIT: secure random unavailable: {e}"))?;
        Ok(Self {
            secret,
            used_nonces: Mutex::new(HashMap::new()),
        })
    }

    /// 返回注入脚本使用的会话密钥编码，不暴露给 iframe 页面脚本的全局作用域。
    pub(crate) fn script_secret(&self) -> String {
        STANDARD.encode(self.secret)
    }

    /// 校验并消费一次剪贴板请求凭证。
    pub(crate) fn verify_and_consume(
        &self,
        nonce: &str,
        issued_at: u64,
        proof: &str,
    ) -> Result<(), String> {
        let nonce_bytes = decode_nonce(nonce)?;
        let proof_bytes = STANDARD
            .decode(proof)
            .map_err(|_| "CLIPBOARD_AUTH_INVALID: malformed proof".to_string())?;
        if proof_bytes.len() != PROOF_BYTES {
            return Err("CLIPBOARD_AUTH_INVALID: malformed proof".to_string());
        }

        let expected = self.expected_proof(nonce, issued_at);
        if !constant_time_equal(&expected, &proof_bytes) {
            return Err("CLIPBOARD_AUTH_INVALID: invalid proof".to_string());
        }
        if !timestamp_is_fresh(issued_at) {
            return Err("CLIPBOARD_NONCE_EXPIRED: request timestamp is stale".to_string());
        }

        let now = Instant::now();
        let mut used_nonces = self
            .used_nonces
            .lock()
            .map_err(|_| "CLIPBOARD_AUTH_STATE: lock poisoned".to_string())?;
        used_nonces.retain(|_, used_at| now.duration_since(*used_at) <= NONCE_TTL);
        if used_nonces.contains_key(&nonce_bytes) {
            return Err("CLIPBOARD_NONCE_REPLAY: request already consumed".to_string());
        }
        if used_nonces.len() >= MAX_USED_NONCES {
            return Err("CLIPBOARD_RATE_LIMITED: too many pending requests".to_string());
        }
        used_nonces.insert(nonce_bytes, now);
        Ok(())
    }

    fn expected_proof(&self, nonce: &str, issued_at: u64) -> [u8; PROOF_BYTES] {
        let mut mac = HmacSha256::new_from_slice(&self.secret).expect("fixed-size HMAC key");
        mac.update(SIGNING_CONTEXT);
        mac.update(nonce.as_bytes());
        mac.update(b":");
        mac.update(issued_at.to_string().as_bytes());
        let digest = mac.finalize().into_bytes();
        let mut proof = [0u8; PROOF_BYTES];
        proof.copy_from_slice(&digest);
        proof
    }
}

fn timestamp_is_fresh(issued_at: u64) -> bool {
    let Ok(now) = SystemTime::now().duration_since(UNIX_EPOCH) else {
        return false;
    };
    let now_millis = now.as_millis();
    let issued_at = u128::from(issued_at);
    if issued_at > now_millis {
        return false;
    }
    now_millis - issued_at <= NONCE_TTL.as_millis()
}

fn decode_nonce(value: &str) -> Result<[u8; NONCE_BYTES], String> {
    if value.len() != NONCE_HEX_LEN {
        return Err("CLIPBOARD_NONCE_INVALID: malformed nonce".to_string());
    }
    let bytes = value.as_bytes();
    let mut nonce = [0u8; NONCE_BYTES];
    for index in 0..NONCE_BYTES {
        let high = hex_value(bytes[index * 2])
            .ok_or_else(|| "CLIPBOARD_NONCE_INVALID: malformed nonce".to_string())?;
        let low = hex_value(bytes[index * 2 + 1])
            .ok_or_else(|| "CLIPBOARD_NONCE_INVALID: malformed nonce".to_string())?;
        nonce[index] = (high << 4) | low;
    }
    Ok(nonce)
}

fn hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn constant_time_equal(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut difference = 0u8;
    for (left, right) in left.iter().zip(right) {
        difference |= left ^ right;
    }
    difference == 0
}

/// 剪贴板图片读取结果：自包含的 PNG data URL（可直接作为 Blob/File 来源）。
#[derive(serde::Serialize)]
pub struct ClipboardImageResponse {
    /// 形如 `data:image/png;base64,...`
    pub data_url: String,
    pub mime: String,
    pub filename: String,
}

/// 从系统剪贴板读取图片并编码为 PNG data URL。
///
/// 剪贴板无图片时返回 `Ok(None)`；读取/编码失败返回 `Err`（前缀 `CLIPBOARD_IMAGE_`）。
#[tauri::command]
pub async fn read_clipboard_image(
    app: tauri::AppHandle,
    nonce: String,
    issued_at: u64,
    proof: String,
) -> Result<Option<ClipboardImageResponse>, String> {
    app.state::<ClipboardBridgeState>()
        .verify_and_consume(&nonce, issued_at, &proof)?;

    // arboard 的 Clipboard::new()/get_image() 是阻塞调用（Linux 上需连接显示服务器），
    // 放到 blocking 线程避免阻塞异步运行时与 UI。
    let result =
        tokio::task::spawn_blocking(move || -> Result<Option<ClipboardImageResponse>, String> {
            // 超过约 50MP 的剪贴板图片（≈200MB RGBA）直接拒绝，避免撑爆内存
            const MAX_PIXELS: u64 = 50_000_000;

            let mut clipboard =
                arboard::Clipboard::new().map_err(|e| format!("CLIPBOARD_IMAGE_ACCESS: {e}"))?;

            let image_data = match clipboard.get_image() {
                Ok(data) => data,
                // 剪贴板里没有图片（普通文本/文件等），不是错误
                Err(arboard::Error::ContentNotAvailable) => return Ok(None),
                Err(e) => return Err(format!("CLIPBOARD_IMAGE_READ: {e}")),
            };

            if image_data.width == 0 || image_data.height == 0 {
                return Ok(None);
            }
            let pixel_count = image_data.width as u64 * image_data.height as u64;
            if pixel_count > MAX_PIXELS {
                return Err(format!(
                    "CLIPBOARD_IMAGE_TOO_LARGE: {}x{} ({} px)",
                    image_data.width, image_data.height, pixel_count
                ));
            }

            // arboard 返回 RGBA8 像素，直接包装为 RgbaImage 后用 image 编码成 PNG
            let rgba = image::RgbaImage::from_raw(
                image_data.width as u32,
                image_data.height as u32,
                image_data.bytes.into_owned(),
            )
            .ok_or_else(|| "CLIPBOARD_IMAGE_DECODE: invalid rgba buffer".to_string())?;

            let mut cursor = std::io::Cursor::new(Vec::new());
            image::DynamicImage::ImageRgba8(rgba)
                .write_to(&mut cursor, image::ImageFormat::Png)
                .map_err(|e| format!("CLIPBOARD_IMAGE_ENCODE: {e}"))?;

            let b64 = STANDARD.encode(cursor.into_inner());
            Ok(Some(ClipboardImageResponse {
                data_url: format!("data:image/png;base64,{b64}"),
                mime: "image/png".to_string(),
                filename: "clipboard-image.png".to_string(),
            }))
        })
        .await
        .map_err(|e| format!("CLIPBOARD_IMAGE_TASK: {e}"))??;

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nonce_is_single_use() {
        let state = ClipboardBridgeState::new().unwrap();
        let nonce = "0123456789abcdef0123456789abcdef";
        let issued_at = current_timestamp_millis();
        let proof = STANDARD.encode(state.expected_proof(nonce, issued_at));

        state.verify_and_consume(nonce, issued_at, &proof).unwrap();
        let error = state
            .verify_and_consume(nonce, issued_at, &proof)
            .unwrap_err();
        assert!(error.starts_with("CLIPBOARD_NONCE_REPLAY:"));
    }

    #[test]
    fn forged_proof_does_not_consume_nonce() {
        let state = ClipboardBridgeState::new().unwrap();
        let nonce = "abcdefabcdefabcdefabcdefabcdefab";
        let issued_at = current_timestamp_millis();
        let proof = STANDARD.encode(state.expected_proof(nonce, issued_at));

        assert!(state
            .verify_and_consume(nonce, issued_at, "invalid")
            .is_err());
        state.verify_and_consume(nonce, issued_at, &proof).unwrap();
    }

    #[test]
    fn malformed_nonce_is_rejected() {
        let state = ClipboardBridgeState::new().unwrap();
        let proof = STANDARD.encode([0u8; PROOF_BYTES]);

        assert!(state
            .verify_and_consume("not-a-nonce", current_timestamp_millis(), &proof)
            .is_err());
    }

    #[test]
    fn stale_timestamp_is_rejected() {
        let state = ClipboardBridgeState::new().unwrap();
        let nonce = "0123456789abcdef0123456789abcdef";
        let issued_at = current_timestamp_millis().saturating_sub(NONCE_TTL.as_millis() as u64 + 1);
        let proof = STANDARD.encode(state.expected_proof(nonce, issued_at));

        let error = state
            .verify_and_consume(nonce, issued_at, &proof)
            .unwrap_err();
        assert!(error.starts_with("CLIPBOARD_NONCE_EXPIRED:"));
    }

    #[test]
    fn future_timestamp_is_rejected() {
        let issued_at = current_timestamp_millis().saturating_add(1_000);

        assert!(!timestamp_is_fresh(issued_at));
    }

    fn current_timestamp_millis() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
    }
}
