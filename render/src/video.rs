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
    pub width: u32,
    pub height: u32,
    pub matrix: Matrix,
    pub full_range: bool,
    pub data: Data,
}

struct Pending { frame: ff::frame::Video, pts: f64 }

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
    pub fn open(path: &str, want_hw: bool) -> Result<Decoder, String> {
        ff::init().map_err(|e| e.to_string())?;
        unsafe { sys::av_log_set_level(sys::AV_LOG_ERROR as i32) };
        let input = ff::format::input(&path).map_err(|e| format!("open {path}: {e}"))?;
        let st = input.streams().best(ff::media::Type::Video).ok_or("no video stream")?;
        let stream = st.index();
        let tb: f64 = st.time_base().into();
        let fr: f64 = st.avg_frame_rate().into();
        let fps = if fr.is_finite() && fr > 1.0 { fr } else { 30.0 };
        let mut ctx = ff::codec::context::Context::from_parameters(st.parameters()).map_err(|e| e.to_string())?;
        let mut hw = false;
        unsafe {
            let c = ctx.as_mut_ptr();
            if want_hw {
                let mut dev: *mut sys::AVBufferRef = ptr::null_mut();
                let r = sys::av_hwdevice_ctx_create(&mut dev, sys::AVHWDeviceType::AV_HWDEVICE_TYPE_VAAPI, ptr::null(), ptr::null_mut(), 0);
                if r >= 0 {
                    (*c).hw_device_ctx = dev;
                    (*c).get_format = Some(get_vaapi_format);
                    (*c).extra_hw_frames = 4;
                    hw = true;
                }
            } else {
                (*c).thread_count = 4;
                (*c).thread_type = sys::FF_THREAD_FRAME as i32 | sys::FF_THREAD_SLICE as i32;
            }
        }
        let dec = ctx.decoder().video().map_err(|e| format!("decoder: {e}"))?;
        let (width, height) = (dec.width(), dec.height());
        Ok(Decoder {
            input, dec, stream, tb, fps, dur: 1.0 / fps, start_ts: None, loop_off: 0.0, last_pts: 0.0,
            state: State::Reading, pending: None, hw, width, height, scaler: None,
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
                    self.pending = Some(Pending { frame, pts });
                    return true;
                }
                Err(ff::Error::Eof) => self.restart(),
                Err(ff::Error::Other { errno }) if errno == libc::EAGAIN => {
                    if self.state == State::Draining { self.restart(); continue; }
                    let mut pkt = ff::Packet::empty();
                    match pkt.read(&mut self.input) {
                        Ok(()) => { if pkt.stream() == self.stream { let _ = self.dec.send_packet(&pkt); } }
                        Err(ff::Error::Eof) => { let _ = self.dec.send_eof(); self.state = State::Draining; }
                        Err(_) => { errs += 1; if errs > 50 { return false; } }
                    }
                }
                Err(_) => { errs += 1; if errs > 50 { return false; } self.restart(); }
            }
        }
    }

    /// Loop: seek back to the start and continue the timeline.
    fn restart(&mut self) {
        unsafe { sys::av_seek_frame(self.input.as_mut_ptr(), -1, 0, sys::AVSEEK_FLAG_BACKWARD as i32); }
        self.dec.flush();
        self.state = State::Reading;
        self.loop_off = self.last_pts + self.dur;
        self.start_ts = None;
    }

    /// Return the newest frame whose presentation time is <= target (media seconds). Earlier due frames
    /// are dropped without any export/upload. None if no new frame is due yet.
    pub fn advance(&mut self, target: f64) -> Option<Frame> {
        let mut cand: Option<Pending> = None;
        loop {
            if self.pending.is_none() && !self.decode_next() { break; }
            let p = self.pending.as_ref().unwrap();
            if p.pts <= target + self.dur * 0.5 { cand = self.pending.take(); } else { break; }
        }
        let p = cand?;
        self.make(p)
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
        Some(Frame { pts: p.pts, width: w, height: h, matrix, full_range, data })
    }
}
