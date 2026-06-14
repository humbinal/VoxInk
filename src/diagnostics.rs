//! 诊断信息导出（M11 任务 11.4）。把环境/版本/配置（脱敏）写入文本文件，便于排障。

use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::config::VoxInkConfig;

/// 构建期注入（见 build.rs）。`BUILD_TIME` 为 RFC3339 UTC。
pub const GIT_HASH: &str = env!("VOXINK_GIT_HASH");
pub const BUILD_TIME: &str = env!("VOXINK_BUILD_TIME");
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// 构建时间按**用户本地时区**显示（带偏移，如 "2026-06-14 15:11:08 +08:00"）；解析失败回退原值。
pub fn build_time_display() -> String {
    match chrono::DateTime::parse_from_rfc3339(BUILD_TIME) {
        Ok(dt) => dt
            .with_timezone(&chrono::Local)
            .format("%Y-%m-%d %H:%M:%S %:z")
            .to_string(),
        Err(_) => BUILD_TIME.to_string(),
    }
}

/// 生成诊断文本（**不含任何密钥**，仅标注是否已配置）。
pub fn report(config: &VoxInkConfig) -> String {
    let env_key = std::env::var("DASHSCOPE_API_KEY")
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false);
    let any_backend_key = config
        .asr
        .backends
        .values()
        .any(|b| !b.api_key.trim().is_empty());
    let api_key_set = env_key || any_backend_key;
    format!(
        "VoxInk 诊断信息\n\
         ================\n\
         版本: {VERSION}\n\
         构建时间: {build_time}\n\
         Git 提交: {GIT_HASH}\n\
         操作系统: {os} {arch}\n\
         ----------------\n\
         界面语言: {language}\n\
         主题: {theme}\n\
         开机自启: {autostart}\n\
         启动最小化: {minimized}\n\
         窗口置顶: {on_top}\n\
         默认模式: {mode:?}\n\
         实时后端: {streaming_backend}\n\
         离线后端: {offline_backend}\n\
         识别语言: {asr_lang}\n\
         最长录音(秒): {max_secs}\n\
         API Key 已配置: {api_key_set}\n\
         历史保留(天): {retention}\n",
        build_time = build_time_display(),
        os = std::env::consts::OS,
        arch = std::env::consts::ARCH,
        language = config.general.language,
        theme = config.general.theme,
        autostart = config.general.launch_at_startup,
        minimized = config.general.start_minimized,
        on_top = config.general.window_on_top,
        mode = config.asr.default_mode,
        streaming_backend = config.asr.streaming_backend,
        offline_backend = config.asr.offline_backend,
        asr_lang = config.asr.language,
        max_secs = config.asr.max_recording_seconds,
        retention = config.text.history_retention_days,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_time_is_rfc3339_and_localized_with_offset() {
        assert!(
            chrono::DateTime::parse_from_rfc3339(BUILD_TIME).is_ok(),
            "BUILD_TIME 不是 RFC3339: {BUILD_TIME}"
        );
        let disp = build_time_display();
        // 本地化后应带时区偏移（+hh:mm 或 -hh:mm）。
        assert!(
            disp.contains('+') || disp.contains('-'),
            "本地构建时间缺少时区偏移: {disp}"
        );
    }
}

/// 导出诊断到配置目录，返回文件路径。
pub fn export(config: &VoxInkConfig) -> Result<PathBuf> {
    let dir = VoxInkConfig::config_path()?
        .parent()
        .map(|p| p.to_path_buf())
        .context("无法定位配置目录")?;
    let path = dir.join("voxink_diagnostics.txt");
    std::fs::write(&path, report(config))
        .with_context(|| format!("写入诊断文件失败: {}", path.display()))?;
    Ok(path)
}
