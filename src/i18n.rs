//! 多语言（M11 任务 11.3）。
//!
//! 词典见 crate 根 `locales/app.yaml`；`rust_i18n::i18n!` 在 `main.rs` 注册（编译期嵌入）。
//! locale 是 rust-i18n 的全局状态，本应用文案与 gpui-component 内置文案共享同一 locale，
//! 故切换语言只需 `rust_i18n::set_locale` 一次即可同时影响二者。

/// 翻译键 → 当前语言字符串（键缺失时返回键本身）。
pub fn tr(key: &str) -> String {
    rust_i18n::t!(key).to_string()
}

/// 把配置里的语言值规范化为受支持的 locale（默认 zh-CN）。
pub fn normalize_locale(lang: &str) -> &'static str {
    match lang.trim().to_ascii_lowercase().as_str() {
        "en" | "en-us" | "english" => "en",
        _ => "zh-CN",
    }
}

/// 应用语言（设置全局 locale；同时影响 gpui-component 内置文案）。
pub fn apply_locale(lang: &str) {
    rust_i18n::set_locale(normalize_locale(lang));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translations_resolve_per_locale() {
        apply_locale("zh-CN");
        assert_eq!(tr("record.start"), "开始录音");
        assert_eq!(tr("settings.title"), "设置");
        apply_locale("en");
        assert_eq!(tr("record.start"), "Start recording");
        assert_eq!(tr("settings.title"), "Settings");
        apply_locale("zh-CN"); // 复位，避免影响其它用例
    }

    #[test]
    fn normalize_locale_maps_english_variants() {
        assert_eq!(normalize_locale("en"), "en");
        assert_eq!(normalize_locale("English"), "en");
        assert_eq!(normalize_locale("zh-CN"), "zh-CN");
        assert_eq!(normalize_locale("anything"), "zh-CN");
    }
}
