//! 应用状态契约 —— §2.1。
//!
//! 这里定义的枚举与状态字段是跨模块、跨里程碑的核心契约（Tier 1），
//! 只在本模块定义一次，其他模块引用使用。

use serde::{Deserialize, Serialize};

/// 录音状态机。状态转移见 §4.1.2。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordingState {
    /// 空闲，等待用户操作
    Idle,
    /// 正在录音
    // 由 M2 录音按钮状态机构造（§4.1.2）。
    #[allow(dead_code)]
    Recording,
    /// 正在处理（上传转写 / 本地推理）
    // 由 M2/M4 在转写阶段构造（§4.1.2）。
    #[allow(dead_code)]
    Processing,
}

/// 转录处理模式（用户在主界面切换）。
///
/// 语义：此枚举描述"如何处理音频数据"，而非"由谁识别"。
/// "由谁识别"（云端/本地）由 `AsrConfig.backend_id` 决定（见 §2.5）。
/// 本地后端（qwen-asr）当前仅支持 Offline 模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TranscriptionMode {
    /// 实时流式：音频分帧实时发送，识别结果增量返回
    Streaming,
    /// 离线整段：录音完成后一次性发送完整音频转写
    Offline,
}

/// 应用全局状态（§2.1）。
///
/// 字段集合是契约；其在 GPUI 中的承载方式属实现细节。
#[derive(Debug, Clone)]
pub struct AppState {
    /// 当前录音状态
    pub recording_state: RecordingState,
    /// 用户选择的转录模式
    pub transcription_mode: TranscriptionMode,
    /// 文本编辑器中已确认（稳定）的文本
    // M1 文本由编辑器实体直接持有；本契约字段在 M6 流式增量更新时维护。
    #[allow(dead_code)]
    pub text_content: String,
    /// 流式识别中未稳定的尾部文本（视觉区分，见 §4.2.1）
    // 在 M6 流式识别中维护（§4.2.1）。
    #[allow(dead_code)]
    pub pending_text: String,
    /// 当前录音时长（秒），仅 Recording/Processing 有意义
    pub recording_duration_secs: u32,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            recording_state: RecordingState::Idle,
            transcription_mode: TranscriptionMode::Streaming,
            text_content: String::new(),
            pending_text: String::new(),
            recording_duration_secs: 0,
        }
    }
}
