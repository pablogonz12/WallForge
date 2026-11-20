//! Video demux/decode. Software (ffmpeg) or VAAPI (frames exported as DRM PRIME / DMA-BUF).
use ffmpeg_next as ff;
use ffmpeg_sys_next as sys;
use std::ptr;

#[repr(C)]
pub struct DrmObject { pub fd: i32, pub size: usize, pub modifier: u64 }
#[repr(C)]
pub struct DrmPlane { pub object_index: i32, pub offset: isize, pub pitch: isize }
#[repr(C)]
pub struct DrmLayer { pub format: u32, pub nb_planes: i32, pub planes: [DrmPlane; 4] }
#[repr(C)]
pub struct DrmDesc { pub nb_objects: i32, pub objects: [DrmObject; 4], pub nb_layers: i32, pub layers: [DrmLayer; 4] }

/// A DRM-PRIME mapped hardware frame. Dropping releases the VAAPI surface back to the pool.
pub struct DrmFrame(*mut sys::AVFrame);
impl DrmFrame {
    pub fn desc(&self) -> &DrmDesc { unsafe { &*((*self.0).data[0] as *const DrmDesc) } }
}
impl Drop for DrmFrame {
    fn drop(&mut self) { unsafe { sys::av_frame_free(&mut self.0) } }
}

pub enum Data { Drm(DrmFrame), Sw(ff::frame::Video) }

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Matrix { Bt601, Bt709, Bt2020 }

pub struct Frame {
    pub pts: f64,
    pub pos: f64, // position inside the current loop
    pub width: u32,
    pub height: u32,
    pub matrix: Matrix,
    pub full_range: bool,
    pub data: Data,
}

struct Pending { frame: ff::frame::Video, pts: f64, pos: f64 }

#[derive(PartialEq)]
enum State { Reading, Draining }

pub struct Decoder {
    input: ff::format::context::Input,
    dec: ff::decoder::Video,
    stream: usize,
    tb: f64,
    pub fps: f64,
    dur: f64,
    start_ts: Option<f64>,
    loop_off: f64,
    last_pts: f64,
    state: State,
    pending: Option<Pending>,
    pub hw: bool,
    pub width: u32,
    pub height: u32,
    scaler: Option<(ff::software::scaling::Context, ff::frame::Video)>,
    /// Set when the stream cannot be decoded (no frame in a whole pass, too many errors).
    pub failed: bool,
    got_in_loop: u32,
    loops: u32,
    /// Single-frame video: show it once, never decode again.
    pub single: bool,
    delivered: bool,
    pub map_fails: u32,
    /// Duration of the video stream in seconds (0 = unknown).
    pub stream_secs: f64,
}

unsafe extern "C" fn get_vaapi_format(_c: *mut sys::AVCodecContext, fmts: *const sys::AVPixelFormat) -> sys::AVPixelFormat {
    let mut p = fmts;
    while *p != sys::AVPixelFormat::AV_PIX_FMT_NONE {
        if *p == sys::AVPixelFormat::AV_PIX_FMT_VAAPI { return *p; }
        p = p.add(1);
    }
    sys::AVPixelFormat::AV_PIX_FMT_NONE
}

impl Decoder {
    pub fn open(path: &str, want_hw: bool, cap: f64) -> Result<Decoder, String> {
        ff::init().map_err(|e| e.to_string())?;
        unsafe { sys::av_log_set_level(if std::env::var_os("WALLFORGE_FFMPEG_LOG").is_some() { sys::AV_LOG_WARNING as i32 } else { sys::AV_LOG_QUIET }) };
        let input = ff::format::input(&path).map_err(|e| format!("open {path}: {e}"))?;
        let st = input.streams().best(ff::media::Type::Video).ok_or("no video stream")?;
        let stream = st.index();
        let tb: f64 = st.time_base().into();
        let fr: f64 = st.avg_frame_rate().into();
        let fps = if fr.is_finite() && fr > 1.0 { fr } else { 30.0 };
        let stream_secs = if st.duration() > 0 { st.duration() as f64 * tb } else { 0.0 };
        let mut ctx = ff::codec::context::Context::from_parameters(st.parameters()).map_err(|e| e.to_string())?;
        let mut hw = false;
        unsafe {
            let c = ctx.as_mut_ptr();
            // video much faster than the fps cap: let the decoder skip non-reference frames entirely
            if fps >= cap * 1.9 { (*c).skip_frame = sys::AVDiscard::AVDISCARD_NONREF; }
            if want_hw {
                let mut dev: *mut sys::AVBufferRef = ptr::null_mut();
                let r = sys::av_hwdevice_ctx_create(&mut dev, sys::AVHWDeviceType::AV_HWDEVICE_TYPE_VAAPI, ptr::null(), ptr::null_mut(), 0);
                if r >= 0 {
                    (*c).hw_device_ctx = dev;
                    (*c).get_format = Some(get_vaapi_format);
                    (*c).extra_hw_frames = 1;
                    hw = true;
                }
            } else {
                (*c).thread_count = 4;
                (*c).thread_type = if std::env::var_os("WF_SLICE").is_some() { sys::FF_THREAD_SLICE as i32 } else { sys::FF_THREAD_FRAME as i32 | sys::FF_THREAD_SLICE as i32 };
            }
        }
        // libdav1d (the default AV1 decoder) cannot do VAAPI; use the native "av1" decoder for hardware
        let native_av1 = if want_hw && ctx.id() == ff::codec::Id::AV1 { ff::decoder::find_by_name("av1") } else { None };
        let dec = match native_av1 {
            Some(c) => ctx.decoder().open_as(c).and_then(|o| o.video()),
            None => ctx.decoder().video(),
        }.map_err(|e| format!("decoder: {e}"))?;
        let (width, height) = (dec.width(), dec.height());
        Ok(Decoder {
            input, dec, stream, tb, fps, dur: 1.0 / fps, start_ts: None, loop_off: 0.0, last_pts: 0.0,
            state: State::Reading, pending: None, hw, width, height, scaler: None,
            failed: false, got_in_loop: 0, loops: 0, single: false, delivered: false, map_fails: 0, stream_secs,
        })
    }

    fn decode_next(&mut self) -> bool {
        let mut frame = ff::frame::Video::empty();
        let mut errs = 0;
        loop {
            match self.dec.receive_frame(&mut frame) {
                Ok(()) => {
                    let ts = frame.timestamp().or(frame.pts()).map(|t| t as f64 * self.tb);
                    let t = match ts { Some(t) => t, None => self.last_pts + self.dur };
                    let st = *self.start_ts.get_or_insert(t);
                    let pts = t - st + self.loop_off;
                    self.last_pts = pts;
                    self.got_in_loop += 1;
                    self.pending = Some(Pending { frame, pts, pos: t - st });
                    return true;
                }
                Err(ff::Error::Eof) => { self.restart(); if self.failed { return false; } }
                Err(ff::Error::Other { errno }) if errno == libc::EAGAIN => {
                    if self.state == State::Draining { self.restart(); if self.failed { return false; } continue; }
                    let mut pkt = ff::Packet::empty();
                    match pkt.read(&mut self.input) {
                        Ok(()) => { if pkt.stream() == self.stream { let _ = self.dec.send_packet(&pkt); } }
                        Err(ff::Error::Eof) => { let _ = self.dec.send_eof(); self.state = State::Draining; }
                        Err(_) => { errs += 1; if errs > 50 { self.failed = true; return false; } }
                    }
                }
                Err(_) => { errs += 1; if errs > 50 { self.failed = true; return false; } }
            }
        }
    }

    /// Loop: seek back to the start and continue the timeline.
    fn restart(&mut self) {
        if self.got_in_loop == 0 { self.failed = true; return; }
        if self.loops == 0 && self.got_in_loop == 1 { self.single = true; }
        self.loops += 1;
        self.got_in_loop = 0;
        unsafe { sys::av_seek_frame(self.input.as_mut_ptr(), -1, 0, sys::AVSEEK_FLAG_BACKWARD as i32); }
        self.dec.flush();
        self.state = State::Reading;
        self.loop_off = self.last_pts + self.dur;
        self.start_ts = None;
    }

    /// Return the newest frame whose presentation time is <= target (media seconds). Earlier due frames
    /// are dropped without any export/upload. None if no new frame is due yet.
    pub fn frame_dur(&self) -> f64 { self.dur }

    /// Media time of the next undelivered frame (None if unknown / static).
    pub fn next_pts(&self) -> Option<f64> { if self.single { None } else { self.pending.as_ref().map(|p| p.pts) } }

    pub fn advance(&mut self, target: f64) -> Option<Frame> {
        if self.single && self.delivered { return None; }
        let mut cand: Option<Pending> = None;
        loop {
            if self.pending.is_none() && !self.decode_next() { break; }
            let p = self.pending.as_ref().unwrap();
            if p.pts <= target + self.dur * 0.5 { cand = self.pending.take(); } else { break; }
        }
        let p = cand?;
        let f = self.make(p);
        if f.is_some() { self.delivered = true; } else { self.map_fails += 1; }
        f
    }

    fn make(&mut self, p: Pending) -> Option<Frame> {
        let f = &p.frame;
        let (w, h) = (f.width(), f.height());
        let (matrix, full_range);
        unsafe {
            let r = f.as_ptr();
            matrix = match (*r).colorspace as i32 {
                1 => Matrix::Bt709, 5 | 6 => Matrix::Bt601, 9 | 10 => Matrix::Bt2020,
                _ => if h >= 720 { Matrix::Bt709 } else { Matrix::Bt601 },
            };
            full_range = (*r).color_range as i32 == 2;
        }
        let data = if self.hw {
            unsafe {
                let dst = sys::av_frame_alloc();
                (*dst).format = sys::AVPixelFormat::AV_PIX_FMT_DRM_PRIME as i32;
                let r = sys::av_hwframe_map(dst, f.as_ptr(), sys::AV_HWFRAME_MAP_READ as i32);
                if r < 0 { let mut d = dst; sys::av_frame_free(&mut d); return None; }
                Data::Drm(DrmFrame(dst))
            }
        } else {
            use ff::format::Pixel;
            match f.format() {
                Pixel::YUV420P | Pixel::YUVJ420P | Pixel::NV12 => Data::Sw(p.frame),
                fmt => {
                    if self.scaler.is_none() {
                        let s = ff::software::scaling::Context::get(fmt, w, h, Pixel::YUV420P, w, h, ff::software::scaling::Flags::FAST_BILINEAR).ok()?;
                        self.scaler = Some((s, ff::frame::Video::new(Pixel::YUV420P, w, h)));
                    }
                    let (s, out) = self.scaler.as_mut().unwrap();
                    s.run(f, out).ok()?;
                    let mut o = ff::frame::Video::new(Pixel::YUV420P, w, h);
                    o.clone_from(out);
                    Data::Sw(o)
                }
            }
        };
        Some(Frame { pts: p.pts, pos: p.pos, width: w, height: h, matrix, full_range, data })
    }
}

// ---------------- embedded (TEX) video textures: software decode to RGBA, looped ----------------

struct SendBox<T>(T);
unsafe impl<T> Send for SendBox<T> {}

/// Sequential RGBA decoder over an in-memory mp4 (spilled to an unlinked temp file so ffmpeg can seek).
struct RgbaDec {
    input: ff::format::context::Input, dec: ff::decoder::Video, stream: usize, draining: bool, got: u32,
    frame: ff::frame::Video, scaler: Option<ff::software::scaling::Context>, out: ff::frame::Video,
    pub w: u32, pub h: u32, pub fps: f64, n: u64, rgba: Vec<u8>,
}

impl RgbaDec {
    fn open(bytes: &[u8], want: (u32, u32)) -> Result<RgbaDec, String> {
        ff::init().map_err(|e| e.to_string())?;
        unsafe { sys::av_log_set_level(if std::env::var_os("WALLFORGE_FFMPEG_LOG").is_some() { sys::AV_LOG_WARNING as i32 } else { sys::AV_LOG_QUIET }) };
        static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let dir = std::env::var_os("XDG_RUNTIME_DIR").map(|d| std::path::PathBuf::from(d).join("wallforge")).filter(|d| std::fs::create_dir_all(d).is_ok()).unwrap_or_else(std::env::temp_dir);
        let path = dir.join(format!("embedded-{}-{}.mp4", std::process::id(), SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
        std::fs::write(&path, bytes).map_err(|e| format!("video temp file: {e}"))?;
        let input = ff::format::input(&path);
        let _ = std::fs::remove_file(&path); // the open descriptor keeps the data alive
        let input = input.map_err(|e| format!("embedded video: {e}"))?;
        let st = input.streams().best(ff::media::Type::Video).ok_or("embedded video: no video stream")?;
        let stream = st.index();
        let fr: f64 = st.avg_frame_rate().into();
        let fps = if fr.is_finite() && fr > 1.0 && fr < 240.0 { fr } else { 30.0 };
        let mut ctx = ff::codec::context::Context::from_parameters(st.parameters()).map_err(|e| e.to_string())?;
        unsafe { (*ctx.as_mut_ptr()).thread_count = 2; }
        let dec = ctx.decoder().video().map_err(|e| format!("embedded video decoder: {e}"))?;
        let (sw, sh) = (dec.width().max(1), dec.height().max(1));
        // shrink by powers of two while still at least the on-screen size
        let (mut w, mut h) = (sw, sh);
        while w / 2 >= want.0.max(1) && h / 2 >= want.1.max(1) && w > 2 && h > 2 { w /= 2; h /= 2; }
        let (w, h) = (w & !1, h & !1);
        let (w, h) = (w.max(2), h.max(2));
        Ok(RgbaDec { input, dec, stream, draining: false, got: 0, frame: ff::frame::Video::empty(), scaler: None, out: ff::frame::Video::new(ff::format::Pixel::RGBA, w, h), w, h, fps, n: 0, rgba: vec![0; (w * h * 4) as usize] })
    }

    /// Decode the next frame (looping at end of stream) into self.frame. Returns its index on the loop-free timeline.
    fn decode(&mut self) -> bool {
        let mut errs = 0;
        loop {
            match self.dec.receive_frame(&mut self.frame) {
                Ok(()) => { self.got += 1; self.n += 1; return true; }
                Err(ff::Error::Eof) => { if self.got == 0 { return false; } self.got = 0; self.draining = false; unsafe { sys::av_seek_frame(self.input.as_mut_ptr(), -1, 0, sys::AVSEEK_FLAG_BACKWARD as i32); } self.dec.flush(); }
                Err(ff::Error::Other { errno }) if errno == libc::EAGAIN => {
                    let mut pkt = ff::Packet::empty();
                    match pkt.read(&mut self.input) {
                        Ok(()) => { if pkt.stream() == self.stream { let _ = self.dec.send_packet(&pkt); } }
                        Err(ff::Error::Eof) => { if !self.draining { let _ = self.dec.send_eof(); self.draining = true; } }
                        Err(_) => { errs += 1; if errs > 50 { return false; } }
                    }
                }
                Err(_) => { errs += 1; if errs > 50 { return false; } }
            }
        }
    }

    /// Convert self.frame to tightly packed RGBA in self.rgba.
    fn convert(&mut self) {
        if self.scaler.is_none() {
            match ff::software::scaling::Context::get(self.frame.format(), self.frame.width(), self.frame.height(), ff::format::Pixel::RGBA, self.w, self.h, ff::software::scaling::Flags::BILINEAR) {
                Ok(s) => self.scaler = Some(s), Err(_) => return,
            }
        }
        if self.scaler.as_mut().unwrap().run(&self.frame, &mut self.out).is_err() { return; }
        let (stride, row) = (self.out.stride(0), self.w as usize * 4);
        let d = self.out.data(0);
        for y in 0..self.h as usize { self.rgba[y * row..(y + 1) * row].copy_from_slice(&d[y * stride..y * stride + row]); }
    }

    /// Decode up to the frame due at media time t (seconds) and convert only that one.
    fn seek_to(&mut self, t: f64) -> bool {
        let target = (t.max(0.0) * self.fps) as u64 + 1;
        let mut ok = false;
        while self.n < target || !ok { if !self.decode() { return ok; } ok = true; }
        self.convert();
        true
    }
}

struct Shared { latest: std::sync::Mutex<(Vec<u8>, u64)>, stop: std::sync::atomic::AtomicBool }

/// A looping video that feeds one GL texture. Live: a decode thread paces itself at the video's frame rate and
/// publishes the newest frame. Screenshot (fixed time): decodes synchronously to the requested time.
pub struct VideoTex {
    pub tex: u32, w: u32, h: u32, dec: Option<RgbaDec>, shared: std::sync::Arc<Shared>, thread: Option<std::thread::JoinHandle<()>>, seen: u64,
}

impl VideoTex {
    /// Open and decode the first frame. Returns the output size too.
    pub fn open(bytes: &[u8], want: (u32, u32), clamp: bool, nearest: bool) -> Result<VideoTex, String> {
        let mut dec = RgbaDec::open(bytes, want)?;
        if !dec.seek_to(0.0) { return Err("embedded video: no decodable frames".into()); }
        let (w, h) = (dec.w, dec.h);
        let mut tex = 0;
        unsafe {
            gl::GenTextures(1, &mut tex); gl::BindTexture(gl::TEXTURE_2D, tex);
            let f = if nearest { gl::NEAREST } else { gl::LINEAR } as i32;
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, f); gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, f);
            let c = if clamp { gl::CLAMP_TO_EDGE } else { gl::REPEAT } as i32;
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_S, c); gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_T, c);
            gl::TexImage2D(gl::TEXTURE_2D, 0, gl::RGBA8 as i32, w as i32, h as i32, 0, gl::RGBA, gl::UNSIGNED_BYTE, dec.rgba.as_ptr() as *const _);
        }
        let mut v = VideoTex { tex, w, h, dec: None, shared: std::sync::Arc::new(Shared { latest: std::sync::Mutex::new((Vec::new(), 0)), stop: Default::default() }), thread: None, seen: 0 };
        v.dec = Some(dec);
        Ok(v)
    }
    pub fn size(&self) -> (u32, u32) { (self.w, self.h) }

    fn upload(&self, px: &[u8]) {
        unsafe {
            gl::BindTexture(gl::TEXTURE_2D, self.tex);
            gl::TexSubImage2D(gl::TEXTURE_2D, 0, 0, 0, self.w as i32, self.h as i32, gl::RGBA, gl::UNSIGNED_BYTE, px.as_ptr() as *const _);
        }
    }

    /// Call from the render thread (GL context current) once per tick. `fixed` = media time for screenshots.
    pub fn update(&mut self, fixed: Option<f64>) {
        if let Some(t) = fixed {
            if let Some(mut d) = self.dec.take() { if d.seek_to(t) { self.upload(&d.rgba); } self.dec = Some(d); }
            return;
        }
        if self.thread.is_none() {
            let Some(d) = self.dec.take() else { return };
            let (sh, d0) = (self.shared.clone(), SendBox(d));
            self.thread = Some(std::thread::spawn(move || {
                let d0 = d0; let mut d = d0.0; // (whole-struct capture so the Send wrapper applies)
                let start = std::time::Instant::now();
                let base = d.n;
                while !sh.stop.load(std::sync::atomic::Ordering::Relaxed) {
                    if !d.decode() { break; }
                    let due = std::time::Duration::from_secs_f64((d.n - base) as f64 / d.fps);
                    let now = start.elapsed();
                    if now > due + std::time::Duration::from_secs_f64(1.5 / d.fps) { continue; } // behind: drop the frame unconverted
                    d.convert();
                    while !sh.stop.load(std::sync::atomic::Ordering::Relaxed) {
                        let e = start.elapsed();
                        if e >= due { break; }
                        std::thread::sleep((due - e).min(std::time::Duration::from_millis(20)));
                    }
                    let mut g = sh.latest.lock().unwrap();
                    g.0.clear(); g.0.extend_from_slice(&d.rgba); g.1 += 1;
                }
            }));
        }
        let g = self.shared.latest.try_lock();
        if let Ok(g) = g { if g.1 != self.seen { self.seen = g.1; self.upload(&g.0); } }
    }
}

impl Drop for VideoTex {
    fn drop(&mut self) {
        self.shared.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(t) = self.thread.take() { let _ = t.join(); }
    }
}
