mod audio;
mod check;
mod egl;
mod engine;
mod gfx;
mod pkg;
mod scene;
mod scene_run;
mod shader;
mod tex;
mod testgen;
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
static SIG_PIPE_W: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);
extern "C" fn on_signal(_: i32) {
    STOP.store(true, Ordering::SeqCst);
    let fd = SIG_PIPE_W.load(Ordering::SeqCst);
    if fd >= 0 { unsafe { libc::write(fd, [1u8].as_ptr() as *const _, 1); } } // wakes poll() so it can sleep without a timeout
}

/// Watchdog: ffmpeg (frame threads) and VAAPI can block forever on corrupt streams / GPU faults.
/// The main thread marks when it enters the decoder; a helper checks every 2 s and aborts if one call took > 6 s.
static DECODING_SINCE_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
fn now_ms(base: &Instant) -> u64 { base.elapsed().as_millis() as u64 + 1 }
fn spawn_watchdog(base: Instant) {
    let _ = std::thread::Builder::new().name("watchdog".into()).spawn(move || loop {
        std::thread::sleep(Duration::from_secs(2));
        let since = DECODING_SINCE_MS.load(Ordering::Relaxed);
        if since != 0 && now_ms(&base).saturating_sub(since) > 6000 {
            eprintln!("wallforge-render: decoder stalled for more than 6 s (corrupt file or GPU fault); exiting");
            unsafe { libc::_exit(1) };
        }
    });
}

fn fatal(msg: &str) -> ! { eprintln!("wallforge-render: {msg}"); std::process::exit(1) }

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
    check: bool,
    assets_dir: Option<PathBuf>,
    props: Vec<(String, String)>,
    make_test: Option<PathBuf>,
    screenshot: Option<PathBuf>,
    shot_size: (i32, i32),
    shot_time: f32,
}

fn usage() -> ! {
    eprintln!("usage: wallforge-render --output NAME --bg <dir|video> [--scaling fill|fit|stretch] [--fps N] \
               [--layer background|bottom|top|overlay] [--silent] [--volume N] [--duration SECS] [--software] [--assets-dir DIR] [--set-property key=value]...\n       wallforge-render --check <dir|video>   (probe, print one JSON line, no window)");
    std::process::exit(2)
}

fn parse_args() -> Args {
    let mut a = Args { output: String::new(), bg: PathBuf::new(), scaling: Scaling::Fill, fps: 30.0,
        layer: zwlr_layer_shell_v1::Layer::Background, duration: None, force_sw: false, volume: 100, silent: false, check: false, assets_dir: None, props: vec![], make_test: None, screenshot: None, shot_size: (1920, 1080), shot_time: 0.0 };
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
            "--assets-dir" => a.assets_dir = Some(PathBuf::from(val())),
            "--set-property" => { let v = val(); match v.split_once('=') { Some((k, x)) => a.props.push((k.to_string(), x.to_string())), None => usage() } }
            "--screenshot" => a.screenshot = Some(PathBuf::from(val())),
            "--screenshot-size" => { let v = val(); match v.split_once('x').and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?))) { Some(s) => a.shot_size = s, None => usage() } }
            "--screenshot-time" => a.shot_time = val().parse().unwrap_or_else(|_| usage()),
            "--make-test-scene" => { a.make_test = Some(PathBuf::from(val())); }
            "--check" => { a.check = true; a.bg = PathBuf::from(val()); }
            _ => usage(),
        }
    }
    if a.check || a.make_test.is_some() { return a; }
    if (a.output.is_empty() && a.screenshot.is_none()) || a.bg.as_os_str().is_empty() || a.fps <= 0.0 { usage() }
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
    let t_proc = Instant::now();
    std::panic::set_hook(Box::new(|info| {
        let m = info.payload().downcast_ref::<&str>().map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned()).unwrap_or_else(|| "panic".into());
        let th = std::thread::current();
        let loc = info.location().map(|l| format!(" ({}:{})", l.file().rsplit('/').next().unwrap_or(""), l.line())).unwrap_or_default();
        let line = m.lines().next().unwrap_or("panic");
        if th.name() == Some("audio") {
            eprintln!("wallforge-render: audio thread failed: {line}{loc}; continuing without audio");
        } else {
            eprintln!("wallforge-render: fatal: {line}{loc}");
            std::process::exit(1);
        }
    }));
    let args = parse_args();
    if let Some(d) = &args.make_test {
        testgen::make(d).unwrap_or_else(|e| fatal(&e));
        println!("{}", d.display());
        return;
    }
    if scene_run::is_scene(&args.bg) {
        if args.check { scene_run::check(&args); }
        scene_run::run(args);
        return;
    }
    let video_path = resolve_video(&args.bg).unwrap_or_else(|e| {
        if args.check { println!("{}", serde_json::json!({"ok": false, "error": e})); }
        fatal(&e)
    });
    if args.check {
        let b = Instant::now();
        spawn_watchdog(b);
        DECODING_SINCE_MS.store(now_ms(&b), Ordering::Relaxed);
        match check::probe(&video_path) {
            Ok(v) => { println!("{v}"); return; }
            Err(e) => { println!("{}", serde_json::json!({"ok": false, "error": e})); fatal(&e) }
        }
    }
    // open the decoder first: bad files fail fast, before any Wayland / EGL work
    let mut dec = video::Decoder::open(&video_path, !args.force_sw, args.fps).unwrap_or_else(|e| fatal(&e));
    if std::env::var_os("WALLFORGE_FFMPEG_LOG").is_none() { unsafe { ffmpeg_sys_next::av_log_set_level(ffmpeg_sys_next::AV_LOG_QUIET) }; }
    let wd_base = Instant::now();
    spawn_watchdog(wd_base);
    let mut sig_pipe = [0i32; 2];
    unsafe {
        if libc::pipe2(sig_pipe.as_mut_ptr(), libc::O_NONBLOCK | libc::O_CLOEXEC) == 0 { SIG_PIPE_W.store(sig_pipe[1], Ordering::SeqCst); }
    }
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
    let native = std::env::var("WF_NATIVE").map(|v| v != "0").unwrap_or(false);
    let ((mut bw, mut bh), mut is_native) = layout(&viewport, _fs.is_some(), app.frac, app.size, phys(app.size, app.frac), (dec.width, dec.height), native, args.scaling);
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
    set_opaque(&compositor, &surface, &qh, app.size);
    let mut gfx = Gfx::new(dpy, args.scaling);
    eprintln!("wallforge-render: {}x{} @{:.2}fps decode={} dmabuf={} -> {}x{} @{} cap",
        dec.width, dec.height, dec.fps, if dec.hw { "vaapi" } else { "software" }, gfx.can_dmabuf, bw, bh, args.fps);

    let audio = if args.silent { None } else { audio::Audio::start(&video_path, args.volume) };
    if audio.is_some() { eprintln!("wallforge-render: audio on (volume {})", args.volume); }
    let mut a_paused = true;
    let test_pause = std::env::var_os("WALLFORGE_TEST_PAUSE").is_some();
    let dbg = std::env::var_os("WALLFORGE_DEBUG").is_some();
    let mut held_cb = false;
    let period = Duration::from_secs_f64(1.0 / args.fps);
    let start = Instant::now();
    let (mut base_wall, mut base_pts, mut last_render) = (Instant::now(), 0.0f64, Instant::now());
    let mut next_slot = Instant::now(); // fps-cap pacing grid
    let mut next_due = Instant::now();  // earliest wall time something new can be shown
    let mut ring: VecDeque<video::Frame> = VecDeque::new();
    let mut shown = 0u64;
    let mut first = true;
    let mut dirty = true;
    let mut no_frame_polls = 0;
    let mut exit_code = 0;
    let mut last_present = Instant::now();
    let (mut st_to, mut st_ev, mut st_t) = (0u64, 0u64, Instant::now());

    let sw_fallback = |why: &str, dec: &mut video::Decoder| -> bool {
        eprintln!("wallforge-render: {why}, falling back to software decode");
        match video::Decoder::open(&video_path, false, args.fps) { Ok(d) => { *dec = d; true } Err(e) => { eprintln!("wallforge-render: software reopen failed: {e}"); false } }
    };

    while !STOP.load(Ordering::SeqCst) && !app.closed {
        if let Some(d) = args.duration { if start.elapsed().as_secs_f64() >= d { break; } }
        if dbg && st_t.elapsed() >= Duration::from_secs(2) { eprintln!("wallforge-render: sleeps {} blocking polls {} shown {} in 2s", st_to, st_ev, shown); st_to = 0; st_ev = 0; st_t = Instant::now(); }
        if let Err(e) = queue.dispatch_pending(&mut app) { eprintln!("wallforge-render: wayland error ({e}); exiting"); break; }
        if test_pause {
            // test hook: pretend the compositor sends no frame callbacks between 3 s and 7 s
            let t = start.elapsed().as_secs_f64();
            if t >= 3.0 && t < 7.0 { if app.cb_ready { app.cb_ready = false; held_cb = true; } }
            else if held_cb { app.cb_ready = true; held_cb = false; }
        }
        if app.resized {
            app.resized = false;
            ((bw, bh), is_native) = layout(&viewport, _fs.is_some(), app.frac, app.size, phys(app.size, app.frac), (dec.width, dec.height), native, args.scaling);
            win.resize(bw, bh, 0, 0);
            set_opaque(&compositor, &surface, &qh, app.size);
            eprintln!("wallforge-render: surface {}x{} logical -> {}x{} physical (scale {})", app.size.0, app.size.1, bw, bh, if app.frac > 0 { app.frac as f64 / 120.0 } else { scale as f64 });
            dirty = true;
        }
        let now = Instant::now();
        if (app.cb_ready && now >= next_due) || (dirty && !first) {
            if now.duration_since(last_render) > Duration::from_millis(500) && !first {
                base_wall = now; // woke from pause: continue where we stopped, do not fast-forward
                base_pts = ring.back().map(|f| f.pts).unwrap_or(0.0);
                next_slot = now;
            }
            last_render = now;
            let target = base_pts + now.duration_since(base_wall).as_secs_f64();
            let t0 = Instant::now();
            DECODING_SINCE_MS.store(now_ms(&wd_base), Ordering::Relaxed);
            let mut newf = dec.advance(if first { 0.0 } else { target });
            DECODING_SINCE_MS.store(0, Ordering::Relaxed);
            if dbg && t0.elapsed() > Duration::from_millis(8) { eprintln!("wallforge-render: slow advance {:.1} ms at media {:.2}s", t0.elapsed().as_secs_f64() * 1e3, target); }
            if newf.is_none() && dec.hw && shown == 0 && (dec.failed || { no_frame_polls += 1; no_frame_polls > 3 }) {
                if !sw_fallback("VAAPI produced no frames", &mut dec) { exit_code = 1; break; }
                base_wall = Instant::now(); base_pts = 0.0; no_frame_polls = 0;
                newf = dec.advance(0.0);
            } else if newf.is_none() && dec.hw && dec.map_fails > 3 {
                if !sw_fallback("VAAPI frame export failed", &mut dec) { exit_code = 1; break; }
                base_wall = Instant::now(); base_pts = 0.0; ring.clear(); dec.map_fails = 0;
                newf = dec.advance(0.0);
            }
            if dec.failed && newf.is_none() {
                eprintln!("wallforge-render: video cannot be decoded (corrupt or unsupported); exiting");
                exit_code = 1; break;
            }
            let mut changed = false;
            if let Some(mut f) = newf.take() {
                if let Err(e) = gfx.upload(&f) {
                    if !sw_fallback(&format!("hardware import failed ({e})"), &mut dec) { exit_code = 1; break; }
                    base_wall = Instant::now(); base_pts = 0.0; ring.clear();
                    match dec.advance(0.0) {
                        Some(f2) => f = f2,
                        None => { eprintln!("wallforge-render: software decode produced no frame; exiting"); exit_code = 1; break; }
                    }
                    if let Err(e) = gfx.upload(&f) { eprintln!("wallforge-render: upload failed ({e}); exiting"); exit_code = 1; break; }
                }
                let f_pos = f.pos;
                ring.push_back(f);
                while ring.len() > 1 { ring.pop_front(); }
                if let Some(a) = &audio { a.set_video_pos(f_pos); }
                changed = true; shown += 1;
            }
            if changed || dirty {
                if app.cb_ready { surface.frame(&qh, ()); app.cb_ready = false; }
                gfx.draw(bw, bh, is_native);
                unsafe { eglSwapBuffers(dpy, esurf); }
                dirty = false;
                if dbg && changed { let g = last_present.elapsed(); if g > period * 2 { eprintln!("wallforge-render: present gap {:.1} ms", g.as_secs_f64() * 1e3); } }
                last_present = Instant::now();
                if first { first = false; if dbg { eprintln!("wallforge-render: first frame {:.0} ms after start", t_proc.elapsed().as_secs_f64() * 1e3); } base_wall = Instant::now(); base_pts = ring.back().map(|f| f.pts).unwrap_or(0.0); next_slot = base_wall; }
            }
            // next wake: the cap grid, but no earlier than the next video frame is actually due
            next_slot += period;
            if next_slot < now { next_slot = now; }
            next_due = next_slot;
            if let Some(p) = dec.next_pts() {
                let due = base_wall + Duration::from_secs_f64((p - dec.frame_dur() * 0.5 - base_pts).max(0.0));
                if due > next_due { next_due = due; }
            }
            if dec.single && !first { next_due = now + Duration::from_secs(3600); } // still image: nothing to wake for
        }
        if let Some(a) = &audio {
            let want = !first && !app.cb_ready && last_render.elapsed() > Duration::from_millis(400);
            let want = want || first;
            if want != a_paused { a_paused = want; a.set_paused(want); }
        }
        if let Err(e) = conn.flush() { eprintln!("wallforge-render: lost connection to the compositor ({e}); exiting"); break; }
        if let Some(guard) = queue.prepare_read() {
            let mut fds = [libc::pollfd { fd: std::os::fd::AsRawFd::as_raw_fd(&guard.connection_fd()), events: libc::POLLIN, revents: 0 },
                           libc::pollfd { fd: sig_pipe[0], events: libc::POLLIN, revents: 0 }];
            // Sleep until the earliest real deadline; with nothing pending (hidden / paused) block with no timer at all.
            let now = Instant::now();
            let mut deadline: Option<Instant> = None;
            let mut upd = |d: Instant| deadline = Some(deadline.map_or(d, |x| x.min(d)));
            if app.cb_ready { upd(next_due); }
            else if audio.is_some() && !a_paused { upd(last_render + Duration::from_millis(400)); }
            if let Some(d) = args.duration { upd(start + Duration::from_secs_f64(d)); }
            if test_pause {
                let t = start.elapsed().as_secs_f64();
                if t < 3.0 { upd(start + Duration::from_secs(3)); } else if t < 7.0 { upd(start + Duration::from_secs(7)); }
            }
            let timeout_ms: i32 = match deadline {
                None => -1,
                Some(d) => { let r = d.saturating_duration_since(now); (r.as_micros().div_ceil(1000) as i32).clamp(if r.is_zero() { 0 } else { 1 }, 3_600_000) }
            };
            // Frame just presented and the next one is not due yet: the compositor's events (frame done, buffer
            // release) would each wake us. Sleep on the signal pipe only until the frame is due, then read them all at once.
            let sleep_only = !app.cb_ready && !first && next_due > now && last_render.elapsed() < period * 3;
            let timeout_ms = if sleep_only { let r = next_due - now; let t = r.as_micros().div_ceil(1000).clamp(1, 3_600_000) as i32; if timeout_ms >= 0 { t.min(timeout_ms) } else { t } } else { timeout_ms };
            let r = unsafe { libc::poll(fds.as_mut_ptr().add(sleep_only as usize), 2 - sleep_only as libc::nfds_t, timeout_ms) };
            if dbg { if sleep_only { st_to += 1 } else { st_ev += 1 } }
            if sleep_only && r >= 0 { fds[0].revents = libc::POLLIN; }
            if (r > 0 || sleep_only) && fds[0].revents != 0 {
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
    if exit_code != 0 { std::process::exit(exit_code); }
}

fn set_opaque(c: &wl_compositor::WlCompositor, s: &wl_surface::WlSurface, qh: &QueueHandle<App>, size: (i32, i32)) {
    let r = c.create_region(qh, ());
    r.add(0, 0, size.0.max(1), size.1.max(1));
    s.set_opaque_region(Some(&r));
    r.destroy();
}

/// Buffer size + viewport setup. In "native" mode the buffer has the video's own size (when smaller than the
/// physical output) and the compositor scales/crops through wp_viewport, so the shader draws fewer pixels.
fn layout(viewport: &Option<wp_viewport::WpViewport>, fs_ok: bool, frac: u32, logical: (i32, i32), phys: (i32, i32),
          video: (u32, u32), native: bool, scaling: Scaling) -> ((i32, i32), bool) {
    let Some(v) = viewport else { return (phys, false) };
    if native && scaling == Scaling::Fill && (video.0 as i64 * video.1 as i64) < (phys.0 as i64 * phys.1 as i64) && logical.0 > 0 && logical.1 > 0 {
        let (vw, vh) = (video.0 as f64, video.1 as f64);
        let oa = logical.0 as f64 / logical.1 as f64;
        let (cw, ch) = if vw / vh > oa { (vh * oa, vh) } else { (vw, vw / oa) };
        v.set_source((vw - cw) / 2.0, (vh - ch) / 2.0, cw, ch);
        v.set_destination(logical.0, logical.1);
        return ((video.0 as i32, video.1 as i32), true);
    }
    v.set_source(-1.0, -1.0, -1.0, -1.0);
    if fs_ok && frac > 0 { v.set_destination(logical.0, logical.1); } else { v.set_destination(-1, -1); }
    (phys, false)
}
