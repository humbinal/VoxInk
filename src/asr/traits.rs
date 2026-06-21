//! ASR 后端契约 —— §2.2（Tier 1）。
//!
//! 所有后端（云服务、自定义服务）必须实现 `AsrBackend`。
//!
//! 关于 `#[async_trait]`：当前 Rust 原生 async fn in trait 不是 dyn-compatible，
//! 而注册表需要 `Arc<dyn AsrBackend>`（trait 对象），故使用 `async_trait` 宏装箱
//! 返回的 future（§2.2 已声明此为实现细节，以当前工具链为准）。

use async_trait::async_trait;

use super::config::AsrConfig;
use super::error::AsrError;

/// ASR 后端统一接口。
///
/// 约束：Send + Sync（线程间安全传递）+ 'static（可放入 tokio::spawn）。
#[async_trait]
pub trait AsrBackend: Send + Sync + 'static {
    /// 后端唯一标识符，对应 `AsrConfig.backend_id`。
    /// 示例: "aliyun_bailian_streaming", "qwen3_asr_selfhosted"
    fn backend_id(&self) -> &str;

    /// 用户可见的后端名称。示例: "阿里云百炼（实时）", "Qwen3-ASR 自建服务"
    fn display_name(&self) -> &str;

    /// 本后端是否支持实时流式识别。
    fn supports_streaming(&self) -> bool;

    /// 本后端是否支持离线整段识别。
    fn supports_offline(&self) -> bool;

    /// 本后端单次录音的硬时长上限（秒）。`None` = 无后端侧限制，仅受用户配置的
    /// `max_recording_seconds` 约束。
    ///
    /// 用于录制侧**提前自动停止**：例如离线同步后端受请求体（~10MB base64）限制，
    /// 录太久会在上传阶段失败，故在达到能力上限时就停止录音，而非录完才报错。
    fn max_recording_seconds(&self) -> Option<u32> {
        None
    }

    /// 验证配置是否有效（如测试 API Key 连通性）。
    async fn validate_config(&self, config: &AsrConfig) -> Result<(), AsrError>;

    /// 实时流式识别。
    /// - `audio_rx`：音频 chunk 接收通道，每个 chunk 为 16kHz/16-bit/单声道 PCM 字节；
    ///   通道关闭表示录音结束，后端应发送结束信号并等待最终结果。
    /// - `result_tx`：识别结果发送通道，实时发送 partial/final；发完 final 后可 drop。
    async fn transcribe_streaming(
        &self,
        config: &AsrConfig,
        audio_rx: tokio::sync::mpsc::Receiver<Vec<u8>>,
        result_tx: tokio::sync::mpsc::Sender<StreamingResult>,
    ) -> Result<(), AsrError>;

    /// 离线整段识别。
    /// - `audio_data`：完整 WAV 文件字节。
    /// - 返回完整转写文本。
    async fn transcribe_offline(
        &self,
        config: &AsrConfig,
        audio_data: Vec<u8>,
    ) -> Result<String, AsrError>;
}

/// 流式识别的单次增量结果（§2.2）。
#[derive(Debug, Clone)]
#[allow(dead_code)] // 由 M6 流式后端构造与消费
pub struct StreamingResult {
    /// 本次增量文本（仅新增部分）。
    pub delta_text: String,
    /// 是否为句子结束的最终结果。
    /// true：文本已稳定，应转为正常样式并固化到 `text_content`；
    /// false：中间结果，应以斜体/浅色显示在 `pending_text`。
    pub is_final: bool,
    /// 结果时间戳。
    pub timestamp: chrono::DateTime<chrono::Utc>,
}
