//! Автовключение инхибитора по видео, как в GNOME Caffeine: смотрим MPRIS
//! (`PlaybackStatus == Playing`) у всех плееров через `playerctl`.
//!
//! Своими силами с zbus было бы больше кода и подписок на появление плееров;
//! ``playerctl`` — одна маленькая команда и он уже стоит в системе.

use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::process::Command;

/// Причину отсутствия playerctl печатаем один раз: опрос идёт каждые
/// несколько секунд, и повторять одно и то же в stderr бессмысленно.
static PLAYERCTL_WARNED: AtomicBool = AtomicBool::new(false);

/// Есть ли воспроизведение хоть у одного плеера.
pub async fn any_playing() -> bool {
    let out = Command::new("playerctl")
        .args(["--all-players", "status"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .await;
    match out {
        Ok(o) => {
            let text = String::from_utf8_lossy(&o.stdout).to_lowercase();
            text.lines().any(|l| l.trim() == "playing")
        }
        Err(e) => {
            if !PLAYERCTL_WARNED.swap(true, Ordering::Relaxed) {
                eprintln!(
                    "tray-idle-inhibitor: playerctl не запускается ({e}); \
--auto не сможет включать инхибитор по медиа"
                );
            }
            false
        }
    }
}
