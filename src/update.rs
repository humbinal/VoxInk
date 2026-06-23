//! 版本检查与自动更新（M13，§11.3）。
//!
//! - 版本来源：GitHub Releases API（`releases/latest`），用 semver 与当前版本比较。
//! - 自替换：运行中的 exe 不可覆盖但可改名——下载新 exe → 校验 SHA256 →
//!   `当前 → *.old.exe`、`*.new.exe → 当前` 两步改名 → 启动新进程 + 退出。
//! - 启动时清理上次残留的 `*.old.exe`（[`cleanup_old_exe`]）。
//!
//! 网络调用须在 tokio 运行时执行（reqwest 需 reactor），由调用方在 `GlobalTokioHandle`
//! 上 `handle.spawn(...)`；本模块只提供纯 async/同步函数，不接触 GPUI。

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use anyhow::{Context, Result, anyhow, bail};
use futures_util::StreamExt;
use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};

/// 发布仓库（§11.3 契约）。
const REPO: &str = "humbinal/VoxInk";
/// 主程序产物名（CI 与客户端共同约定，§11.3）。
const EXE_ASSET: &str = "VoxInk.exe";
/// SHA256 摘要产物名。
const SHA256_ASSET: &str = "VoxInk.exe.sha256";

/// `releases/latest` 检查结果。
#[derive(Debug, Clone)]
pub struct LatestRelease {
    /// 去掉 `v` 前缀的版本号（如 "0.2.0"）。
    pub version: String,
    /// 是否比当前运行版本（`diagnostics::VERSION`）更新。
    pub is_newer: bool,
    /// Release 说明正文（展示用，可能为空）。
    pub changelog: String,
    /// `VoxInk.exe` 下载直链（缺失则为空）。
    pub exe_url: String,
    /// `VoxInk.exe.sha256` 下载直链（缺失则为空）。
    pub sha256_url: String,
}

// ── GitHub API 反序列化（仅取所需字段）──

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

/// Release 页面 URL（自更新不可用时供用户手动下载）。
pub fn release_page_url() -> String {
    format!("https://github.com/{REPO}/releases/latest")
}

/// 项目主页 URL（「关于」区跳转链接）。
pub fn repo_url() -> String {
    format!("https://github.com/{REPO}")
}

/// Releases 列表页 URL（网络受限无法在线更新时供用户手动下载任意版本）。
pub fn releases_url() -> String {
    format!("https://github.com/{REPO}/releases")
}

/// 构建带 GitHub 必需 `User-Agent` 的 HTTP 客户端（缺 UA → GitHub 返回 403）。
fn client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(format!("VoxInk/{}", crate::diagnostics::VERSION))
        .build()
        .context("构建 HTTP 客户端失败")
}

/// 拉取最新 Release 并与当前版本比较（§11.3）。
pub async fn check_latest() -> Result<LatestRelease> {
    let client = client()?;
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let resp = client
        .get(&url)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()
        .await
        .context("请求 GitHub Releases 失败")?
        .error_for_status()
        .context("GitHub Releases 返回错误状态")?;
    let release: Release = resp.json().await.context("解析 Release JSON 失败")?;

    let tag = release.tag_name.trim();
    // tag 约定为 vX.Y.Z；去前缀后按 semver 解析（§11.2 版本号契约）。
    let version = tag
        .strip_prefix('v')
        .or_else(|| tag.strip_prefix('V'))
        .unwrap_or(tag)
        .to_string();
    let latest = Version::parse(&version).with_context(|| format!("无法解析发布版本号: {tag}"))?;
    let current =
        Version::parse(crate::diagnostics::VERSION).context("无法解析当前程序版本号")?;

    let find = |name: &str| {
        release
            .assets
            .iter()
            .find(|a| a.name == name)
            .map(|a| a.browser_download_url.clone())
            .unwrap_or_default()
    };

    Ok(LatestRelease {
        is_newer: latest > current,
        version,
        changelog: release.body.unwrap_or_default(),
        exe_url: find(EXE_ASSET),
        sha256_url: find(SHA256_ASSET),
    })
}

/// 下载新版 exe、校验 SHA256 并就地替换当前可执行文件（§11.3 自替换流程）。
///
/// `progress` 为下载百分比（0–100），调用方可在前台轮询展示。成功返回后由调用方
/// [`spawn_restart`] 启动新进程并退出当前进程。失败时保证不留下半替换状态。
pub async fn download_and_apply(
    exe_url: String,
    sha256_url: String,
    progress: Arc<AtomicU8>,
) -> Result<()> {
    if exe_url.is_empty() || sha256_url.is_empty() {
        bail!("该 Release 缺少 {EXE_ASSET} 或 {SHA256_ASSET} 资产，无法自动更新");
    }
    let client = client()?;

    // 1. 下载期望的 SHA256（小文件，先取以便下载后立即比对）。
    let want_sha = fetch_sha256(&client, &sha256_url).await?;

    // 2. 流式下载 exe，更新进度。
    let bytes = download(&client, &exe_url, &progress).await?;

    // 3. 校验完整性。
    let got_sha = sha256_hex(&bytes);
    if !got_sha.eq_ignore_ascii_case(&want_sha) {
        bail!("下载文件校验失败：期望 {want_sha}，实际 {got_sha}");
    }

    // 4. 两步改名自替换。
    let current = std::env::current_exe().context("无法定位当前可执行文件路径")?;
    let new_path = sibling(&current, "new.exe");
    let old_path = sibling(&current, "old.exe");

    tokio::fs::write(&new_path, &bytes)
        .await
        .with_context(|| format!("写入新版本临时文件失败: {}", new_path.display()))?;

    // 清掉可能残留的旧文件，避免 rename 目标已存在。
    let _ = std::fs::remove_file(&old_path);
    std::fs::rename(&current, &old_path)
        .with_context(|| format!("重命名当前可执行文件失败: {}", current.display()))?;
    if let Err(e) = std::fs::rename(&new_path, &current) {
        // 回滚：把当前文件名还原，删除临时文件，保证下次仍能正常启动。
        let _ = std::fs::rename(&old_path, &current);
        let _ = std::fs::remove_file(&new_path);
        return Err(anyhow!("替换可执行文件失败: {e}"));
    }
    Ok(())
}

/// 启动同路径下（已被替换为新版的）可执行文件。
pub fn spawn_restart() -> Result<()> {
    let current = std::env::current_exe().context("无法定位当前可执行文件路径")?;
    std::process::Command::new(&current)
        .spawn()
        .with_context(|| format!("启动新版本失败: {}", current.display()))?;
    Ok(())
}

/// 删除上次更新残留的 `*.old.exe`（启动早期调用；幂等，失败不阻塞启动）。
pub fn cleanup_old_exe() {
    let Ok(current) = std::env::current_exe() else {
        return;
    };
    let old = sibling(&current, "old.exe");
    if old.exists() {
        match std::fs::remove_file(&old) {
            Ok(()) => tracing::info!("已清理旧版本可执行文件: {}", old.display()),
            Err(e) => tracing::warn!("清理旧版本失败（可能仍被占用，下次启动再试）: {e}"),
        }
    }
}

// ───────────────────────────── 内部辅助 ─────────────────────────────

/// 与 `current` 同目录、同文件名主干、指定后缀的兄弟路径（如 `VoxInk.old.exe`）。
fn sibling(current: &Path, suffix: &str) -> PathBuf {
    let dir = current.parent().unwrap_or_else(|| Path::new("."));
    let stem = current
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("VoxInk");
    dir.join(format!("{stem}.{suffix}"))
}

/// 下载 `.sha256` 文件并取其首个空白分隔 token（小写十六进制摘要）。
async fn fetch_sha256(client: &reqwest::Client, url: &str) -> Result<String> {
    let text = client
        .get(url)
        .send()
        .await
        .context("下载 SHA256 校验文件失败")?
        .error_for_status()
        .context("SHA256 校验文件返回错误状态")?
        .text()
        .await
        .context("读取 SHA256 校验文件失败")?;
    let token = text
        .split_whitespace()
        .next()
        .context("SHA256 校验文件为空")?;
    Ok(token.trim().to_ascii_lowercase())
}

/// 流式下载到内存，按 Content-Length 更新进度（0–100）。
async fn download(
    client: &reqwest::Client,
    url: &str,
    progress: &Arc<AtomicU8>,
) -> Result<Vec<u8>> {
    let resp = client
        .get(url)
        .send()
        .await
        .context("下载更新失败")?
        .error_for_status()
        .context("下载更新返回错误状态")?;
    let total = resp.content_length();
    let mut buf: Vec<u8> = match total {
        Some(n) => Vec::with_capacity(n as usize),
        None => Vec::new(),
    };
    let mut downloaded: u64 = 0;
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("下载数据流中断")?;
        downloaded += chunk.len() as u64;
        buf.extend_from_slice(&chunk);
        if let Some(total) = total
            && total > 0
        {
            let pct = ((downloaded.saturating_mul(100)) / total).min(100) as u8;
            progress.store(pct, Ordering::Relaxed);
        }
    }
    progress.store(100, Ordering::Relaxed);
    Ok(buf)
}

/// 计算字节切片的 SHA256，返回小写十六进制字符串。
fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut s = String::with_capacity(digest.len() * 2);
    for b in digest {
        use std::fmt::Write;
        let _ = write!(s, "{b:02x}");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_matches_known_vector() {
        // SHA256("abc") 的标准向量。
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn sibling_derives_old_and_new_names() {
        let cur = Path::new("C:/apps/VoxInk.exe");
        assert_eq!(sibling(cur, "old.exe"), Path::new("C:/apps/VoxInk.old.exe"));
        assert_eq!(sibling(cur, "new.exe"), Path::new("C:/apps/VoxInk.new.exe"));
    }
}
