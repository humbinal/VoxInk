use std::io;
use std::process::Command;

fn main() -> io::Result<()> {
    // 注入构建期信息：Git 短哈希 + 构建时间（供「关于」与诊断导出使用，M11 任务 11.4）。
    let git_hash = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=VOXINK_GIT_HASH={git_hash}");

    // 构建时间戳（UTC，秒级）。无 chrono 依赖时用 SystemTime 计算 UTC 字符串。
    let build_time = build_timestamp();
    println!("cargo:rustc-env=VOXINK_BUILD_TIME={build_time}");

    // 仅在编译目标为 Windows 系统时运行该注入逻辑
    #[cfg(target_os = "windows")]
    {
        winresource::WindowsResource::new()
            .set_icon("assets/icon.ico") // 指定您的 ico 文件路径
            .compile()?;
    }
    Ok(())
}

/// 生成构建时间为 RFC3339 UTC（如 "2026-06-14T07:11:08Z"，无外部依赖）。
/// 运行期再按用户本地时区显示（见 diagnostics::build_time_display）。
fn build_timestamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0) as i64;
    // 民用历法换算（1970 起，UTC）。
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (h, mi, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

/// Howard Hinnant 的 days→(year,month,day) 算法。
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}
