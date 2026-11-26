//! Инхибитор idle для wlroots (labwc, sway): своя wayland-поверхность с буфером
//! и `zwp_idle_inhibit_manager_v1.create_inhibitor`.
//!
//! Почему именно так: композитор учитывает инхибитор только для поверхности,
//! у которой привязан буфер и которая закоммичена; пустую поверхность без
//! буфера wlroots пропускает. Waybar вешает инхибитор на поверхность бара
//! (она видимая), нам нужна собственная невидимая: 1×1 прозрачный буфер.

use std::os::fd::{AsFd, FromRawFd};
use wayland_client::protocol::wl_buffer::WlBuffer;
use wayland_client::protocol::wl_compositor::WlCompositor;
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::protocol::wl_shm::WlShm;
use wayland_client::protocol::wl_shm_pool::WlShmPool;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols::wp::idle_inhibit::zv1::client::{
    zwp_idle_inhibit_manager_v1::ZwpIdleInhibitManagerV1, zwp_idle_inhibitor_v1::ZwpIdleInhibitorV1,
};

#[derive(Default)]
struct State {
    shm: Option<WlShm>,
    compositor: Option<WlCompositor>,
    inhibit_manager: Option<ZwpIdleInhibitManagerV1>,
}

macro_rules! noop_dispatch {
    ($($t:ty),* $(,)?) => {$(
        impl Dispatch<$t, ()> for State {
            fn event(
                _: &mut Self,
                _: &$t,
                _: <$t as wayland_client::Proxy>::Event,
                _: &(),
                _: &Connection,
                _: &QueueHandle<Self>,
            ) {}
        }
    )*};
}

noop_dispatch!(
    WlShm,
    WlShmPool,
    WlBuffer,
    WlCompositor,
    WlSurface,
    ZwpIdleInhibitManagerV1,
    ZwpIdleInhibitorV1
);

impl Dispatch<WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        registry: &WlRegistry,
        event: <WlRegistry as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        use wayland_client::protocol::wl_registry::Event;
        let Event::Global {
            name,
            interface,
            version,
        } = event
        else {
            return;
        };
        match interface.as_str() {
            "wl_shm" => {
                // wl_shm требует версию >= 1; 1 достаточно для create_pool/ARGB8888.
                state.shm = Some(registry.bind::<WlShm, _, _>(name, version.clamp(1, 1), qh, ()))
            }
            "wl_compositor" => {
                state.compositor =
                    Some(registry.bind::<WlCompositor, _, _>(name, version.min(4), qh, ()))
            }
            "zwp_idle_inhibit_manager_v1" => {
                state.inhibit_manager = Some(registry.bind::<ZwpIdleInhibitManagerV1, _, _>(
                    name,
                    version.min(1),
                    qh,
                    (),
                ))
            }
            _ => {}
        }
    }
}

/// Держит соединение и невидимую поверхность. Пока объект жив, инхибитор активен.
pub struct Inhibitor {
    conn: Connection,
    _queue: wayland_client::EventQueue<State>,
    // Держим поверхность и буфер живыми: без них инхибитор потеряет силу.
    _surface: WlSurface,
    _buffer: WlBuffer,
    _inhibitor: ZwpIdleInhibitorV1,
}

impl Inhibitor {
    /// Создать инхибитор. Ошибкой будет отсутствие wayland или протокола.
    pub fn new() -> Result<Self, String> {
        let conn = Connection::connect_to_env().map_err(|e| format!("wayland: {e}"))?;
        let mut queue = conn.new_event_queue();
        let qh = queue.handle();
        let mut state = State::default();

        // roundtrip() на Display диспатчит только очередь по умолчанию.
        // Реестр живёт в нашей очереди, поэтому диспатчим её явно.
        let _registry = conn.display().get_registry(&qh, ());
        conn.roundtrip()
            .map_err(|e| format!("wayland roundtrip: {e}"))?;
        conn.flush().map_err(|e| format!("wayland flush: {e}"))?;
        queue
            .roundtrip(&mut state)
            .map_err(|e| format!("wayland dispatch: {e}"))?;

        let shm = state.shm.take().ok_or("композитор не даёт wl_shm")?;
        let compositor = state
            .compositor
            .take()
            .ok_or("композитор не даёт wl_compositor")?;
        let manager = state
            .inhibit_manager
            .take()
            .ok_or("композитор не даёт zwp_idle_inhibit_manager_v1")?;

        let surface = compositor.create_surface(&qh, ());
        let buffer = transparent_1x1(&shm, &qh)?;
        surface.attach(Some(&buffer), 0, 0);
        surface.commit();

        let inhibitor = manager.create_inhibitor(&surface, &qh, ());
        manager.destroy();
        conn.flush().map_err(|e| e.to_string())?;
        // Без roundtrip композитор не увидит привязанный буфер и проигнорирует инхибитор.
        queue
            .roundtrip(&mut state)
            .map_err(|e| format!("wayland dispatch: {e}"))?;

        Ok(Self {
            conn,
            _queue: queue,
            _surface: surface,
            _buffer: buffer,
            _inhibitor: inhibitor,
        })
    }
}

impl Drop for Inhibitor {
    fn drop(&mut self) {
        self.conn.flush().ok();
    }
}

fn transparent_1x1(shm: &WlShm, qh: &QueueHandle<State>) -> Result<WlBuffer, String> {
    const SIZE: i64 = 4;
    let fd = unsafe { libc::memfd_create(c"tray-idle-inhibitor".as_ptr(), libc::MFD_CLOEXEC) };
    if fd < 0 {
        return Err("memfd_create: не удалось".into());
    }
    if unsafe { libc::ftruncate(fd, SIZE) } != 0 {
        unsafe { libc::close(fd) };
        return Err("ftruncate: не удалось".into());
    }
    let file = unsafe { std::fs::File::from_raw_fd(fd) };
    let pool = shm.create_pool(AsFd::as_fd(&file), SIZE as i32, qh, ());
    // Пиксель уже нулевой: ARGB8888 1×1, stride 4, полностью прозрачный.
    let buffer = pool.create_buffer(
        0,
        1,
        1,
        4,
        wayland_client::protocol::wl_shm::Format::Argb8888,
        qh,
        (),
    );
    pool.destroy();
    Ok(buffer)
}
