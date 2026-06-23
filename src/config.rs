//! 持久化配置（§2.7 schema、§8 配置与安全管理）—— M2 任务 2.3 / 2.4。
//!
//! - 以 TOML 存储于各平台配置目录（§8.1），路径经 `directories` 解析，不硬编码。
//! - 敏感字段 `asr.api_key` 以 AES-256-GCM 加密落盘，明文仅存在于内存（§8.3）。
//! - 加载时不存在则返回默认值；保存时自动加密 API Key。

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
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
    pub audio: AudioConfig,
    pub shortcuts: ShortcutsConfig,
    pub text: TextConfig,
    pub storage: StorageConfig,
    pub window: WindowConfig,
    pub polish: PolishConfig,
    pub mini: MiniConfig,
    pub update: UpdateConfig,
}

/// 自动更新状态（§2.7 的 `[update]` 段，M13）。非用户直改字段，由更新模块维护。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct UpdateConfig {
    /// 上次检查更新的 Unix 时间戳（秒）；0 = 从未检查。用于启动时每日节流。
    pub last_check: i64,
    /// 用户「跳过此版本」选择的版本号（如 "0.3.0"）；该版本不再弹启动提示。
    pub skipped_version: String,
}

/// 迷你条窗口位置（物理像素，跨会话持久化）。`saved=false` 时用默认右上角。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct MiniConfig {
    pub saved: bool,
    pub x: i32,
    pub y: i32,
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
    /// 启动时静默检查 GitHub 新版本（每日至多一次，见 [`UpdateConfig`]）。M13。
    pub auto_check_update: bool,
}

/// 持久化的 ASR 设置（§2.7 的 `[asr]` 段）。
///
/// 注意：这与运行期的 `AsrConfig`（§2.5，M4 落地）是不同的类型——
/// 持久化配置含 `default_mode` / `max_recording_seconds` 等界面级设置。
///
/// 2026-06-14 重构：实时(streaming) 与离线(offline) 各自独立选择后端实现，且**每个后端**有独立配置
/// （endpoint、api_key；大文件后端另含 OSS 参数）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AsrSettings {
    /// 默认转录模式，对应 `TranscriptionMode`
    pub default_mode: TranscriptionMode,
    /// "zh" | "en" | "auto"
    pub language: String,
    pub max_recording_seconds: u32,
    /// 实时模式选用的后端 id（须支持流式）。
    pub streaming_backend: String,
    /// 离线模式选用的后端 id（须支持离线）。
    pub offline_backend: String,
    /// 各后端的独立配置（按后端 id 索引）。BTreeMap 保证序列化顺序稳定。
    pub backends: BTreeMap<String, BackendSettings>,
}

/// 单个后端的独立配置（§2.7 的 `[asr.backends.<id>]`）。
/// 敏感字段 `api_key` / `oss_access_key_secret` 落盘前加密（§8.3），内存中为明文。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct BackendSettings {
    /// API Key（云服务/自建服务，可选）。留空则运行期回退到环境变量：
    /// 阿里云后端用 `DASHSCOPE_API_KEY`，Qwen3-ASR 自建服务用 `QWEN3_ASR_API_KEY`。
    pub api_key: String,
    /// 接入地址；留空则用后端内置默认值。
    pub endpoint: String,
    /// 以下仅大文件后端 `aliyun_bailian_filetrans` 需要（OSS 中转）。
    pub oss_endpoint: String,
    pub oss_bucket: String,
    pub oss_access_key_id: String,
    pub oss_access_key_secret: String,
}

impl AsrSettings {
    /// 取某后端的配置副本（不存在则返回该后端的默认值）。
    pub fn backend(&self, id: &str) -> BackendSettings {
        self.backends.get(id).cloned().unwrap_or_default()
    }
}

/// 音频输入设置（§2.7 的 `[audio]` 段，2026-06-19）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioConfig {
    /// 首选麦克风设备名（cpal 设备名）。留空 = 跟随系统默认输入设备。
    /// 设备不存在时（如已拔出）运行期回退默认设备。
    pub input_device: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ShortcutsConfig {
    // 全局热键（OS 级注册，应用不在前台也生效）。
    pub toggle_recording: String,
    pub toggle_window: String,
    pub copy_and_paste: String,
    pub toggle_mini_bar: String,
    // 应用内快捷键（仅主窗口聚焦时生效，不做 OS 注册）。
    pub app_copy_all: String,
    pub app_new_record: String,
    pub app_toggle_mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TextConfig {
    pub auto_copy: bool,
    pub append_mode: bool,
    pub history_retention_days: u32,
}

/// 录音音频文件的存储设置（§2.7 的 `[storage]` 段，2026-06-16）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct StorageConfig {
    /// 是否持久化录音音频；false 则沿用临时文件、转写后删除（不入库）。
    pub save_audio: bool,
    /// 音频根目录；留空则用默认（`{LOCALAPPDATA}/VoxInk/recordings`，见 [`StorageConfig::audio_root`]）。
    /// 更改仅对**新**录音生效，已有片段在 DB 中记绝对路径、留在原处（§4.2.2）。
    pub audio_dir: String,
    /// 音频保留天数；0 表示永久保留。与文本保留（`text.history_retention_days`）独立。
    pub audio_retention_days: u32,
}

impl StorageConfig {
    /// 默认音频根目录：`{平台本地数据目录}/VoxInk/recordings`。
    /// 大体积媒体放本地数据目录（Windows 为 `%LOCALAPPDATA%`），不放可漫游的配置目录。
    pub fn default_audio_root() -> Result<PathBuf> {
        let base = BaseDirs::new().context("无法定位用户数据目录")?;
        Ok(base.data_local_dir().join("VoxInk").join("recordings"))
    }

    /// 当前生效的音频根目录：配置非空则用之，否则用默认。
    pub fn audio_root(&self) -> Result<PathBuf> {
        if self.audio_dir.trim().is_empty() {
            Self::default_audio_root()
        } else {
            Ok(PathBuf::from(self.audio_dir.trim()))
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowConfig {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// 转录后 AI 润色设置（`[polish]` 段）。
///
/// 单一 OpenAI 兼容客户端：用户配置 `base_url + model + api_key`（主流厂商均提供
/// OpenAI 兼容 `/chat/completions`）。`api_key` 落盘前加密（§8.3），内存中为明文。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PolishConfig {
    /// OpenAI 兼容 API 基址（如 `https://api.deepseek.com/v1`）。空 = 未配置。
    pub base_url: String,
    /// 模型名（如 `deepseek-chat`、`gpt-4o-mini`、`qwen-plus`）。
    pub model: String,
    /// API Key（加密落盘）。
    pub api_key: String,
    /// 采样温度（润色宜偏低，默认 0.3）。
    pub temperature: f32,
    /// 当前选用的模板 id。
    pub active_template: String,
    /// 润色场景模板（内置 + 用户改写）。缺省回退内置集，避免空模板列表。
    #[serde(default = "default_polish_templates")]
    pub templates: Vec<PolishTemplate>,
}

/// 单个润色模板：以 `prompt` 作为 system 提示词。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PolishTemplate {
    pub id: String,
    pub name: String,
    pub prompt: String,
}

impl PolishConfig {
    /// 取当前选用模板（找不到则取第一个）。
    pub fn active(&self) -> Option<&PolishTemplate> {
        self.templates
            .iter()
            .find(|t| t.id == self.active_template)
            .or_else(|| self.templates.first())
    }
}

/// 是否为内置模板（按固定 id 判定）。内置模板可改提示词但不可改名/删除。
pub fn is_builtin_template(id: &str) -> bool {
    matches!(id, "general" | "written" | "meeting" | "todo" | "email")
}

/// 内置默认润色模板（首次运行写入；用户可在设置内改写其提示词）。
pub fn default_polish_templates() -> Vec<PolishTemplate> {
    let t = |id: &str, name: &str, prompt: &str| PolishTemplate {
        id: id.to_string(),
        name: name.to_string(),
        prompt: prompt.to_string(),
    };
    vec![
        t(
            "general",
            "通用润色",
            "你是中文文字润色助手。请在不改变原意的前提下，修正语音转写文本中的错别字、断句与标点，去除口语赘词与重复，使其通顺自然。只输出润色后的正文，不要解释。",
        ),
        t(
            "written",
            "口语转书面",
            "请把下面这段口语化的语音转写整理成书面、正式的表达：去除语气词与重复，规范用词与标点，保持原意。只输出整理后的正文。",
        ),
        t(
            "meeting",
            "会议纪要",
            "请把下面的语音转写整理成结构化会议纪要：用要点列出关键决议、讨论事项与待办（含负责人/时间，如有）。只输出纪要正文。",
        ),
        t(
            "todo",
            "待办清单",
            "请从下面的语音转写中提炼出可执行的待办清单，用简洁的条目列出，每条一个动作。只输出清单。",
        ),
        t(
            "email",
            "邮件",
            "请把下面的语音转写改写成一封措辞得体、条理清晰的中文邮件（含称呼与结尾），保持原意。只输出邮件正文。",
        ),
    ]
}

// ───────────────────────────── 默认值（§2.7）─────────────────────────────

impl Default for VoxInkConfig {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
            general: GeneralConfig::default(),
            asr: AsrSettings::default(),
            audio: AudioConfig::default(),
            shortcuts: ShortcutsConfig::default(),
            text: TextConfig::default(),
            storage: StorageConfig::default(),
            window: WindowConfig::default(),
            polish: PolishConfig::default(),
            mini: MiniConfig::default(),
            update: UpdateConfig::default(),
        }
    }
}

impl Default for PolishConfig {
    fn default() -> Self {
        Self {
            base_url: String::new(),
            model: String::new(),
            api_key: String::new(),
            temperature: 0.3,
            active_template: "general".to_string(),
            templates: default_polish_templates(),
        }
    }
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            save_audio: true,
            audio_dir: String::new(),
            audio_retention_days: 90,
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
            auto_check_update: true,
        }
    }
}

impl Default for AsrSettings {
    fn default() -> Self {
        Self {
            default_mode: TranscriptionMode::Streaming,
            language: "zh".to_string(),
            max_recording_seconds: 600,
            streaming_backend: "aliyun_bailian_streaming".to_string(),
            offline_backend: "aliyun_bailian_offline".to_string(),
            backends: BTreeMap::new(),
        }
    }
}

impl Default for ShortcutsConfig {
    fn default() -> Self {
        Self {
            // 默认采用 Ctrl+Shift 系（2026-06-19）：相比 Ctrl+Alt 更少被全局工具占用，
            // 且键位有助记含义——Space=说话、W=Window、V=paste。可在设置面板内捕获改键。
            toggle_recording: "Ctrl+Shift+Space".to_string(),
            toggle_window: "Ctrl+Shift+W".to_string(),
            copy_and_paste: "Ctrl+Shift+V".to_string(),
            // B=Bar（迷你状态条）。
            toggle_mini_bar: "Ctrl+Shift+B".to_string(),
            // 应用内：C=Copy、N=New、M=Mode（聚焦主窗口时生效）。
            app_copy_all: "Ctrl+Shift+C".to_string(),
            app_new_record: "Ctrl+Shift+N".to_string(),
            app_toggle_mode: "Ctrl+Shift+M".to_string(),
        }
    }
}

impl Default for TextConfig {
    fn default() -> Self {
        Self {
            auto_copy: false,
            append_mode: true,
            // 文本记录默认保留 1 年（音频另由 storage.audio_retention_days 控制，默认 90 天）。
            history_retention_days: 365,
        }
    }
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            x: 0,
            y: 0,
            // 双栏布局（左记录栏 + 右编辑区）默认更宽（§6.1，2026-06-14 重设计）。
            width: 920,
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

    /// 日志目录：`{平台本地数据目录}/VoxInk/logs`（Windows 为 `%LOCALAPPDATA%`）。
    /// 与录音归档同放本地数据目录，不放可漫游的配置目录。
    pub fn log_dir() -> Result<PathBuf> {
        let base = BaseDirs::new().context("无法定位用户数据目录")?;
        Ok(base.data_local_dir().join("VoxInk").join("logs"))
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

        // 解密各后端的敏感字段（失败则清空该字段，不阻断启动）。
        for (id, b) in config.asr.backends.iter_mut() {
            for (field, value) in [
                ("api_key", &mut b.api_key),
                ("oss_access_key_secret", &mut b.oss_access_key_secret),
            ] {
                match decrypt_api_key(value) {
                    Ok(plain) => *value = plain,
                    Err(e) => {
                        tracing::warn!(
                            "后端 {id} 的 {field} 解密失败（可能更换了设备），已清空: {e:#}"
                        );
                        value.clear();
                    }
                }
            }
        }

        // 解密润色 API Key。
        match decrypt_api_key(&config.polish.api_key) {
            Ok(plain) => config.polish.api_key = plain,
            Err(e) => {
                tracing::warn!("润色 api_key 解密失败（可能更换了设备），已清空: {e:#}");
                config.polish.api_key.clear();
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
        for b in to_store.asr.backends.values_mut() {
            b.api_key = encrypt_api_key(&b.api_key)?;
            b.oss_access_key_secret = encrypt_api_key(&b.oss_access_key_secret)?;
        }
        to_store.polish.api_key = encrypt_api_key(&to_store.polish.api_key)?;

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

    let blob = BASE64
        .decode(blob_b64)
        .context("API Key 密文 base64 解码失败")?;
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
        assert_eq!(back.window.width, 920);
    }

    #[test]
    fn default_mode_serializes_lowercase() {
        let cfg = VoxInkConfig::default();
        let text = toml::to_string_pretty(&cfg).unwrap();
        assert!(text.contains("default_mode = \"streaming\""));
    }
}
