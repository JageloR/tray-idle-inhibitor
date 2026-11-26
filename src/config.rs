//! Конфигурация: системный файл `tray-idle-inhibitor/config.toml` из
//! `XDG_CONFIG_DIRS` (обычно `/etc/xdg`) и пользовательский из
//! `XDG_CONFIG_HOME` (обычно `~/.config`).
//!
//! Пользовательский файл переопределяет только указанные в нём ключи,
//! остальные берутся из системного или из значений по умолчанию. Флаги
//! командной строки сильнее конфига. Файл читается один раз при запуске.

use std::path::{Path, PathBuf};

const APP: &str = "tray-idle-inhibitor";
const DEFAULT_INTERVAL_SECS: u64 = 5;

#[derive(Debug, Clone)]
pub struct Config {
    pub auto: bool,
    pub start_on: bool,
    pub interval_secs: u64,
    pub color: Color,
}

#[derive(Debug, Clone)]
pub struct Color {
    pub mode: ColorMode,
    /// Имя темы LXQt для `mode = "theme"`; пусто — тема из `lxqt.conf`.
    pub theme: Option<String>,
    /// Цвет для `mode = "fixed"`.
    pub fg: (u8, u8, u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorMode {
    /// Цвет шрифта активной темы панели, иначе чёрный/белый по фону.
    Theme,
    /// Всегда чёрный/белый по фону панели, без темы.
    Auto,
    /// Всегда `fg`.
    Fixed,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            auto: false,
            start_on: false,
            interval_secs: DEFAULT_INTERVAL_SECS,
            color: Color {
                mode: ColorMode::Theme,
                theme: None,
                fg: (255, 255, 255),
            },
        }
    }
}

/// Слой из одного файла: присутствуют только указанные ключи.
#[derive(Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Patch {
    auto: Option<bool>,
    start_on: Option<bool>,
    interval_secs: Option<u64>,
    color: Option<ColorPatch>,
}

#[derive(Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ColorPatch {
    mode: Option<String>,
    theme: Option<String>,
    fg: Option<String>,
}

impl Config {
    fn apply(&mut self, patch: Patch) {
        if let Some(v) = patch.auto {
            self.auto = v;
        }
        if let Some(v) = patch.start_on {
            self.start_on = v;
        }
        if let Some(v) = patch.interval_secs {
            if v == 0 {
                eprintln!(
                    "tray-idle-inhibitor: config: interval_secs = 0, беру {DEFAULT_INTERVAL_SECS}"
                );
            } else {
                self.interval_secs = v;
            }
        }
        let Some(color) = patch.color else { return };
        if let Some(mode) = color.mode {
            match mode.trim().to_ascii_lowercase().as_str() {
                "theme" => self.color.mode = ColorMode::Theme,
                "auto" => self.color.mode = ColorMode::Auto,
                "fixed" => self.color.mode = ColorMode::Fixed,
                other => eprintln!(
                    "tray-idle-inhibitor: config: неизвестный color.mode «{other}» (theme|auto|fixed), оставляю {:?}",
                    self.color.mode
                ),
            }
        }
        if let Some(theme) = color.theme {
            let theme = theme.trim();
            self.color.theme = (!theme.is_empty()).then(|| theme.to_string());
        }
        if let Some(fg) = color.fg {
            match crate::theme::hex_color(&fg) {
                Some(rgb) => self.color.fg = rgb,
                None => eprintln!(
                    "tray-idle-inhibitor: config: color.fg «{fg}» не #rrggbb, оставляю прежний"
                ),
            }
        }
    }
}

fn config_file(dir: &Path) -> PathBuf {
    dir.join(APP).join("config.toml")
}

/// `$XDG_CONFIG_DIRS` по спецификации — через двоеточие, по умолчанию `/etc/xdg`.
fn system_file() -> Option<PathBuf> {
    let raw = std::env::var("XDG_CONFIG_DIRS")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "/etc/xdg".to_string());
    raw.split(':')
        .filter(|p| !p.trim().is_empty())
        .map(|d| config_file(Path::new(d)))
        .find(|p| p.is_file())
}

/// Пользовательский файл: только `$XDG_CONFIG_HOME` (по умолчанию `~/.config`),
/// без перебора `XDG_CONFIG_DIRS` — так требует спецификация.
fn user_file() -> PathBuf {
    if let Some(dir) = std::env::var("XDG_CONFIG_HOME")
        .ok()
        .filter(|v| !v.trim().is_empty())
    {
        return config_file(Path::new(&dir));
    }
    let home = std::env::var("HOME").map(PathBuf::from).unwrap_or_default();
    config_file(&home.join(".config"))
}

pub fn load() -> Config {
    let mut cfg = Config::default();
    if let Some(path) = system_file() {
        merge_file(&mut cfg, &path);
    }
    let user = user_file();
    if user.is_file() {
        merge_file(&mut cfg, &user);
    }
    cfg
}

fn merge_file(cfg: &mut Config, path: &Path) {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("tray-idle-inhibitor: {}: {e}", path.display());
            return;
        }
    };
    match toml::from_str::<Patch>(&text) {
        Ok(patch) => cfg.apply(patch),
        Err(e) => eprintln!(
            "tray-idle-inhibitor: {}: {e}; файл пропущен целиком",
            path.display()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn merged(text: &str) -> Config {
        let mut cfg = Config::default();
        cfg.apply(toml::from_str(text).expect("toml"));
        cfg
    }

    #[test]
    fn user_keys_override_defaults() {
        let cfg = merged(
            "auto = true\nstart_on = true\ninterval_secs = 30\n\
             [color]\nmode = \"fixed\"\nfg = \"#102030\"\n",
        );
        assert!(cfg.auto);
        assert!(cfg.start_on);
        assert_eq!(cfg.interval_secs, 30);
        assert_eq!(cfg.color.mode, ColorMode::Fixed);
        assert_eq!(cfg.color.fg, (16, 32, 48));
    }

    #[test]
    fn partial_patch_keeps_other_keys() {
        let mut cfg = merged("interval_secs = 9\n[color]\ntheme = \"system\"\n");
        cfg.apply(toml::from_str("auto = true").expect("toml"));
        assert!(cfg.auto);
        assert_eq!(cfg.interval_secs, 9);
        assert_eq!(cfg.color.theme.as_deref(), Some("system"));
        assert_eq!(cfg.color.mode, ColorMode::Theme);
    }

    #[test]
    fn bad_values_are_ignored() {
        let cfg = merged(
            "interval_secs = 0\n[color]\nmode = \"rainbow\"\nfg = \"white\"\ntheme = \"  \"\n",
        );
        assert_eq!(cfg.interval_secs, DEFAULT_INTERVAL_SECS);
        assert_eq!(cfg.color.mode, ColorMode::Theme);
        assert_eq!(cfg.color.fg, (255, 255, 255));
        assert_eq!(cfg.color.theme, None);
    }

    #[test]
    fn unknown_key_rejects_file() {
        assert!(toml::from_str::<Patch>("autoo = true").is_err());
    }
}
