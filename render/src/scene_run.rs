//! Wayland + EGL host loop for scene wallpapers (mirrors the video path in main.rs, without a decoder).
use crate::egl::*;
use crate::engine::Engine;
use crate::pkg::Vfs;
use crate::scene::{self, Props, Scene};
use crate::{fatal, set_opaque, App, Args, SIG_PIPE_W, STOP};
use std::ffi::c_void;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use wayland_client::globals::registry_queue_init;
use wayland_client::protocol::{wl_compositor, wl_output};
use wayland_client::{Connection, Proxy};
use wayland_protocols::wp::fractional_scale::v1::client::wp_fractional_scale_manager_v1;
use wayland_protocols::wp::viewporter::client::wp_viewporter;
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

/// Is `bg` a directory whose project.json says type "scene"?
pub fn is_scene(bg: &Path) -> bool {
    if !bg.is_dir() { return false; }
    let Ok(t) = std::fs::read_to_string(bg.join("project.json")) else { return false };
    serde_json::from_str::<serde_json::Value>(&t).map(|j| j["type"].as_str().map(|s| s.eq_ignore_ascii_case("scene")).unwrap_or(false)).unwrap_or(false)
}

pub fn load(args: &Args) -> Result<(Vfs, Scene), String> {
    let txt = std::fs::read_to_string(args.bg.join("project.json")).map_err(|e| format!("project.json: {e}"))?;
    let proj: serde_json::Value = serde_json::from_str(&txt).map_err(|e| format!("project.json: {e}"))?;
    let props = Props::load(&proj, &args.props);
    let vfs = Vfs::open(&args.bg, args.assets_dir.clone())?;
    let file = proj["file"].as_str().unwrap_or("scene.json").to_string();
    let j = vfs.read_json(&file).or_else(|| vfs.read_json("scene.json")).ok_or_else(|| format!("{file} not found in the wallpaper"))?;
    let s = scene::parse(&j, &props)?;
    Ok((vfs, s))
}

/// `--check` for a scene directory: one JSON line. Exit code 3 when the scene cannot be rendered.
pub fn check(args: &Args) -> ! {
    match load(args) {
        Ok((vfs, s)) => {
            let r = scene::report(&vfs, &s);
            println!("{r}");
            std::process::exit(if s.fatal.is_empty() { 0 } else { 3 })
        }
        Err(e) => { println!("{}", serde_json::json!({"ok": false, "error": e})); std::process::exit(1) }
    }
}

pub fn run(args: Args) {
    let (vfs, scn) = load(&args).unwrap_or_else(|e| fatal(&e));
    if !scn.fatal.is_empty() {
        eprintln!("wallforge-render: unsupported scene: {}", scn.fatal.join("; "));
        std::process::exit(3);
    }
    for w in &scn.warnings { eprintln!("wallforge-render: note: {w}"); }

    if let Some(path) = args.screenshot.clone() { return screenshot(&args, vfs, scn, &path); }

    let mut sig_pipe = [0i32; 2];
    unsafe {
        if libc::pipe2(sig_pipe.as_mut_ptr(), libc::O_NONBLOCK | libc::O_CLOEXEC) == 0 { SIG_PIPE_W.store(sig_pipe[1], Ordering::SeqCst); }
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = crate::on_signal as *const () as usize;
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
    let fs = match (&fsm, &viewport) { (Some(m), Some(_)) => Some(m.get_fractional_scale(&surface, &qh, ())), _ => None };
    if fs.is_none() && scale > 1 { surface.set_buffer_scale(scale); }
    surface.commit();
    while !app.configured && !app.closed { queue.blocking_dispatch(&mut app).unwrap(); }
    if app.closed { eprintln!("layer surface closed"); return; }
    if fs.is_some() { let _ = queue.roundtrip(&mut app); }
    app.resized = false;

    let phys = |logical: (i32, i32), frac: u32| -> (i32, i32) {
        if fs.is_some() && frac > 0 {
            (((logical.0 as i64 * frac as i64 + 60) / 120) as i32, ((logical.1 as i64 * frac as i64 + 60) / 120) as i32)
        } else { (logical.0 * scale, logical.1 * scale) }
    };
    let apply_viewport = |frac: u32, logical: (i32, i32)| {
        if let Some(v) = &viewport {
            v.set_source(-1.0, -1.0, -1.0, -1.0);
            if fs.is_some() && frac > 0 { v.set_destination(logical.0, logical.1); } else { v.set_destination(-1, -1); }
        }
    };
    apply_viewport(app.frac, app.size);
    let (bw, bh) = phys(app.size, app.frac);
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
        let w = wayland_egl::WlEglSurface::new(surface.id(), bw, bh).expect("wl_egl_window");
        esurf = eglCreatePlatformWindowSurface(dpy, cfg, w.ptr() as *mut c_void, std::ptr::null());
        win = w;
        assert!(!esurf.is_null(), "eglCreateWindowSurface");
        assert!(eglMakeCurrent(dpy, esurf, esurf, ctx) != 0, "eglMakeCurrent");
        eglSwapInterval(dpy, 0);
    }
    gl::load_with(|n| proc_addr(n) as *const _);
    set_opaque(&compositor, &surface, &qh, app.size);
    let mut engine = Engine::new(vfs, scn, (bw, bh), args.scaling).unwrap_or_else(|e| fatal(&e));

    let period = Duration::from_secs_f64(1.0 / args.fps);
    let start = Instant::now();
    let mut next_due = Instant::now();
    let mut dirty = true;
    let mut shown = 0u64;
    while !STOP.load(Ordering::SeqCst) && !app.closed {
        if let Some(d) = args.duration { if start.elapsed().as_secs_f64() >= d { break; } }
        if let Err(e) = queue.dispatch_pending(&mut app) { eprintln!("wallforge-render: wayland error ({e}); exiting"); break; }
        if app.resized {
            app.resized = false;
            apply_viewport(app.frac, app.size);
            let (w, h) = phys(app.size, app.frac);
            win.resize(w, h, 0, 0);
            set_opaque(&compositor, &surface, &qh, app.size);
            engine.resize((w, h));
            eprintln!("wallforge-render: surface {}x{} logical -> {}x{} physical", app.size.0, app.size.1, w, h);
            dirty = true;
        }
        if engine.text_tick() { dirty = true; }
        let now = Instant::now();
        if app.cb_ready && (dirty || (engine.animated && now >= next_due)) {
            engine.tick();
            surface.frame(&qh, ());
            app.cb_ready = false;
            engine.draw();
            unsafe { eglSwapBuffers(dpy, esurf); }
            dirty = false; shown += 1;
            next_due = if next_due + period < now { now + period } else { next_due + period };
        }
        if let Err(e) = conn.flush() { eprintln!("wallforge-render: lost connection to the compositor ({e}); exiting"); break; }
        if let Some(guard) = queue.prepare_read() {
            let mut fds = [libc::pollfd { fd: std::os::fd::AsRawFd::as_raw_fd(&guard.connection_fd()), events: libc::POLLIN, revents: 0 },
                           libc::pollfd { fd: sig_pipe[0], events: libc::POLLIN, revents: 0 }];
            let now = Instant::now();
            let mut deadline: Option<Instant> = None;
            if engine.animated && app.cb_ready { deadline = Some(next_due); }
            if let Some(d) = engine.text_deadline() { deadline = Some(deadline.map_or(d, |x| x.min(d))); }
            if let Some(d) = args.duration {
                let e = start + Duration::from_secs_f64(d);
                deadline = Some(deadline.map_or(e, |x| x.min(e)));
            }
            let timeout_ms: i32 = match deadline {
                None => -1,
                Some(d) => { let r = d.saturating_duration_since(now); (r.as_micros().div_ceil(1000) as i32).clamp(if r.is_zero() { 0 } else { 1 }, 3_600_000) }
            };
            let r = unsafe { libc::poll(fds.as_mut_ptr(), 2, timeout_ms) };
            if r > 0 && fds[0].revents != 0 {
                if let Err(e) = guard.read() {
                    if !matches!(&e, wayland_client::backend::WaylandError::Io(io) if io.kind() == std::io::ErrorKind::WouldBlock) {
                        eprintln!("wallforge-render: lost connection to the compositor ({e}); exiting"); break;
                    }
                }
            } else { drop(guard); }
        }
    }
    drop(engine);
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

#[allow(dead_code)]
fn _unused(_: zwlr_layer_surface_v1::Anchor) {}

/// Headless render: EGL pbuffer (surfaceless Mesa platform, else default display), N frames, PNG dump, no window.
fn screenshot(args: &Args, vfs: Vfs, scn: Scene, path: &Path) {
    let (w, h) = args.shot_size;
    let (dpy, ctx, surf);
    unsafe {
        let mut d = eglGetPlatformDisplay(EGL_PLATFORM_SURFACELESS_MESA, std::ptr::null_mut(), std::ptr::null());
        let (mut ma, mut mi) = (0, 0);
        if d.is_null() || eglInitialize(d, &mut ma, &mut mi) == 0 {
            d = eglGetDisplay(std::ptr::null_mut());
            if d.is_null() || eglInitialize(d, &mut ma, &mut mi) == 0 { fatal("EGL init failed (headless)"); }
        }
        dpy = d;
        eglBindAPI(EGL_OPENGL_ES_API);
        let ca = [EGL_SURFACE_TYPE, EGL_PBUFFER_BIT, EGL_RENDERABLE_TYPE, EGL_OPENGL_ES3_BIT,
                  EGL_RED_SIZE, 8, EGL_GREEN_SIZE, 8, EGL_BLUE_SIZE, 8, EGL_ALPHA_SIZE, 8, EGL_NONE];
        let mut cfg: EGLConfig = std::ptr::null_mut(); let mut n = 0;
        if eglChooseConfig(dpy, ca.as_ptr(), &mut cfg, 1, &mut n) == 0 || n == 0 { fatal("no EGL pbuffer config"); }
        let cx = [EGL_CONTEXT_CLIENT_VERSION, 3, EGL_NONE];
        ctx = eglCreateContext(dpy, cfg, std::ptr::null_mut(), cx.as_ptr());
        if ctx.is_null() { fatal("eglCreateContext"); }
        let pa = [EGL_WIDTH, w, EGL_HEIGHT, h, EGL_NONE];
        surf = eglCreatePbufferSurface(dpy, cfg, pa.as_ptr());
        if surf.is_null() || eglMakeCurrent(dpy, surf, surf, ctx) == 0 { fatal("pbuffer surface / eglMakeCurrent"); }
    }
    gl::load_with(|n| proc_addr(n) as *const _);
    let mut engine = Engine::new(vfs, scn, (w, h), args.scaling).unwrap_or_else(|e| fatal(&e));
    // two warm-up frames at t-ε so ping-pong/time-dependent state settles, last frame at the requested time
    let t = args.shot_time;
    for i in 0..3 {
        engine.set_time(Some(if i < 2 { (t - 0.05 * (2 - i) as f32).max(0.0) } else { t }));
        engine.tick();
        engine.draw();
    }
    let mut px = vec![0u8; (w * h * 4) as usize];
    unsafe {
        gl::BindFramebuffer(gl::FRAMEBUFFER, 0);
        gl::Finish();
        gl::ReadPixels(0, 0, w, h, gl::RGBA, gl::UNSIGNED_BYTE, px.as_mut_ptr() as *mut _);
    }
    let row = (w * 4) as usize;
    let mut flipped = Vec::with_capacity(px.len());
    for y in (0..h as usize).rev() { flipped.extend_from_slice(&px[y * row..(y + 1) * row]); }
    for p in flipped.chunks_mut(4) { p[3] = 255; }
    match image::save_buffer(path, &flipped, w as u32, h as u32, image::ColorType::Rgba8) {
        Ok(()) => eprintln!("wallforge-render: screenshot {} ({}x{}, t={:.2}s)", path.display(), w, h, t),
        Err(e) => fatal(&format!("writing {}: {e}", path.display())),
    }
    drop(engine);
    unsafe { eglMakeCurrent(dpy, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut()); eglDestroySurface(dpy, surf); }
}
