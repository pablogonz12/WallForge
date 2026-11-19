mod audio;
mod egl;
mod gfx;
mod video;

use egl::*;
use gfx::{Gfx, Scaling};
use std::collections::VecDeque;
use std::ffi::c_void;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::{wl_callback, wl_compositor, wl_output, wl_region, wl_registry, wl_surface};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols::wp::fractional_scale::v1::client::{wp_fractional_scale_manager_v1, wp_fractional_scale_v1};
use wayland_protocols::wp::viewporter::client::{wp_viewport, wp_viewporter};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

static STOP: AtomicBool = AtomicBool::new(false);
extern "C" fn on_signal(_: i32) { STOP.store(true, Ordering::SeqCst); }

struct Args {
    output: String,
    bg: PathBuf,
    scaling: Scaling,
    fps: f64,
    layer: zwlr_layer_shell_v1::Layer,
    duration: Option<f64>,
    force_sw: bool,
    volume: u32,
    silent: bool,
}

fn usage() -> ! {
    eprintln!("usage: wallforge-render --output NAME --bg <dir|video> [--scaling fill|fit|stretch] [--fps N] \
               [--layer background|bottom|top|overlay] [--silent] [--volume N] [--duration SECS] [--software]");
    std::process::exit(2)
}

fn parse_args() -> Args {
    let mut a = Args { output: String::new(), bg: PathBuf::new(), scaling: Scaling::Fill, fps: 30.0,
        layer: zwlr_layer_shell_v1::Layer::Background, duration: None, force_sw: false, volume: 100, silent: false };
    let mut it = std::env::args().skip(1);
    while let Some(k) = it.next() {
        let mut val = || it.next().unwrap_or_else(|| usage());
        match k.as_str() {
            "--output" => a.output = val(),
            "--bg" => a.bg = PathBuf::from(val()),
            "--scaling" => a.scaling = match val().as_str() { "fill" => Scaling::Fill, "fit" => Scaling::Fit, "stretch" => Scaling::Stretch, _ => usage() },
            "--fps" => a.fps = val().parse().unwrap_or_else(|_| usage()),
            "--layer" => a.layer = match val().as_str() {
                "background" => zwlr_layer_shell_v1::Layer::Background, "bottom" => zwlr_layer_shell_v1::Layer::Bottom,
                "top" => zwlr_layer_shell_v1::Layer::Top, "overlay" => zwlr_layer_shell_v1::Layer::Overlay, _ => usage() },
            "--duration" => a.duration = Some(val().parse().unwrap_or_else(|_| usage())),
            "--volume" => a.volume = val().parse::<u32>().unwrap_or_else(|_| usage()).min(100),
            "--silent" => a.silent = true,
            "--software" => a.force_sw = true,
            _ => usage(),
        }
    }
    if a.output.is_empty() || a.bg.as_os_str().is_empty() || a.fps <= 0.0 { usage() }
    a
}

fn resolve_video(bg: &PathBuf) -> Result<String, String> {
    if bg.is_dir() {
        let txt = std::fs::read_to_string(bg.join("project.json")).map_err(|e| format!("project.json: {e}"))?;
        let j: serde_json::Value = serde_json::from_str(&txt).map_err(|e| format!("project.json: {e}"))?;
        let ty = j["type"].as_str().unwrap_or("").to_lowercase();
        if ty != "video" { return Err(format!("unsupported wallpaper type '{ty}' (only video)")); }
        let f = j["file"].as_str().ok_or("project.json has no \"file\"")?;
        Ok(bg.join(f).to_string_lossy().into_owned())
    } else { Ok(bg.to_string_lossy().into_owned()) }
}

struct App {
    outputs: Vec<(wl_output::WlOutput, Option<String>, i32)>,
    configured: bool,
    closed: bool,
    size: (i32, i32),
    resized: bool,
    cb_ready: bool,
    frac: u32, // preferred fractional scale in 1/120 (0 = unknown)
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for App {
    fn event(_: &mut Self, _: &wl_registry::WlRegistry, _: wl_registry::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<wp_viewporter::WpViewporter, ()> for App {
    fn event(_: &mut Self, _: &wp_viewporter::WpViewporter, _: wp_viewporter::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<wp_viewport::WpViewport, ()> for App {
    fn event(_: &mut Self, _: &wp_viewport::WpViewport, _: wp_viewport::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1, ()> for App {
    fn event(_: &mut Self, _: &wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1, _: wp_fractional_scale_manager_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<wp_fractional_scale_v1::WpFractionalScaleV1, ()> for App {
    fn event(s: &mut Self, _: &wp_fractional_scale_v1::WpFractionalScaleV1, e: wp_fractional_scale_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let wp_fractional_scale_v1::Event::PreferredScale { scale } = e { if scale != s.frac { s.frac = scale; s.resized = true; } }
    }
}
impl Dispatch<wl_compositor::WlCompositor, ()> for App {
    fn event(_: &mut Self, _: &wl_compositor::WlCompositor, _: wl_compositor::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<wl_region::WlRegion, ()> for App {
    fn event(_: &mut Self, _: &wl_region::WlRegion, _: wl_region::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<wl_surface::WlSurface, ()> for App {
    fn event(_: &mut Self, _: &wl_surface::WlSurface, _: wl_surface::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<zwlr_layer_shell_v1::ZwlrLayerShellV1, ()> for App {
    fn event(_: &mut Self, _: &zwlr_layer_shell_v1::ZwlrLayerShellV1, _: zwlr_layer_shell_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<wl_callback::WlCallback, ()> for App {
    fn event(s: &mut Self, _: &wl_callback::WlCallback, e: wl_callback::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let wl_callback::Event::Done { .. } = e { s.cb_ready = true; }
    }
}
impl Dispatch<wl_output::WlOutput, ()> for App {
    fn event(s: &mut Self, o: &wl_output::WlOutput, e: wl_output::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        let Some(ent) = s.outputs.iter_mut().find(|x| &x.0 == o) else { return };
        match e {
            wl_output::Event::Name { name } => ent.1 = Some(name),
            wl_output::Event::Scale { factor } => ent.2 = factor,
            _ => {}
        }
    }
}
impl Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, ()> for App {
    fn event(s: &mut Self, l: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, e: zwlr_layer_surface_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match e {
            zwlr_layer_surface_v1::Event::Configure { serial, width, height } => {
                l.ack_configure(serial);
                let sz = (width as i32, height as i32);
                if sz != s.size { s.size = sz; s.resized = true; }
                s.configured = true;
            }
            zwlr_layer_surface_v1::Event::Closed => s.closed = true,
            _ => {}
        }
    }
}

fn main() {
    let args = parse_args();
    let video_path = resolve_video(&args.bg).unwrap_or_else(|e| { eprintln!("wallforge-render: {e}"); std::process::exit(1) });
    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = on_signal as *const () as usize;
        libc::sigaction(libc::SIGTERM, &sa, std::ptr::null_mut());
        libc::sigaction(libc::SIGINT, &sa, std::ptr::null_mut());
    }

    let conn = Connection::connect_to_env().unwrap_or_else(|e| { eprintln!("wayland connect: {e}"); std::process::exit(1) });
    let (globals, mut queue) = registry_queue_init::<App>(&conn).expect("registry");
    let qh = queue.handle();
    let mut app = App { outputs: vec![], configured: false, closed: false, size: (0, 0), resized: false, cb_ready: true, frac: 0 };
    let compositor: wl_compositor::WlCompositor = globals.bind(&qh, 4..=6, ()).expect("wl_compositor");
    let shell: zwlr_layer_shell_v1::ZwlrLayerShellV1 = globals.bind(&qh, 1..=4, ()).expect("zwlr_layer_shell_v1 (compositor lacks layer-shell)");
    let outs: Vec<(u32, u32)> = globals.contents().with_list(|l| l.iter().filter(|g| g.interface == "wl_output").map(|g| (g.name, g.version)).collect());
    for (name, ver) in outs {
        let o = globals.registry().bind::<wl_output::WlOutput, _, _>(name, ver.min(4), &qh, ());
        app.outputs.push((o, None, 1));
    }
    queue.roundtrip(&mut app).unwrap();
    queue.roundtrip(&mut app).unwrap();
    let Some((out, _, scale)) = app.outputs.iter().find(|o| o.1.as_deref() == Some(args.output.as_str())).cloned() else {
        let names: Vec<_> = app.outputs.iter().filter_map(|o| o.1.clone()).collect();
        eprintln!("wallforge-render: output {} not found (have: {})", args.output, names.join(", "));
        std::process::exit(1)
    };

    let surface = compositor.create_surface(&qh, ());
    let empty = compositor.create_region(&qh, ());
    surface.set_input_region(Some(&empty));
    empty.destroy();
    let layer = shell.get_layer_surface(&surface, Some(&out), args.layer, "wallforge".into(), &qh, ());
    layer.set_anchor(zwlr_layer_surface_v1::Anchor::Top | zwlr_layer_surface_v1::Anchor::Bottom | zwlr_layer_surface_v1::Anchor::Left | zwlr_layer_surface_v1::Anchor::Right);
    layer.set_exclusive_zone(-1);
    layer.set_size(0, 0);
    layer.set_keyboard_interactivity(zwlr_layer_surface_v1::KeyboardInteractivity::None);
    let vpr: Option<wp_viewporter::WpViewporter> = globals.bind(&qh, 1..=1, ()).ok();
    let fsm: Option<wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1> = globals.bind(&qh, 1..=1, ()).ok();
    let viewport = vpr.as_ref().map(|v| v.get_viewport(&surface, &qh, ()));
    let _fs = match (&fsm, &viewport) { (Some(m), Some(_)) => Some(m.get_fractional_scale(&surface, &qh, ())), _ => None };
    if _fs.is_none() && scale > 1 { surface.set_buffer_scale(scale); }
    surface.commit();
    while !app.configured && !app.closed {
        queue.blocking_dispatch(&mut app).unwrap();
    }
    if app.closed { eprintln!("layer surface closed"); return; }

    // ---- EGL ----
    if _fs.is_some() { let _ = queue.roundtrip(&mut app); }
    app.resized = false;
    eprintln!("wallforge-render: fractional-scale={} viewporter={} preferred={}", fsm.is_some(), vpr.is_some(), app.frac as f64 / 120.0);
    // buffer size in physical pixels: fractional scale (wp-viewporter) or integer output scale
    let phys = |logical: (i32, i32), frac: u32| -> (i32, i32) {
        if _fs.is_some() && frac > 0 {
            (((logical.0 as i64 * frac as i64 + 60) / 120) as i32, ((logical.1 as i64 * frac as i64 + 60) / 120) as i32)
        } else { (logical.0 * scale, logical.1 * scale) }
    };
    let (mut bw, mut bh) = phys(app.size, app.frac);
    if let Some(v) = &viewport { if _fs.is_some() && app.frac > 0 { v.set_destination(app.size.0, app.size.1); } }
    let wl_display = conn.backend().display_ptr() as *mut c_void;
    let (dpy, ctx, esurf, win);
    unsafe {
        dpy = eglGetPlatformDisplay(EGL_PLATFORM_WAYLAND_KHR, wl_display, std::ptr::null());
        let (mut ma, mut mi) = (0, 0);
        assert!(eglInitialize(dpy, &mut ma, &mut mi) != 0, "eglInitialize");
        eglBindAPI(EGL_OPENGL_ES_API);
        let ca = [EGL_SURFACE_TYPE, EGL_WINDOW_BIT, EGL_RENDERABLE_TYPE, EGL_OPENGL_ES3_BIT,
                  EGL_RED_SIZE, 8, EGL_GREEN_SIZE, 8, EGL_BLUE_SIZE, 8, EGL_ALPHA_SIZE, 0, EGL_NONE];
        let mut cfg: EGLConfig = std::ptr::null_mut(); let mut n = 0;
        assert!(eglChooseConfig(dpy, ca.as_ptr(), &mut cfg, 1, &mut n) != 0 && n > 0, "no EGL config");
        let cx = [EGL_CONTEXT_CLIENT_VERSION, 3, EGL_NONE];
        ctx = eglCreateContext(dpy, cfg, std::ptr::null_mut(), cx.as_ptr());
        assert!(!ctx.is_null(), "eglCreateContext");
        win = wayland_egl::WlEglSurface::new(surface.id(), bw, bh).expect("wl_egl_window");
        esurf = eglCreatePlatformWindowSurface(dpy, cfg, win.ptr() as *mut c_void, std::ptr::null());
        assert!(!esurf.is_null(), "eglCreateWindowSurface");
        assert!(eglMakeCurrent(dpy, esurf, esurf, ctx) != 0, "eglMakeCurrent");
        eglSwapInterval(dpy, 0);
    }
    let mut gfx = Gfx::new(dpy, args.scaling);
    let mut dec = video::Decoder::open(&video_path, !args.force_sw, args.fps).unwrap_or_else(|e| { eprintln!("wallforge-render: {e}"); std::process::exit(1) });
    eprintln!("wallforge-render: {}x{} @{:.2}fps decode={} dmabuf={} -> {}x{} @{} cap",
        dec.width, dec.height, dec.fps, if dec.hw { "vaapi" } else { "software" }, gfx.can_dmabuf, bw, bh, args.fps);

    let audio = if args.silent { None } else { audio::Audio::start(&video_path, args.volume) };
    if audio.is_some() { eprintln!("wallforge-render: audio on (volume {})", args.volume); }
    let mut a_paused = true;
    let test_pause = std::env::var_os("WALLFORGE_TEST_PAUSE").is_some();
    let mut held_cb = false;
    let period = Duration::from_secs_f64(1.0 / args.fps);
    let start = Instant::now();
    let (mut base_wall, mut base_pts, mut last_render) = (Instant::now(), 0.0f64, Instant::now());
    let mut next_due = Instant::now();
    let mut ring: VecDeque<video::Frame> = VecDeque::new();
    let mut shown = 0u64;
    let mut first = true;
    let mut dirty = true;
    let mut no_frame_polls = 0;

    while !STOP.load(Ordering::SeqCst) && !app.closed {
        if let Some(d) = args.duration { if start.elapsed().as_secs_f64() >= d { break; } }
        if let Err(e) = queue.dispatch_pending(&mut app) { eprintln!("wallforge-render: wayland error ({e}); exiting"); break; }
        if test_pause {
            // test hook: pretend the compositor sends no frame callbacks between 3 s and 7 s
            let t = start.elapsed().as_secs_f64();
            if t >= 3.0 && t < 7.0 { if app.cb_ready { app.cb_ready = false; held_cb = true; } }
            else if held_cb { app.cb_ready = true; held_cb = false; }
        }
        if app.resized {
            app.resized = false;
            let (nw, nh) = phys(app.size, app.frac);
            bw = nw; bh = nh;
            if let Some(v) = &viewport { if _fs.is_some() && app.frac > 0 { v.set_destination(app.size.0, app.size.1); } }
            win.resize(bw, bh, 0, 0);
            eprintln!("wallforge-render: surface {}x{} logical -> {}x{} physical (scale {})", app.size.0, app.size.1, bw, bh, if app.frac > 0 { app.frac as f64 / 120.0 } else { scale as f64 });
            dirty = true;
        }
        let now = Instant::now();
        if (app.cb_ready && now >= next_due) || (dirty && !first) {
            if now.duration_since(last_render) > Duration::from_millis(500) && !first {
                base_wall = now; // woke from pause: continue where we stopped, do not fast-forward
                base_pts = ring.back().map(|f| f.pts).unwrap_or(0.0);
            }
            last_render = now;
            let target = base_pts + now.duration_since(base_wall).as_secs_f64();
            let mut newf = dec.advance(if first { 0.0 } else { target });
            if newf.is_none() && dec.hw && shown == 0 {
                no_frame_polls += 1;
                if no_frame_polls > 3 {
                    eprintln!("wallforge-render: VAAPI produced no frames, falling back to software");
                    dec = video::Decoder::open(&video_path, false, args.fps).expect("sw reopen");
                    base_wall = Instant::now(); base_pts = 0.0;
                }
            }
            let mut changed = false;
            if let Some(mut f) = newf.take() {
                if let Err(e) = gfx.upload(&f) {
                    eprintln!("wallforge-render: hardware import failed ({e}); falling back to software decode");
                    dec = video::Decoder::open(&video_path, false, args.fps).expect("sw reopen");
                    base_wall = Instant::now(); base_pts = 0.0; ring.clear();
                    f = dec.advance(0.0).expect("sw first frame");
                    gfx.upload(&f).expect("sw upload");
                }
                let f_pos = f.pos;
                ring.push_back(f);
                while ring.len() > 1 { ring.pop_front(); }
                if let Some(a) = &audio { a.set_video_pos(f_pos); }
                changed = true; shown += 1;
            }
            if changed || dirty {
                if app.cb_ready { surface.frame(&qh, ()); app.cb_ready = false; }
                gfx.draw(bw, bh);
                unsafe { eglSwapBuffers(dpy, esurf); }
                dirty = false;
                if first { first = false; base_wall = Instant::now(); base_pts = ring.back().map(|f| f.pts).unwrap_or(0.0); }
            }
            next_due += period;
            if next_due < now { next_due = now; }
        }
        if let Some(a) = &audio {
            let want = !first && !app.cb_ready && last_render.elapsed() > Duration::from_millis(400);
            let want = want || first;
            if want != a_paused { a_paused = want; a.set_paused(want); }
        }
        if let Err(e) = conn.flush() { eprintln!("wallforge-render: lost connection to the compositor ({e}); exiting"); break; }
        if let Some(guard) = queue.prepare_read() {
            let mut fds = [libc::pollfd { fd: std::os::fd::AsRawFd::as_raw_fd(&guard.connection_fd()), events: libc::POLLIN, revents: 0 }];
            let timeout_ms: i32 = if app.cb_ready {
                let d = next_due.saturating_duration_since(Instant::now());
                (d.as_millis() as i32).min(250).max(0) + if d.subsec_micros() % 1000 > 0 { 1 } else { 0 }
            } else { 250 }; // paused (no frame callback): sleep, wake only to check signals
            let r = unsafe { libc::poll(fds.as_mut_ptr(), 1, timeout_ms) };
            if r > 0 {
                if let Err(e) = guard.read() {
                    if !matches!(&e, wayland_client::backend::WaylandError::Io(io) if io.kind() == std::io::ErrorKind::WouldBlock) {
                        eprintln!("wallforge-render: lost connection to the compositor ({e}); exiting"); break;
                    }
                }
            } else { drop(guard); }
        }
    }
    drop(audio);
    unsafe {
        eglMakeCurrent(dpy, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut());
        eglDestroySurface(dpy, esurf);
    }
    drop(win);
    layer.destroy();
    surface.destroy();
    let _ = conn.flush();
    eprintln!("wallforge-render: exit (presented {shown} frames)");
}
