//! 品牌图标的**纯绘制**（只依赖 tiny-skia，不引用 crate 内任何东西）。
//!
//! 设计为"唯一真源"：被运行期（`branding/mod.rs`）与编译期（`build.rs` 经 `#[path]`
//! 包含）共用，保证 exe 图标 / 任务栏 / 托盘三处视觉完全一致。
//!
//! 概念 C「语音气泡 + 波形」：teal 圆角方底 + 白色对话气泡（带小尾巴）+ 气泡内 teal 波形条。
//! 右下角可叠加状态徽标（不同颜色圆点）表示录制/转录状态。

use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Transform};

/// 图标状态（与 app 的录制/转录状态映射；见 branding/mod.rs）。
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum IconStatus {
    /// 非录制（无徽标）。
    Idle,
    /// 正在录制（离线：停止后再转写）——红。
    Recording,
    /// 正在录制 + 实时转录（流式）——橙。
    RecordingRealtime,
    /// 正在转录（停止后离线转写中）——蓝。
    Transcribing,
}

// 品牌色 teal ≈ hsl(172,58,43)。
const BRAND: (u8, u8, u8) = (46, 173, 156);
const WHITE: (u8, u8, u8) = (255, 255, 255);
// 状态徽标颜色。
// 状态徽标色与 src/theme.rs 的「转录模式开关」同色，提升系统一致性。
// 离线 = MODE_OFFLINE hsl(198, 32, 62)； 这里叠加到主题色上不明显, 固调亮了一些;
// 实时 = MODE_STREAMING hsl(340, 32, 62)；
// 转录 = 与二者饱和度相近的橙 hsl(35, 48, 62)。
// draw.rs 须独立（被 build.rs 复用）， 无法 import theme，故此处镜像其 RGB——改动 theme 这两色时请同步更新此处。
const MODE_OFFLINE_C: (u8, u8, u8) = (100, 153, 206);
const MODE_STREAMING_C: (u8, u8, u8) = (189, 127, 148);
const TRANSCRIBE_C: (u8, u8, u8) = (205, 166, 112);

/// 某状态的徽标颜色（Idle 无徽标）。
fn badge_color(status: IconStatus) -> Option<(u8, u8, u8)> {
    match status {
        IconStatus::Idle => None,
        // 离线录制 → 离线开关蓝；实时录制 → 实时开关色；转录中 → 一致饱和度的橙。
        IconStatus::Recording => Some(MODE_OFFLINE_C),
        IconStatus::RecordingRealtime => Some(MODE_STREAMING_C),
        IconStatus::Transcribing => Some(TRANSCRIBE_C),
    }
}

/// 渲染完整应用图标（teal 底 + 气泡波形 + 可选状态徽标），返回**直通** RGBA8。
pub fn render_icon_rgba(size: u32, status: IconStatus) -> Vec<u8> {
    to_straight_rgba(&build_icon_pixmap(size, status))
}

/// 绘制完整图标到 Pixmap（内部预乘）。
fn build_icon_pixmap(size: u32, status: IconStatus) -> Pixmap {
    let mut pm = Pixmap::new(size, size).expect("create pixmap");
    let s = size as f32;
    draw_squircle(&mut pm, s, 0.0, 0.0, s, BRAND);
    draw_bubble_with_wave(&mut pm, s);
    if let Some(c) = badge_color(status) {
        // 右下角徽标：直径 ≈ 40% 画布，带白色描边圈以保证在任意底色上可辨。
        let d = s * 0.40;
        let cx = s - d * 0.5 - s * 0.04;
        let cy = s - d * 0.5 - s * 0.04;
        fill_circle(&mut pm, cx, cy, d * 0.5 + s * 0.04, WHITE);
        fill_circle(&mut pm, cx, cy, d * 0.5, c);
    }
    pm
}

/// 仅渲染状态徽标圆点（透明底），用于任务栏 overlay。Idle 返回 None。
pub fn render_badge_rgba(size: u32, status: IconStatus) -> Option<Vec<u8>> {
    let c = badge_color(status)?;
    let mut pm = Pixmap::new(size, size).expect("create pixmap");
    let s = size as f32;
    let cx = s * 0.5;
    let r = s * 0.42;
    fill_circle(&mut pm, cx, cx, r + s * 0.06, WHITE);
    fill_circle(&mut pm, cx, cx, r, c);
    Some(to_straight_rgba(&pm))
}

/// teal 圆角方底（squircle 近似：圆角矩形）。
fn draw_squircle(pm: &mut Pixmap, full: f32, _x: f32, _y: f32, _w: f32, color: (u8, u8, u8)) {
    // 边距进一步收窄，让图标在任务栏/托盘里视觉占比更大、不显小。
    let margin = full * 0.02;
    let radius = full * 0.22;
    let rect = rounded_rect(
        margin,
        margin,
        full - margin * 2.0,
        full - margin * 2.0,
        radius,
    );
    fill(pm, &rect, color, 255);
}

/// 白色对话气泡（圆角矩形 + 左下尾巴）+ 气泡内 teal 波形条。
fn draw_bubble_with_wave(pm: &mut Pixmap, full: f32) {
    // 气泡整体加大；因尾巴在下、视觉重心偏上，故主体略微下移以求平衡。
    let bw = full * 0.62;
    let bh = full * 0.48;
    let bx = (full - bw) * 0.5;
    let by = full * 0.25;
    let r = bh * 0.34;

    // 气泡 + 尾巴合并为一条路径。
    let mut pb = PathBuilder::new();
    // 圆角矩形主体。
    pb.move_to(bx + r, by);
    pb.line_to(bx + bw - r, by);
    pb.quad_to(bx + bw, by, bx + bw, by + r);
    pb.line_to(bx + bw, by + bh - r);
    pb.quad_to(bx + bw, by + bh, bx + bw - r, by + bh);
    // 底边到尾巴根部。
    let tail_x = bx + bw * 0.30;
    pb.line_to(tail_x + full * 0.07, by + bh);
    // 尾巴尖（向左下）。
    pb.line_to(bx + bw * 0.16, by + bh + full * 0.12);
    pb.line_to(tail_x - full * 0.005, by + bh);
    pb.line_to(bx + r, by + bh);
    pb.quad_to(bx, by + bh, bx, by + bh - r);
    pb.line_to(bx, by + r);
    pb.quad_to(bx, by, bx + r, by);
    pb.close();
    if let Some(path) = pb.finish() {
        let mut paint = Paint {
            anti_alias: true,
            ..Default::default()
        };
        paint.set_color_rgba8(WHITE.0, WHITE.1, WHITE.2, 255);
        pm.fill_path(
            &path,
            &paint,
            FillRule::Winding,
            Transform::identity(),
            None,
        );
    }

    // 气泡内 teal 波形条（3 根：短、长、中——更少更大，小尺寸下也能看清）。
    let heights = [0.5_f32, 1.0, 0.72];
    let n = heights.len();
    let bar_w = bw * 0.12;
    let gap = (bw - bar_w * n as f32) / (n as f32 + 1.0);
    let max_h = bh * 0.52;
    let cy = by + bh * 0.5;
    for (i, &h) in heights.iter().enumerate() {
        let x = bx + gap + i as f32 * (bar_w + gap);
        let bh_i = max_h * h;
        let rect = rounded_rect(x, cy - bh_i * 0.5, bar_w, bh_i, bar_w * 0.5);
        fill(pm, &rect, BRAND, 255);
    }
}

/// 构建圆角矩形路径。
fn rounded_rect(x: f32, y: f32, w: f32, h: f32, r: f32) -> tiny_skia::Path {
    let r = r.min(w * 0.5).min(h * 0.5);
    let mut pb = PathBuilder::new();
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.quad_to(x + w, y, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.quad_to(x + w, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.quad_to(x, y + h, x, y + h - r);
    pb.line_to(x, y + r);
    pb.quad_to(x, y, x + r, y);
    pb.close();
    pb.finish().expect("rounded rect path")
}

fn fill(pm: &mut Pixmap, path: &tiny_skia::Path, c: (u8, u8, u8), a: u8) {
    let mut paint = Paint {
        anti_alias: true,
        ..Default::default()
    };
    paint.set_color_rgba8(c.0, c.1, c.2, a);
    pm.fill_path(path, &paint, FillRule::Winding, Transform::identity(), None);
}

fn fill_circle(pm: &mut Pixmap, cx: f32, cy: f32, r: f32, c: (u8, u8, u8)) {
    let mut pb = PathBuilder::new();
    pb.push_circle(cx, cy, r);
    if let Some(path) = pb.finish() {
        fill(pm, &path, c, 255);
    }
}

/// tiny-skia 内部是预乘 alpha；导出前解预乘为直通 RGBA8（托盘/ico 需要直通）。
fn to_straight_rgba(pm: &Pixmap) -> Vec<u8> {
    let mut out = Vec::with_capacity((pm.width() * pm.height() * 4) as usize);
    for p in pm.pixels() {
        let c = p.demultiply();
        out.push(c.red());
        out.push(c.green());
        out.push(c.blue());
        out.push(c.alpha());
    }
    out
}
