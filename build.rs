use std::io;
use std::process::Command;

// 复用运行期同一份纯绘制（只依赖 tiny-skia），保证 exe 图标 / 任务栏 / 托盘 / 主界面 logo 一致。
// build 脚本仅用其中一部分，故对未用项放行 dead_code。
#[allow(dead_code)]
#[path = "src/branding/draw.rs"]
mod draw;

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

    println!("cargo:rerun-if-changed=src/branding/draw.rs");
    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR");

    // 主界面标题栏 logo（全平台）：按显示尺寸(18px)的 2 倍渲染，避免大幅缩放产生锯齿。
    write_logo_png(&std::path::Path::new(&out_dir).join("voxink_logo.png"), 36);

    // 程序化生成多尺寸 .ico（纯 Rust：tiny-skia 绘制 + ico 编码），再用 winresource 嵌入。
    #[cfg(target_os = "windows")]
    {
        let ico_path = std::path::Path::new(&out_dir).join("voxink_icon.ico");
        write_app_ico(&ico_path);
        winresource::WindowsResource::new()
            .set_icon(ico_path.to_str().expect("ico path utf8"))
            .compile()?;
    }
    Ok(())
}

/// 渲染 Idle 品牌图标为 PNG（直通 RGBA8 → png 编码）。
fn write_logo_png(path: &std::path::Path, size: u32) {
    let rgba = draw::render_icon_rgba(size, draw::IconStatus::Idle);
    let file = std::fs::File::create(path).expect("create logo png");
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), size, size);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().expect("png header");
    w.write_image_data(&rgba).expect("png data");
}

/// 生成多尺寸应用图标（Idle 态，无状态徽标）并写入 .ico。
#[cfg(target_os = "windows")]
fn write_app_ico(path: &std::path::Path) {
    let mut dir = ico::IconDir::new(ico::ResourceType::Icon);
    for size in [16u32, 24, 32, 48, 64, 128, 256] {
        let rgba = draw::render_icon_rgba(size, draw::IconStatus::Idle);
        let image = ico::IconImage::from_rgba_data(size, size, rgba);
        dir.add_entry(ico::IconDirEntry::encode(&image).expect("encode ico entry"));
    }
    let file = std::fs::File::create(path).expect("create ico");
    dir.write(file).expect("write ico");
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
