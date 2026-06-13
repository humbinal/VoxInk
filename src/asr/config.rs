//! ASR 运行期配置契约 —— §2.5（Tier 1）。
//!
//! 这是传给后端的**运行期**配置，区别于持久化的 `crate::config::VoxInkConfig`（§2.7）。
//! 由后者的 `[asr]` 段映射而来（api_key 为解密后的明文）。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AsrConfig {
    /// 当前使用的后端 ID（对应 §7.4 注册表）
    pub backend_id: String,
    /// API Key（云服务）。持久化时加密，见 §8.3；此处为明文。
    pub api_key: String,
    /// API Endpoint
    pub api_endpoint: String,
    /// 本地引擎模型路径（qwen-asr），云后端为 None
    pub local_model_path: Option<String>,
    /// qwen-asr 模型规格（见 §4.3.3）
    pub local_model_size: Option<String>,
    /// 语言代码（"zh" / "en" / "auto"）
    pub language: String,

    // ── OSS 字段（§2.5 的 M4 混合扩展）──
    // qwen3-asr-flash-filetrans 仅接受公网 URL，故大文件需先上传到用户 OSS。
    // 这些字段在 M4 阶段由环境变量填充，M11 设置面板上线后改由加密配置提供。
    /// OSS endpoint，如 `oss-cn-beijing.aliyuncs.com`
    pub oss_endpoint: String,
    /// OSS Bucket 名
    pub oss_bucket: String,
    /// OSS AccessKey ID
    pub oss_access_key_id: String,
    /// OSS AccessKey Secret（敏感）
    pub oss_access_key_secret: String,
}
