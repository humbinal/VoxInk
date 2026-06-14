//! 持久化配置（§2.7 schema、§8 配置与安全管理）—— M2 任务 2.3 / 2.4。
//!
//! - 以 TOML 存储于各平台配置目录（§8.1），路径经 `directories` 解析，不硬编码。
//! - 敏感字段 `asr.api_key` 以 AES-256-GCM 加密落盘，明文仅存在于内存（§8.3）。
//! - 加载时不存在则返回默认值；保存时自动加密 API Key。

use std::fs;
use std::path::PathBuf;

use anyhow::{anyhow, bail, Context, Result};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use directories::BaseDirs;
use serde::{Deserialize, Serialize};

use crate::state::TranscriptionMode;

/// 配置文件结构版本，未来升级据此迁移（§8.2）。
const CONFIG_VERSION: u32 = 1;

// ───────────────────────────── 配置 Schema（§2.7）─────────────────────────────

/// 顶层配置。字段结构与语义见 §2.7。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct VoxInkConfig {
    pub version: u32,
    pub general: GeneralConfig,
    pub asr: AsrSettings,
    pub shortcuts: ShortcutsConfig,
    pub text: TextConfig,
    pub window: WindowConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GeneralConfig {
    /// 界面语言
    pub language: String,
    /// "light" | "dark" | "system"
    pub theme: String,
    pub launch_at_startup: bool,
    pub start_minimized: bool,
    pub window_on_top: bool,
    pub audio_feedback: bool,
}

/// 持久化的 ASR 设置（§2.7 的 `[asr]` 段）。
///
/// 注意：这与运行期的 `AsrConfig`（§2.5，M4 落地）是不同的类型——
/// 持久化配置含 `default_mode` / `max_recording_seconds` 等界面级设置。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AsrSettings {
    pub backend_id: String,
    /// 默认转录模式，对应 `TranscriptionMode`
    pub default_mode: TranscriptionMode,
    pub api_endpoint: String,
    /// 加密存储（§8.3）。内存中为明文，落盘前加密。
    pub api_key: String,
    /// "zh" | "en" | "auto"
    pub language: String,
    pub max_recording_seconds: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ShortcutsConfig {
    pub toggle_recording: String,
    pub toggle_window: String,
    pub copy_and_paste: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TextConfig {
    pub auto_copy: bool,
    pub append_mode: bool,
    pub history_retention_days: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowConfig {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

// ───────────────────────────── 默认值（§2.7）─────────────────────────────

impl Default for VoxInkConfig {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
            general: GeneralConfig::default(),
            asr: AsrSettings::default(),
            shortcuts: ShortcutsConfig::default(),
            text: TextConfig::default(),
            window: WindowConfig::default(),
        }
    }
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            language: "zh-CN".to_string(),
            theme: "system".to_string(),
            launch_at_startup: false,
            start_minimized: true,
            window_on_top: false,
            audio_feedback: true,
        }
    }
}

impl Default for AsrSettings {
    fn default() -> Self {
        Self {
            backend_id: "aliyun_bailian_streaming".to_string(),
            default_mode: TranscriptionMode::Streaming,
            api_endpoint: "wss://dashscope.aliyuncs.com/api-ws/v1/inference".to_string(),
            api_key: String::new(),
            language: "zh".to_string(),
            max_recording_seconds: 600,
        }
    }
}

impl Default for ShortcutsConfig {
    fn default() -> Self {
        Self {
            toggle_recording: "Ctrl+Alt+Space".to_string(),
            toggle_window: "Ctrl+Alt+V".to_string(),
            copy_and_paste: "Ctrl+Alt+B".to_string(),
        }
    }
}

impl Default for TextConfig {
    fn default() -> Self {
        Self {
            auto_copy: false,
            append_mode: true,
            history_retention_days: 30,
        }
    }
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            x: 0,
            y: 0,
            // 双栏布局（左记录栏 + 右编辑区）默认更宽（§6.1，2026-06-14 重设计）。
            width: 880,
            height: 640,
        }
    }
}

// ───────────────────────────── 加载 / 保存 ─────────────────────────────

impl VoxInkConfig {
    /// 配置文件路径：`{平台配置目录}/VoxInk/config.toml`（§8.1）。
    pub fn config_path() -> Result<PathBuf> {
        let base = BaseDirs::new().context("无法定位用户配置目录")?;
        Ok(base.config_dir().join("VoxInk").join("config.toml"))
    }

    /// 加载配置；文件不存在或解析失败时返回默认值（并记录日志）。
    /// 读取后会把密文 `api_key` 解密为内存中的明文。
    pub fn load() -> Self {
        let path = match Self::config_path() {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!("无法解析配置路径，使用默认配置: {e:#}");
                return Self::default();
            }
        };

        if !path.exists() {
            tracing::info!("配置文件不存在，使用默认配置: {}", path.display());
            return Self::default();
        }

        let text = match fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!("读取配置失败，使用默认配置: {e:#}");
                return Self::default();
            }
        };

        let mut config: VoxInkConfig = match toml::from_str(&text) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("解析配置失败，使用默认配置: {e:#}");
                return Self::default();
            }
        };

        // 解密 API Key（失败则清空，不阻断启动）。
        match decrypt_api_key(&config.asr.api_key) {
            Ok(plain) => config.asr.api_key = plain,
            Err(e) => {
                tracing::warn!("API Key 解密失败（可能更换了设备），已清空: {e:#}");
                config.asr.api_key = String::new();
            }
        }

        config
    }

    /// 保存配置；落盘前加密 `api_key`，明文不写入磁盘。
    pub fn save(&self) -> Result<()> {
        let path = Self::config_path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("创建配置目录失败: {}", parent.display()))?;
        }

        // 仅在副本上加密，保持内存中的明文不变。
        let mut to_store = self.clone();
        to_store.asr.api_key = encrypt_api_key(&self.asr.api_key)?;

        let text = toml::to_string_pretty(&to_store).context("序列化配置为 TOML 失败")?;
        fs::write(&path, text).with_context(|| format!("写入配置失败: {}", path.display()))?;
        Ok(())
    }
}

// ───────────────────────────── API Key 加密（§8.3）─────────────────────────────
//
// 方案：HKDF-SHA256(ikm = machine-id, salt = 随机) 派生 256-bit 密钥，AES-256-GCM 加密。
// 存储格式：base64(salt[16] || nonce[12] || ciphertext || tag[16])。
//
// ⚠️ 与 §8.3 的差异并记录：§8.3 示意格式为 base64(nonce || ciphertext || tag)，未指明
// 随机 salt 的存放位置。这里把 salt 一并前置进密文块，使其自包含、可跨重启解密，
// 同时保持"机器绑定 + 随机 salt + AES-256-GCM"的安全语义不变。

const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;
const KEY_LEN: usize = 32;
const TAG_LEN: usize = 16;
const HKDF_INFO: &[u8] = b"voxink.api_key.v1";

/// 机器唯一标识作为 HKDF 的输入密钥材料（IKM）。
fn machine_id_bytes() -> Vec<u8> {
    match machine_uid::get() {
        Ok(id) => id.into_bytes(),
        Err(e) => {
            // 取不到机器 ID 时回退到固定材料，保证功能可用（安全性降级，记录告警）。
            tracing::warn!("无法获取机器唯一标识，使用回退密钥材料: {e}");
            b"voxink-fallback-machine-id".to_vec()
        }
    }
}

fn derive_key(salt: &[u8]) -> [u8; KEY_LEN] {
    let ikm = machine_id_bytes();
    let hk = hkdf::Hkdf::<sha2::Sha256>::new(Some(salt), &ikm);
    let mut okm = [0u8; KEY_LEN];
    hk.expand(HKDF_INFO, &mut okm)
        .expect("HKDF-SHA256 输出 32 字节是有效长度");
    okm
}

/// 加密 API Key 明文为 base64 密文块；空串原样返回空串。
fn encrypt_api_key(plaintext: &str) -> Result<String> {
    use aes_gcm::aead::{Aead, KeyInit};
    use aes_gcm::{Aes256Gcm, Key, Nonce};

    if plaintext.is_empty() {
        return Ok(String::new());
    }

    let mut salt = [0u8; SALT_LEN];
    let mut nonce_bytes = [0u8; NONCE_LEN];
    getrandom::getrandom(&mut salt).map_err(|e| anyhow!("生成随机 salt 失败: {e}"))?;
    getrandom::getrandom(&mut nonce_bytes).map_err(|e| anyhow!("生成随机 nonce 失败: {e}"))?;

    let key = derive_key(&salt);
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce_bytes), plaintext.as_bytes())
        .map_err(|e| anyhow!("AES-256-GCM 加密失败: {e}"))?;

    let mut blob = Vec::with_capacity(SALT_LEN + NONCE_LEN + ciphertext.len());
    blob.extend_from_slice(&salt);
    blob.extend_from_slice(&nonce_bytes);
    blob.extend_from_slice(&ciphertext);
    Ok(BASE64.encode(blob))
}

/// 解密 base64 密文块为明文；空串原样返回空串。
fn decrypt_api_key(blob_b64: &str) -> Result<String> {
    use aes_gcm::aead::{Aead, KeyInit};
    use aes_gcm::{Aes256Gcm, Key, Nonce};

    if blob_b64.is_empty() {
        return Ok(String::new());
    }

    let blob = BASE64.decode(blob_b64).context("API Key 密文 base64 解码失败")?;
    if blob.len() < SALT_LEN + NONCE_LEN + TAG_LEN {
        bail!("API Key 密文长度不足");
    }

    let (salt, rest) = blob.split_at(SALT_LEN);
    let (nonce_bytes, ciphertext) = rest.split_at(NONCE_LEN);

    let key = derive_key(salt);
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
    let plaintext = cipher
        .decrypt(Nonce::from_slice(nonce_bytes), ciphertext)
        .map_err(|e| anyhow!("AES-256-GCM 解密失败: {e}"))?;

    String::from_utf8(plaintext).context("解密结果不是有效 UTF-8")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_key_encrypt_roundtrip() {
        let secret = "sk-bailian-1234567890ABCDEF";
        let blob = encrypt_api_key(secret).unwrap();
        // 密文不等于明文，且不包含明文片段。
        assert_ne!(blob, secret);
        assert!(!blob.contains("sk-bailian"));
        // 可正确还原。
        assert_eq!(decrypt_api_key(&blob).unwrap(), secret);
    }

    #[test]
    fn api_key_empty_is_passthrough() {
        assert_eq!(encrypt_api_key("").unwrap(), "");
        assert_eq!(decrypt_api_key("").unwrap(), "");
    }

    #[test]
    fn api_key_each_encryption_differs() {
        // 随机 salt/nonce 使同一明文每次密文不同。
        let a = encrypt_api_key("same-secret").unwrap();
        let b = encrypt_api_key("same-secret").unwrap();
        assert_ne!(a, b);
        assert_eq!(decrypt_api_key(&a).unwrap(), "same-secret");
        assert_eq!(decrypt_api_key(&b).unwrap(), "same-secret");
    }

    #[test]
    fn default_config_toml_roundtrips() {
        let cfg = VoxInkConfig::default();
        let text = toml::to_string_pretty(&cfg).unwrap();
        let back: VoxInkConfig = toml::from_str(&text).unwrap();
        assert_eq!(back.version, cfg.version);
        assert_eq!(back.asr.default_mode, cfg.asr.default_mode);
        assert_eq!(back.window.width, 880);
    }

    #[test]
    fn default_mode_serializes_lowercase() {
        let cfg = VoxInkConfig::default();
        let text = toml::to_string_pretty(&cfg).unwrap();
        assert!(text.contains("default_mode = \"streaming\""));
    }
}
