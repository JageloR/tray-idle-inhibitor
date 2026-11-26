mod config;
mod icons;
mod logind;
mod media;
mod theme;
mod wayland;

use icons::Rgb;
use ksni::menu::StandardItem;
use ksni::{Icon, MenuItem, ToolTip, Tray, TrayMethods};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};

const USAGE: &str = "\
tray-idle-inhibitor — инхибитор idle (swayidle) в системном трее

Использование:
  tray-idle-inhibitor [ОПЦИИ]

Опции (сильнее значений из конфига):
      --auto             включать инхибитор при воспроизведении медиа (MPRIS)
      --start-on         классическая чашка включённого инхибитора при старте
  -i, --interval <СЕК>   как часто проверять медиа и цвет панели (по умолчанию 5)
      --dump-icons       сохранить иконки в текущий каталог и выйти
  -h, --help             показать эту справку
  -V, --version          показать версию программы

Конфиг (необязательный, читается при запуске):
  /etc/xdg/tray-idle-inhibitor/config.toml     — системный (или первый из XDG_CONFIG_DIRS)
  ~/.config/tray-idle-inhibitor/config.toml    — пользовательский, перекрывает системный
  Ключи: auto, start_on, interval_secs, [color] mode = theme|auto|fixed, theme, fg.

Второй экземпляр не запускается: программа держит блокировку
  $XDG_RUNTIME_DIR/tray-idle-inhibitor.lock и при занятости сразу выходит.

Управление:
  ЛКМ по иконке — включить/выключить инхибитор
  ПКМ — меню (статус, переключить, выход)

Инхибитор держится сразу в двух местах:
  * wayland — своя поверхность с прозрачным буфером (wlroots учитывает только такие);
  * logind  — Inhibit idle, виден в systemd-inhibit --list.
";

/// Итоговые настройки запуска: конфиг, поверх него флаги командной строки.
struct Options {
    auto: bool,
    start_on: bool,
    interval: Duration,
    color: config::Color,
}

impl Options {
    fn color(&self) -> Rgb {
        theme::icon_color(self.color.mode, self.color.fg)
    }
}

/// `--help`/`--auto` поверх значений из файлов; `--interval` перекрывает
/// `interval_secs`, но не затирает `auto`/`start_on` из конфига.
fn base_options() -> Options {
    let cfg = config::load();
    theme::set_theme(cfg.color.theme.clone());
    Options {
        auto: cfg.auto,
        start_on: cfg.start_on,
        interval: Duration::from_secs(cfg.interval_secs),
        color: cfg.color,
    }
}

fn parse_args() -> Result<Option<Options>, String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut opts = base_options();
    let mut interval = None;
    let mut i = 0;
    while i < args.len() {
        let (flag, inline) = match args[i].split_once('=') {
            Some((f, v)) => (f, Some(v.to_string())),
            None => (args[i].as_str(), None),
        };
        let mut value = |name: &str| -> Result<String, String> {
            match &inline {
                Some(v) => Ok(v.clone()),
                None => {
                    i += 1;
                    args.get(i)
                        .cloned()
                        .ok_or_else(|| format!("{name}: нет значения"))
                }
            }
        };
        match flag {
            "--auto" => opts.auto = true,
            "--start-on" => opts.start_on = true,
            "-i" | "--interval" => {
                let v = value(flag)?;
                let s: u64 = v
                    .parse()
                    .map_err(|_| format!("{flag}: нужно число секунд"))?;
                if s < 1 {
                    return Err(format!("{flag}: минимум 1"));
                }
                interval = Some(Duration::from_secs(s));
            }
            "--dump-icons" => {
                let color = opts.color();
                std::fs::write("inhibit.png", icons::png(&icons::coffee(color)))
                    .map_err(|e| e.to_string())?;
                std::fs::write("sleep.png", icons::png(&icons::moon(color)))
                    .map_err(|e| e.to_string())?;
                println!(
                    "inhibit.png / sleep.png — цвет #{:02x}{:02x}{:02x} на панели #{:02x}{:02x}{:02x}, контраст {:.1}:1",
                    color.r, color.g, color.b,
                    theme::panel_bg().r, theme::panel_bg().g, theme::panel_bg().b,
                    theme::panel_bg().contrast(color)
                );
                return Ok(None);
            }
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(None);
            }
            "-V" | "--version" => {
                println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
                return Ok(None);
            }
            other => return Err(format!("неизвестная опция «{other}»\n\n{USAGE}")),
        }
        i += 1;
    }
    if let Some(interval) = interval {
        opts.interval = interval;
    }
    Ok(Some(opts))
}

enum Event {
    Toggle,
    /// Инхибитор нужно привести к состоянию: Some(true) — держать, false — отпустить.
    Sync,
    Quit,
}

struct TrayApp {
    /// Пользователь включил (ручной режим), независимо от медиа.
    manual: bool,
    /// Фактически держим ли инхибитор.
    active: bool,
    color: Rgb,
    tx: UnboundedSender<Event>,
}

impl TrayApp {
    fn icon(&self) -> Icon {
        if self.active {
            icons::coffee(self.color)
        } else {
            icons::moon(self.color)
        }
    }

    fn label(&self) -> &'static str {
        if self.active {
            "Инхибитор включён"
        } else {
            "Сон разрешён"
        }
    }
}

impl Tray for TrayApp {
    fn id(&self) -> String {
        "tray-idle-inhibitor".into()
    }

    fn title(&self) -> String {
        "Idle Inhibitor".into()
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        vec![self.icon()]
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            icon_name: String::new(),
            icon_pixmap: vec![self.icon()],
            title: "Idle Inhibitor".into(),
            description: format!("{}\nЛКМ — переключить", self.label()),
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        let _ = self.tx.send(Event::Toggle);
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        vec![
            MenuItem::Standard(StandardItem {
                label: format!("Статус: {}", self.label()),
                enabled: false,
                ..Default::default()
            }),
            MenuItem::Separator,
            MenuItem::Standard(StandardItem {
                label: if self.active {
                    "Разрешить сон".into()
                } else {
                    "Не давать уснуть".into()
                },
                activate: Box::new(|t: &mut Self| {
                    let _ = t.tx.send(Event::Toggle);
                }),
                ..Default::default()
            }),
            MenuItem::Separator,
            MenuItem::Standard(StandardItem {
                label: "Выход".into(),
                activate: Box::new(|t: &mut Self| {
                    let _ = t.tx.send(Event::Quit);
                }),
                ..Default::default()
            }),
        ]
    }
}

/// Оба инхибитора вместе: wayland — для композитора, logind — для swayidle
/// и systemd (они смотрят `BlockInhibited`). Отпускаем синхронно.
struct Hold {
    _wayland: Option<wayland::Inhibitor>,
    _logind: Option<logind::LogindInhibitor>,
}

async fn acquire() -> Option<Hold> {
    let wayland = match wayland::Inhibitor::new() {
        Ok(i) => Some(i),
        Err(e) => {
            eprintln!("tray-idle-inhibitor: wayland-инхибитор недоступен: {e}");
            None
        }
    };
    let logind = match logind::LogindInhibitor::acquire(
        "tray-idle-inhibitor",
        "Инхибитор сна включён пользователем",
    )
    .await
    {
        Ok(i) => Some(i),
        Err(e) => {
            eprintln!("tray-idle-inhibitor: logind-инхибитор недоступен: {e}");
            None
        }
    };
    if wayland.is_none() && logind.is_none() {
        return None;
    }
    Some(Hold {
        _wayland: wayland,
        _logind: logind,
    })
}

/// Итоговое желание держать инхибитор: ручной режим сильнее медиа,
/// а ручное выключение подавляет auto, пока медиа не остановится.
fn want_inhibit(manual: bool, playing: bool, audio_suppressed: bool) -> bool {
    manual || (playing && !audio_suppressed)
}

/// Что означает клик по иконке. Решение принимается по фактическому
/// состоянию, а не по флагу manual: при активном auto manual может быть
/// false, и `!manual` включал бы инхибитор вместо выключения.
/// Возвращает (manual, audio_suppressed).
fn toggle_state(active: bool, playing: bool) -> (bool, bool) {
    if active {
        (false, playing)
    } else {
        (true, false)
    }
}

/// Привести состояние к want: поднять оба инхибитора или снять.
async fn sync_holder(holder: &mut Option<Hold>, want: bool) {
    match (want, holder.is_some()) {
        (true, false) => *holder = acquire().await,
        (false, true) => *holder = None,
        _ => {}
    }
}

/// Единственный экземпляр: `flock` на `$XDG_RUNTIME_DIR/tray-idle-inhibitor.lock`.
/// Блокировку снимает ядро при завершении процесса, поэтому протухших файлов
/// не бывает, а путь не зависит от того, откуда запущен бинарник.
struct InstanceLock {
    /// Файл держим открытым: закрытие снимает блокировку.
    _file: std::fs::File,
}

fn lock_path() -> PathBuf {
    match std::env::var("XDG_RUNTIME_DIR")
        .ok()
        .filter(|d| !d.trim().is_empty())
    {
        Some(dir) => Path::new(&dir).join("tray-idle-inhibitor.lock"),
        // Общий /tmp: имя с uid, иначе пользователи блокировали бы друг друга.
        None => std::env::temp_dir().join(format!("tray-idle-inhibitor-{}.lock", unsafe {
            libc::geteuid()
        })),
    }
}

/// `Ok(Some)` — блокировка наша, `Ok(None)` — занято другим экземпляром,
/// `Err` — файл недоступен, тогда работаем без защиты от дублей.
fn acquire_lock(path: &Path) -> std::io::Result<Option<InstanceLock>> {
    use std::io::Write;
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::OpenOptionsExt;

    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(path)?;
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        let err = std::io::Error::last_os_error();
        return match err.kind() {
            std::io::ErrorKind::WouldBlock => Ok(None),
            _ => Err(err),
        };
    }
    let _ = file.set_len(0);
    let _ = writeln!(file, "{}", std::process::id());
    Ok(Some(InstanceLock { _file: file }))
}

/// pid из lock-файла — только для сообщения о дубле.
fn lock_owner(path: &Path) -> Option<String> {
    let pid = std::fs::read_to_string(path).ok()?;
    let pid = pid.trim().to_string();
    (!pid.is_empty()).then_some(pid)
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let Some(opts) = parse_args().map_err(|e| {
        eprintln!("tray-idle-inhibitor: {e}");
        std::process::exit(2);
    })?
    else {
        return Ok(());
    };

    let lock_path = lock_path();
    let _lock = match acquire_lock(&lock_path) {
        Ok(Some(lock)) => Some(lock),
        Ok(None) => {
            let pid = lock_owner(&lock_path)
                .map(|pid| format!(" (pid {pid})"))
                .unwrap_or_default();
            eprintln!("tray-idle-inhibitor: уже запущен{pid}, второй экземпляр не нужен");
            return Ok(());
        }
        Err(e) => {
            eprintln!(
                "tray-idle-inhibitor: {}: {e}; работаю без защиты от дублей",
                lock_path.display()
            );
            None
        }
    };

    let (tx, mut rx) = unbounded_channel();

    let manual_start = opts.start_on;
    let mut manual = manual_start;
    let mut holder: Option<Hold> = None;
    if manual_start {
        holder = acquire().await;
    }

    let tray = TrayApp {
        manual,
        active: holder.is_some(),
        color: opts.color(),
        tx: tx.clone(),
    };
    let handle = tray.spawn().await?;

    // Периодические события: проверка медиа (в --auto) и перечитывание цвета.
    {
        let tx = tx.clone();
        let interval = opts.interval;
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(interval);
            loop {
                tick.tick().await;
                if tx.send(Event::Sync).is_err() {
                    break;
                }
            }
        });
    }

    let mut shutdown = false;
    // Медиа в --auto держит инхибитор, пока что-то играет. Ручное выключение
    // приостанавливает auto, пока медиа не остановится: пауза или конец
    // воспроизведения возвращают auto в дело.
    let mut audio_suppressed = false;
    while !shutdown {
        let Some(ev) = rx.recv().await else { break };
        match ev {
            Event::Toggle => {
                let playing = opts.auto && media::any_playing().await;
                (manual, audio_suppressed) = toggle_state(holder.is_some(), playing);
                let want = want_inhibit(manual, playing, audio_suppressed);
                sync_holder(&mut holder, want).await;
                // если инхибитор не удалось поднять, ручное состояние не должно врать
                manual = holder.is_some();
            }
            Event::Sync => {
                let playing = opts.auto && media::any_playing().await;
                if !playing {
                    audio_suppressed = false;
                }
                let want = want_inhibit(manual, playing, audio_suppressed);
                sync_holder(&mut holder, want).await;
            }
            Event::Quit => shutdown = true,
        }

        let active = holder.is_some();
        let color = opts.color();
        let manual_now = manual;
        handle
            .update(move |t: &mut TrayApp| {
                t.active = active;
                t.color = color;
                t.manual = manual_now;
            })
            .await;
    }

    drop(holder);
    handle.shutdown().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_lock(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tii-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("tray-idle-inhibitor.lock")
    }

    #[test]
    fn manual_on_always_holds() {
        assert!(want_inhibit(true, false, false));
        assert!(want_inhibit(true, false, true));
        assert!(want_inhibit(true, true, true));
    }

    #[test]
    fn auto_holds_only_while_playing() {
        assert!(want_inhibit(false, true, false));
        assert!(!want_inhibit(false, false, false));
    }

    #[test]
    fn manual_off_suppresses_auto_until_playback_stops() {
        assert!(!want_inhibit(false, true, true));
        assert!(want_inhibit(false, true, false));
    }

    #[test]
    fn toggle_uses_actual_state_not_manual_flag() {
        // Инхибитор держит auto: клик должен выключить и приостановить auto.
        assert_eq!(toggle_state(true, true), (false, true));
        assert_eq!(toggle_state(true, false), (false, false));
        // Инхибитор выключен: клик включает ручной режим.
        assert_eq!(toggle_state(false, true), (true, false));
        assert_eq!(toggle_state(false, false), (true, false));
    }

    #[test]
    fn second_instance_is_rejected() {
        let path = temp_lock("lock");
        let first = acquire_lock(&path)
            .expect("открытие lock-файла")
            .expect("первый экземпляр берёт блокировку");
        assert!(
            matches!(acquire_lock(&path), Ok(None)),
            "второй экземпляр не должен её взять"
        );
        let pid = std::process::id().to_string();
        assert_eq!(lock_owner(&path).as_deref(), Some(pid.as_str()));
        drop(first);
        assert!(
            matches!(acquire_lock(&path), Ok(Some(_))),
            "после завершения блокировка свободна"
        );
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn missing_directory_is_an_error_not_busy() {
        let dir = std::env::temp_dir().join(format!("tii-gone-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            acquire_lock(&dir.join("tray-idle-inhibitor.lock")).is_err(),
            "нет каталога — это ошибка, а не «занято»"
        );
    }
}
