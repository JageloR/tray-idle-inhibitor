//! Цвет индикатора — та же логика, что в `kb-indicator-rs`.
//!
//! Приоритет:
//!   1. цвет шрифта активной темы LXQt для панели (панель рисует им подписи,
//!      значит он читаем на её фоне): qss `Plugin > QToolButton` → `LXQtPanel` →
//!      `#BackgroundWidget` → палитра темы `window_text_color`;
//!   2. если шрифт не определён — чёрный/белый по яркости фона панели (порог 0.4);
//!   3. нет фона — белый.
//!
//! Фон панели берётся у **реально запущенной** панели: `lxqt-panel` или Waybar
//! (`XDG_CURRENT_DESKTOP` может описывать сессию, а не панель).

use crate::config::ColorMode;
use crate::icons::Rgb;
use std::path::{Path, PathBuf};

const FG_ON_LIGHT: Rgb = Rgb {
    r: 24,
    g: 24,
    b: 24,
};
const FG_ON_DARK: Rgb = Rgb {
    r: 255,
    g: 255,
    b: 255,
};
const PANEL_SELECTORS: &[&str] = &["window#waybar", "window.background", "*"];
/// Яркость, выше которой фон считается светлым.
const LIGHT_LUMA: f32 = 0.4;

fn rgb(t: (u8, u8, u8)) -> Rgb {
    Rgb {
        r: t.0,
        g: t.1,
        b: t.2,
    }
}

/// Относительная яркость по sRGB (WCAG 2.1).
fn luminance(r: u8, g: u8, b: u8) -> f32 {
    let ch = |v: u8| {
        let s = v as f32 / 255.0;
        if s <= 0.03928 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * ch(r) + 0.7152 * ch(g) + 0.0722 * ch(b)
}

fn config_path(rel: &str) -> PathBuf {
    let base = std::env::var("XDG_CONFIG_HOME")
        .ok()
        .filter(|p| !p.is_empty())
        .or_else(|| std::env::var("HOME").ok().map(|h| format!("{h}/.config")))
        .unwrap_or_default();
    PathBuf::from(base).join(rel)
}

fn parse_color(raw: &str) -> Option<(u8, u8, u8)> {
    let v = raw.trim().to_ascii_lowercase();
    if v == "transparent" || v == "inherit" || v == "inherit !important" {
        return None;
    }
    let v = v.trim_end_matches("!important").trim();
    if let Some(hex) = v.strip_prefix('#') {
        let digits: String = hex.chars().take(6).collect();
        if digits.len() == 3 {
            let mut out = [0u8; 3];
            for (i, c) in hex.chars().take(3).enumerate() {
                out[i] = c.to_digit(16)? as u8 * 17;
            }
            return Some((out[0], out[1], out[2]));
        }
        if digits.len() == 6 {
            let r = u8::from_str_radix(&digits[0..2], 16).ok()?;
            let g = u8::from_str_radix(&digits[2..4], 16).ok()?;
            let b = u8::from_str_radix(&digits[4..6], 16).ok()?;
            return Some((r, g, b));
        }
    }
    if let Some(args) = v.strip_prefix("rgba(").or_else(|| v.strip_prefix("rgb(")) {
        let nums: Vec<u8> = args
            .trim_end_matches(')')
            .split(',')
            .filter_map(|p| {
                p.trim()
                    .parse::<f32>()
                    .ok()
                    .map(|f| f.clamp(0.0, 255.0) as u8)
            })
            .collect();
        if nums.len() >= 3 {
            return Some((nums[0], nums[1], nums[2]));
        }
    }
    None
}

/// `@Variant(\0\0\0\x43\0\xff\xff…)` — сериализованный QColor из panel.conf.
/// Панель применяет только сплошной Rgb (cspec=1) с alpha=65535; Invalid (0)
/// означает «фон не задан».
fn parse_qt_variant_color(raw: &str) -> Option<(u8, u8, u8)> {
    let v = raw.trim();
    let inner = v.strip_prefix("@Variant(")?.strip_suffix(')')?;
    let mut bytes: Vec<u8> = Vec::new();
    let mut chars = inner.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            bytes.push(c as u8);
            continue;
        }
        match chars.next()? {
            '0' => bytes.push(0),
            'x' => {
                let hi = chars.next()?.to_digit(16)?;
                let lo = chars.next()?.to_digit(16)?;
                bytes.push((hi * 16 + lo) as u8);
            }
            'n' => bytes.push(b'\n'),
            't' => bytes.push(b'\t'),
            '\\' => bytes.push(b'\\'),
            other => bytes.push(other as u8),
        }
    }
    if bytes.len() < 13 || bytes[3] != 0x43 || bytes[4] != 1 {
        return None;
    }
    let ch16 = |off: usize| u16::from_be_bytes([bytes[off], bytes[off + 1]]);
    let (a, r, g, b) = (ch16(5), ch16(7), ch16(9), ch16(11));
    if a != 65535 {
        return None;
    }
    let to8 = |v: u16| (v / 257) as u8;
    Some((to8(r), to8(g), to8(b)))
}

fn ini_value(path: &Path, section: &str, key: &str) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut in_section = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            in_section = line[1..line.len() - 1].trim() == section;
            continue;
        }
        if !in_section {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            if k.trim() == key {
                return Some(v.trim().to_string());
            }
        }
    }
    None
}

fn parse_lxqt_color(raw: &str) -> Option<(u8, u8, u8)> {
    parse_qt_variant_color(raw).or_else(|| parse_color(raw))
}

fn xdg_data_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(h) = std::env::var("XDG_DATA_HOME") {
        if !h.is_empty() {
            dirs.push(PathBuf::from(h));
        }
    }
    if let Ok(raw) = std::env::var("XDG_DATA_DIRS") {
        for d in raw.split(':').filter(|p| !p.is_empty()) {
            dirs.push(PathBuf::from(d));
        }
    }
    if !dirs.iter().any(|d| d == &PathBuf::from("/usr/share")) {
        dirs.push(PathBuf::from("/usr/share"));
    }
    dirs
}

thread_local! {
    /// Тема из `[color] theme`; ставится один раз при старте, до создания трея.
    /// Нужна, чтобы не зависеть от активной темы панели.
    static CONFIG_THEME: std::cell::RefCell<Option<String>> =
        const { std::cell::RefCell::new(None) };
}

/// Задать тему для цвета иконки (`None` — активная тема панели).
pub fn set_theme(theme: Option<String>) {
    CONFIG_THEME.with(|t| *t.borrow_mut() = theme);
}

/// Имя темы LXQt: из конфига, иначе из `lxqt.conf`.
fn lxqt_theme_name() -> Option<String> {
    if let Some(t) = CONFIG_THEME.with(|t| t.borrow().clone()) {
        return Some(t);
    }
    let theme = ini_value(&config_path("lxqt/lxqt.conf"), "General", "theme")?;
    let theme = theme.trim().to_string();
    (!theme.is_empty()).then_some(theme)
}

fn lxqt_theme_panel_qss() -> Option<PathBuf> {
    let theme = lxqt_theme_name()?;
    let rel = Path::new("lxqt/themes").join(theme).join("lxqt-panel.qss");
    xdg_data_dirs()
        .into_iter()
        .map(|d| d.join(&rel))
        .find(|p| p.is_file())
}

/// Палитра темы лежит в `lxqt/palettes/<Имя>` с заглавной буквы, имя темы
/// может быть в нижнем регистре — ищем без учёта регистра.
fn lxqt_theme_palette_file() -> Option<PathBuf> {
    let theme = lxqt_theme_name()?.to_lowercase();
    for dir in xdg_data_dirs().into_iter().map(|d| d.join("lxqt/palettes")) {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().to_lowercase() == theme {
                return Some(entry.path());
            }
        }
    }
    None
}

fn palette_key(file: &Path, key: &str) -> Option<String> {
    ini_value(file, "Palette", key)
}

fn palette_role_to_key(role: &str) -> Option<&'static str> {
    match role.trim().to_ascii_lowercase().replace('-', "_").as_str() {
        "text" => Some("text_color"),
        "window" => Some("window_color"),
        "windowtext" => Some("window_text_color"),
        "base" => Some("base_color"),
        "highlight" => Some("highlight_color"),
        "highlightedtext" => Some("highlighted_text_color"),
        "tooltipbase" => Some("tooltip_base_color"),
        "tooltiptext" => Some("tooltip_text_color"),
        "link" => Some("link_color"),
        "linkvisited" => Some("link_visited_color"),
        _ => None,
    }
}

fn resolve_palette(role: &str) -> Option<(u8, u8, u8)> {
    let key = palette_role_to_key(role)?;
    if let Some(file) = lxqt_theme_palette_file() {
        if let Some(raw) = palette_key(&file, key) {
            if let Some(c) = parse_color(&raw) {
                return Some(c);
            }
        }
    }
    let lxqt = config_path("lxqt/lxqt.conf");
    let raw = ini_value(&lxqt, "Palette", key)?;
    parse_color(&raw)
}

/// Значение QSS-свойства: явный цвет или токен `palette(<роль>)`.
fn parse_qss_color(raw: &str) -> Option<(u8, u8, u8)> {
    let v = raw.trim();
    if let Some(inner) = v.strip_prefix("palette(") {
        return resolve_palette(inner.strip_suffix(')')?);
    }
    parse_color(v)
}

fn strip_css_comments(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let bytes = css.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i..].starts_with(b"/*") {
            match css[i + 2..].find("*/") {
                Some(end) => i += 2 + end + 2,
                None => break,
            }
        } else if bytes[i..].starts_with(b"//") {
            match css[i..].find('\n') {
                Some(end) => i += end,
                None => break,
            }
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    out
}

/// Первое значение свойства `prop` в правилах с указанными селекторами.
fn qss_color_from_selectors(qss: &str, prop: &str, selectors: &[&str]) -> Option<(u8, u8, u8)> {
    let clean = strip_css_comments(qss);
    for sel in selectors {
        // Селектор может уже включать открывающую скобку ("#BackgroundWidget {").
        let sel_has_brace = sel.trim_end().ends_with('{');
        let mut rest = clean.as_str();
        while let Some(pos) = rest.find(sel) {
            let after = &rest[pos + sel.len()..];
            let (body, consumed) = if sel_has_brace {
                let Some(end) = after.find('}') else { break };
                (&after[..end], end)
            } else {
                let Some(brace) = after.find('{') else { break };
                let Some(end) = after[brace..].find('}') else {
                    break;
                };
                (&after[brace + 1..brace + end], brace + end)
            };
            for decl in body.split(';') {
                let Some((k, v)) = decl.split_once(':') else {
                    continue;
                };
                if k.trim() == prop {
                    if let Some(c) = parse_qss_color(v) {
                        return Some(c);
                    }
                }
            }
            rest = &after[consumed..];
        }
    }
    None
}

/// Фоновый цвет из первого подходящего правила CSS (Waybar/swaybar).
fn background_from_css(css: &str) -> Option<(u8, u8, u8)> {
    let clean = strip_css_comments(css);
    for selector in PANEL_SELECTORS {
        let mut rest = clean.as_str();
        while let Some(pos) = rest.find(selector) {
            let after = &rest[pos + selector.len()..];
            let Some(brace) = after.find('{') else { break };
            let tail = &after[..brace];
            if !tail.contains(',') && !tail.trim().is_empty() && !tail.trim_start().starts_with(',')
            {
                rest = after;
                continue;
            }
            let Some(end) = after[brace..].find('}') else {
                break;
            };
            let body = &after[brace + 1..brace + end];
            for decl in body.split(';') {
                let Some((prop, value)) = decl.split_once(':') else {
                    continue;
                };
                let prop = prop.trim();
                if prop == "background" || prop == "background-color" {
                    if let Some(c) = parse_color(value) {
                        return Some(c);
                    }
                }
            }
            rest = &after[brace + end..];
        }
    }
    None
}

fn process_running(name: &str) -> bool {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return false;
    };
    for entry in entries.flatten() {
        let Ok(comm) = std::fs::read_to_string(entry.path().join("comm")) else {
            continue;
        };
        if comm.trim() == name {
            return true;
        }
    }
    false
}

fn desktop_contains(needle: &str) -> bool {
    std::env::var("XDG_CURRENT_DESKTOP")
        .map(|v| v.to_ascii_lowercase().contains(needle))
        .unwrap_or(false)
}

fn waybar_background() -> Option<(u8, u8, u8)> {
    let css = std::fs::read_to_string(config_path("waybar/style.css")).ok()?;
    background_from_css(&css)
}

fn swaybar_background() -> Option<(u8, u8, u8)> {
    let text = std::fs::read_to_string(config_path("sway/bar")).ok()?;
    text.lines().find_map(|line| {
        let value = line.trim_start().strip_prefix("background")?.trim();
        let hex = value.strip_prefix('#')?;
        parse_color(&format!("#{hex}"))
    })
}

/// Фон панели из конфигов. Приоритет — у реально запущенной панели.
fn panel_background() -> Option<(u8, u8, u8)> {
    let lxqt_running = process_running("lxqt-panel");
    let waybar_running = process_running("waybar");
    let prefer_lxqt = lxqt_running || (!waybar_running && desktop_contains("lxqt"));

    if prefer_lxqt {
        lxqt_background().or_else(waybar_background)
    } else {
        waybar_background()
            .or_else(swaybar_background)
            .or_else(lxqt_background)
    }
}

fn lxqt_theme_background() -> Option<(u8, u8, u8)> {
    let qss = std::fs::read_to_string(lxqt_theme_panel_qss()?).ok()?;
    qss_color_from_selectors(&qss, "background", &["#BackgroundWidget {"])
        .or_else(|| qss_color_from_selectors(&qss, "background-color", &["#BackgroundWidget {"]))
        .or_else(|| {
            lxqt_theme_palette_file()
                .and_then(|f| palette_key(&f, "window_color"))
                .and_then(|raw| parse_color(&raw))
        })
}

fn lxqt_background() -> Option<(u8, u8, u8)> {
    if let Some(c) = lxqt_theme_background() {
        return Some(c);
    }
    if let Some(raw) = ini_value(
        &config_path("lxqt/panel.conf"),
        "panel1",
        "background-color",
    ) {
        if let Some(c) = parse_lxqt_color(&raw) {
            return Some(c);
        }
    }
    let raw = ini_value(&config_path("lxqt/lxqt.conf"), "Palette", "window_color")?;
    parse_lxqt_color(&raw)
}

/// Цвет шрифта активной темы LXQt (он же цвет индикатора).
fn lxqt_font_color() -> Option<(u8, u8, u8)> {
    if let Some(qss) = lxqt_theme_panel_qss().and_then(|p| std::fs::read_to_string(p).ok()) {
        if let Some(c) = qss_color_from_selectors(&qss, "color", &["Plugin > QToolButton"]) {
            return Some(c);
        }
        if let Some(c) = qss_color_from_selectors(&qss, "color", &["LXQtPanel {"]) {
            return Some(c);
        }
        if let Some(c) = qss_color_from_selectors(&qss, "color", &["#BackgroundWidget {"]) {
            return Some(c);
        }
    }
    let raw = palette_key(&lxqt_theme_palette_file()?, "window_text_color")?;
    parse_color(&raw)
}

fn foreground_for(bg: (u8, u8, u8)) -> Rgb {
    if luminance(bg.0, bg.1, bg.2) > LIGHT_LUMA {
        FG_ON_LIGHT
    } else {
        FG_ON_DARK
    }
}

/// Публичный разбор `#rrggbb` (для `[color] fg` в конфиге).
pub fn hex_color(raw: &str) -> Option<(u8, u8, u8)> {
    let v = raw.trim();
    let hex = v.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let c = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    Some((c(0)?, c(2)?, c(4)?))
}

/// Цвет иконки: `Fixed` — из конфига, `Auto` — по фону панели,
/// `Theme` — цвет шрифта темы, иначе по фону панели.
pub fn icon_color(mode: ColorMode, fixed: (u8, u8, u8)) -> Rgb {
    match mode {
        ColorMode::Fixed => return rgb(fixed),
        ColorMode::Theme => {
            if let Some(fg) = lxqt_font_color() {
                return rgb(fg);
            }
        }
        ColorMode::Auto => {}
    }
    match panel_background() {
        Some(bg) => foreground_for(bg),
        None => FG_ON_DARK,
    }
}

/// Фон панели (для справки и `--dump-icons`).
pub fn panel_bg() -> Rgb {
    panel_background().map(rgb).unwrap_or(FG_ON_LIGHT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_variants() {
        assert_eq!(parse_color("#efefef"), Some((239, 239, 239)));
        assert_eq!(parse_color("#fff"), Some((255, 255, 255)));
        assert_eq!(parse_color("rgba(0, 0, 0, 50%)"), Some((0, 0, 0)));
        assert_eq!(parse_color("transparent"), None);
    }

    #[test]
    fn qt_qcolor_only_solid_rgb() {
        // Solid Rgb, alpha=65535, чёрный.
        let solid = r"@Variant(\0\0\0\x43\x01\xff\xff\0\0\0\0\0\0\0\0)";
        assert_eq!(parse_qt_variant_color(solid), Some((0, 0, 0)));
        // Invalid (cspec=0) — фон не задан.
        let invalid = r"@Variant(\0\0\0\x43\0\xff\xff\0\0\0\0\0\0\0\0)";
        assert_eq!(parse_qt_variant_color(invalid), None);
    }

    #[test]
    fn qss_selectors() {
        let qss = "/* c */ Plugin > QToolButton { color: #1a1a1a; }\n\
                   LXQtPanel #BackgroundWidget { background: #efefef; }";
        assert_eq!(
            qss_color_from_selectors(qss, "color", &["Plugin > QToolButton"]),
            Some((26, 26, 26))
        );
        assert_eq!(
            qss_color_from_selectors(qss, "background", &["#BackgroundWidget {"]),
            Some((239, 239, 239))
        );
    }

    #[test]
    fn foreground_threshold() {
        assert_eq!(foreground_for((239, 239, 239)), FG_ON_LIGHT);
        assert_eq!(foreground_for((48, 48, 48)), FG_ON_DARK);
        assert_eq!(foreground_for((0, 0, 0)), FG_ON_DARK);
    }

    #[test]
    fn luma_bounds() {
        assert!(luminance(0, 0, 0) < 0.01);
        assert!(luminance(255, 255, 255) > 0.99);
    }
}
