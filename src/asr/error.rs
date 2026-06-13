//! ASR 错误分类契约 —— §2.3（Tier 1）。
//!
//! 所有后端必须使用此错误类型，不得对外暴露 `anyhow` 或裸字符串错误。
//! 传输层（reqwest / tokio-tungstenite / 本地推理）的错误在各后端内部转换为
//! 合适的变体（见 §2.4 解耦决策）。

/// ASR 统一错误分类法（taxonomy）。
///
/// 部分变体由后续里程碑的后端构造（WebSocketError→M6 流式；ModelNotFound/InferenceError→M8
/// 本地引擎），M4 尚未触达，故整体允许 dead_code 以保持契约完整。
#[derive(Debug, thiserror::Error)]
#[allow(dead_code)]
pub enum AsrError {
    /// 网络/传输层失败。携带可读描述字符串（见 §2.4）。
    #[error("网络连接失败: {0}")]
    NetworkError(String),

    #[error("WebSocket 连接失败: {0}")]
    WebSocketError(String),

    #[error("API 鉴权失败，请检查 API Key")]
    AuthError,

    #[error("API 配额已用尽: {0}")]
    QuotaExceeded(String),

    #[error("音频格式不支持: {0}")]
    UnsupportedFormat(String),

    #[error("转写超时")]
    Timeout,

    #[error("未识别到语音内容")]
    EmptyResult,

    #[error("录音数据为空")]
    EmptyAudio,

    #[error("本地模型未找到: {0}")]
    ModelNotFound(String),

    #[error("本地推理失败: {0}")]
    InferenceError(String),

    #[error("配置无效: {0}")]
    InvalidConfig(String),

    #[error("IO 错误: {0}")]
    IoError(#[from] std::io::Error),
}
