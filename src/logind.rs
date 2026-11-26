//! Инхибитор idle через logind: `org.freedesktop.login1.Manager.Inhibit("idle", …)`.
//!
//! Зачем в дополнение к wayland-инхибитору: `swayidle` (и `systemd-inhibit`)
//! определяют «кто-то не даёт уснуть» не по wayland-поверхности, а по свойству
//! `BlockInhibited` из logind. Инхибитор живёт, пока открыт файловый дескриптор,
//! который вернул D-Bus; закрыли fd — инхибитор снят.

use std::os::fd::{AsRawFd, OwnedFd};
use zbus::zvariant::OwnedFd as ZOwnedFd;

/// Держит logind-инхибитор. Пока объект жив, `BlockInhibited` содержит `idle`.
pub struct LogindInhibitor {
    /// fd держим только ради удержания блокировки.
    fd: OwnedFd,
}

// OwnedFd — Send; соединение не храним, оно нужно только на время вызова.
unsafe impl Send for LogindInhibitor {}

impl LogindInhibitor {
    pub async fn acquire(who: &str, why: &str) -> Result<Self, String> {
        let conn = zbus::Connection::system()
            .await
            .map_err(|e| format!("system bus: {e}"))?;

        let reply = conn
            .call_method(
                Some("org.freedesktop.login1"),
                "/org/freedesktop/login1",
                Some("org.freedesktop.login1.Manager"),
                "Inhibit",
                &("idle", who, why, "block"),
            )
            .await
            .map_err(|e| format!("Inhibit: {e}"))?;

        let fd: ZOwnedFd = reply
            .body()
            .deserialize()
            .map_err(|e| format!("Inhibit fd: {e}"))?;
        let owned: OwnedFd = fd.into();
        // sanity: дескриптор должен быть валиден, иначе блокировка не держится
        if unsafe { libc::fcntl(owned.as_raw_fd(), libc::F_GETFD) } < 0 {
            return Err("Inhibit вернул невалидный fd".into());
        }
        Ok(Self { fd: owned })
    }
}

impl Drop for LogindInhibitor {
    fn drop(&mut self) {
        // Закрытие fd и есть снятие блокировки; флаг лишь для читаемости намерения.
        let _ = self.fd.as_raw_fd();
    }
}
