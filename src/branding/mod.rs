//! 品牌图标运行期集成：把纯绘制（[`draw`]）接到托盘 / 任务栏，并按录制状态刷新。

mod draw;
pub use draw::{IconStatus, render_badge_rgba, render_icon_rgba};

use crate::state::{RecordingState, TranscriptionMode};

/// 由录制状态 + 转录模式映射到图标状态（单一映射真源）。
pub fn icon_status(state: RecordingState, mode: TranscriptionMode) -> IconStatus {
    match state {
        RecordingState::Idle => IconStatus::Idle,
        RecordingState::Recording => match mode {
            TranscriptionMode::Streaming => IconStatus::RecordingRealtime,
            TranscriptionMode::Offline => IconStatus::Recording,
            // 仅录音用灰色徽标，与片段列表的灰圆点全局一致。
            TranscriptionMode::RecordOnly => IconStatus::RecordingOnly,
        },
        RecordingState::Processing => IconStatus::Transcribing,
    }
}

/// 构建托盘图标（teal 品牌底 + 气泡波形 + 状态徽标）。
pub fn tray_icon(status: IconStatus) -> Option<tray_icon::Icon> {
    const SIZE: u32 = 32;
    tray_icon::Icon::from_rgba(render_icon_rgba(SIZE, status), SIZE, SIZE).ok()
}
