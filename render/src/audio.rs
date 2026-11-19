//! Audio for video wallpapers: ffmpeg decode + resample to S16 stereo 48 kHz, played through
//! PipeWire's PulseAudio server (libpulse-simple, blocking writes on one thread).
//! The video clock is published by the main thread; the audio thread drops/pads samples when
//! the two drift apart. Loops with the video, pauses when presentation is paused.
use ffmpeg_next as ff;
use std::ffi::{c_char, c_int, c_void, CString};
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

const RATE: u32 = 48000;

#[repr(C)]
struct SampleSpec { format: c_int, rate: u32, channels: u8 }
#[repr(C)]
struct BufferAttr { maxlength: u32, tlength: u32, prebuf: u32, minreq: u32, fragsize: u32 }

#[link(name = "pulse-simple")]
#[link(name = "pulse")]
extern "C" {
    fn pa_simple_new(server: *const c_char, name: *const c_char, dir: c_int, dev: *const c_char, stream: *const c_char,
        ss: *const SampleSpec, map: *const c_void, attr: *const BufferAttr, err: *mut c_int) -> *mut c_void;
    fn pa_simple_write(s: *mut c_void, data: *const c_void, bytes: usize, err: *mut c_int) -> c_int;
    fn pa_simple_flush(s: *mut c_void, err: *mut c_int) -> c_int;
    fn pa_simple_free(s: *mut c_void);
}

pub struct Shared {
    paused: AtomicBool,
    stop: AtomicBool,
    /// Position of the presented video frame inside the current loop, microseconds (-1 = unknown).
    video_pos_us: AtomicI64,
}

pub struct Audio { sh: Arc<Shared>, th: Option<JoinHandle<()>> }

impl Audio {
    /// None when the file has no audio stream (or volume is 0 / silent).
    pub fn start(path: &str, volume: u32) -> Option<Audio> {
        if volume == 0 { return None; }
        let input = ff::format::input(&path).ok()?;
        input.streams().best(ff::media::Type::Audio)?;
        let loop_secs = input.duration() as f64 / 1e6;
        drop(input);
        let sh = Arc::new(Shared { paused: AtomicBool::new(true), stop: AtomicBool::new(false), video_pos_us: AtomicI64::new(-1) });
        let (sh2, p) = (sh.clone(), path.to_string());
        let th = std::thread::Builder::new().name("audio".into())
            .spawn(move || { if let Err(e) = run(&p, volume, loop_secs, &sh2) { eprintln!("wallforge-render: audio disabled: {e}"); } }).ok()?;
        Some(Audio { sh, th: Some(th) })
    }
    pub fn set_paused(&self, p: bool) { self.sh.paused.store(p, Ordering::Relaxed); }
    pub fn set_video_pos(&self, secs: f64) { self.sh.video_pos_us.store((secs * 1e6) as i64, Ordering::Relaxed); }
}

impl Drop for Audio {
    fn drop(&mut self) {
        self.sh.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.th.take() { let _ = t.join(); }
    }
}

fn wait_unpaused(sh: &Shared, pa: *mut c_void) -> bool {
    let mut flushed = false;
    while sh.paused.load(Ordering::Relaxed) {
        if sh.stop.load(Ordering::Relaxed) { return false; }
        if !flushed { unsafe { let mut e = 0; pa_simple_flush(pa, &mut e); } flushed = true; }
        std::thread::sleep(Duration::from_millis(40));
    }
    !sh.stop.load(Ordering::Relaxed)
}

fn run(path: &str, volume: u32, loop_secs: f64, sh: &Shared) -> Result<(), String> {
    let mut input = ff::format::input(&path).map_err(|e| e.to_string())?;
    let st = input.streams().best(ff::media::Type::Audio).ok_or("no audio")?;
    let idx = st.index();
    let ctx = ff::codec::context::Context::from_parameters(st.parameters()).map_err(|e| e.to_string())?;
    let mut dec = ctx.decoder().audio().map_err(|e| e.to_string())?;
    let loop_samples: u64 = if loop_secs > 0.0 { (loop_secs * RATE as f64) as u64 } else { u64::MAX };

    let ss = SampleSpec { format: 3 /* S16LE */, rate: RATE, channels: 2 };
    let bytes_per_s = (RATE * 4) as u32;
    let attr = BufferAttr { maxlength: u32::MAX, tlength: bytes_per_s / 10, prebuf: u32::MAX, minreq: u32::MAX, fragsize: u32::MAX };
    let (name, sname) = (CString::new("wallforge-render").unwrap(), CString::new("wallpaper").unwrap());
    let mut err = 0;
    let pa = unsafe { pa_simple_new(ptr::null(), name.as_ptr(), 1, ptr::null(), sname.as_ptr(), &ss, ptr::null(), &attr, &mut err) };
    if pa.is_null() { return Err(format!("cannot connect to audio server (pa error {err})")); }
    struct Pa(*mut c_void);
    impl Drop for Pa { fn drop(&mut self) { unsafe { pa_simple_free(self.0) } } }
    let pa = Pa(pa);
    let gain = (volume.min(100) as f32 / 100.0).powi(2); // perceptual-ish curve

    let dbg = std::env::var_os("WALLFORGE_AUDIO_DEBUG").is_some();
    let latency = 0.1f64; // tlength
    let mut swr: Option<ff::software::resampling::Context> = None;
    let mut written: u64 = 0; // samples (per channel) written this loop
    let mut grace = 0u32; // chunks to wait before drift checks (after start/resume/loop)
    let mut frame = ff::frame::Audio::empty();
    let mut out = ff::frame::Audio::empty();
    let mut was_paused = true;
    let mut eof = false;

    // Write interleaved stereo i16 samples, applying gain; returns false to quit.
    let write = |buf: &mut [i16]| -> bool {
        if gain < 0.999 { for s in buf.iter_mut() { *s = (*s as f32 * gain) as i16; } }
        let mut e = 0;
        unsafe { pa_simple_write(pa.0, buf.as_ptr() as *const c_void, buf.len() * 2, &mut e) >= 0 }
    };

    loop {
        if sh.stop.load(Ordering::Relaxed) { return Ok(()); }
        if sh.paused.load(Ordering::Relaxed) {
            if !wait_unpaused(sh, pa.0) { return Ok(()); }
        }
        if was_paused { was_paused = false; grace = 50; }

        // Pull one chunk of decoded, resampled samples (or silence padding at EOF).
        let mut chunk: Vec<i16>;
        if !eof {
            match dec.receive_frame(&mut frame) {
                Ok(()) => {
                    if swr.is_none() {
                        let lay = if frame.channel_layout().is_empty() { ff::ChannelLayout::default(frame.channels() as i32) } else { frame.channel_layout() };
                        swr = Some(ff::software::resampling::Context::get(frame.format(), lay, frame.rate(),
                            ff::format::Sample::I16(ff::format::sample::Type::Packed), ff::ChannelLayout::STEREO, RATE).map_err(|e| e.to_string())?);
                    }
                    if frame.channel_layout().is_empty() { frame.set_channel_layout(ff::ChannelLayout::default(frame.channels() as i32)); }
                    if swr.as_mut().unwrap().run(&frame, &mut out).is_err() { continue; }
                    let n = out.samples();
                    if n == 0 { continue; }
                    let data = out.data(0);
                    chunk = data[..n * 4].chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
                }
                Err(ff::Error::Other { errno }) if errno == libc::EAGAIN => {
                    let mut pkt = ff::Packet::empty();
                    match pkt.read(&mut input) {
                        Ok(()) => { if pkt.stream() == idx { let _ = dec.send_packet(&pkt); } }
                        Err(ff::Error::Eof) => { let _ = dec.send_eof(); }
                        Err(_) => { eof = true; }
                    }
                    continue;
                }
                Err(_) => { eof = true; continue; }
            }
        } else {
            // Audio shorter than the video loop: pad with silence up to the loop length.
            let n = ((loop_samples - written.min(loop_samples)).min(1024)) as usize;
            chunk = vec![0i16; n * 2];
        }

        // Truncate at the loop boundary.
        let mut n = chunk.len() / 2;
        let remain = loop_samples.saturating_sub(written) as usize;
        if n > remain { n = remain; chunk.truncate(n * 2); }

        // Drift correction against the video clock.
        if grace > 0 { grace -= 1; }
        else if let vp @ 0.. = sh.video_pos_us.load(Ordering::Relaxed) {
            let apos = written as f64 / RATE as f64 - latency;
            let mut d = apos - vp as f64 / 1e6;
            if loop_secs > 0.0 { if d > loop_secs / 2.0 { d -= loop_secs } else if d < -loop_secs / 2.0 { d += loop_secs } }
            if dbg { eprintln!("audio drift {d:+.3}s"); }
            if d > 0.12 {
                // audio ahead: insert silence
                let mut sil = vec![0i16; ((d.min(1.0)) * RATE as f64) as usize * 2];
                if !write(&mut sil) { return Ok(()); }
                grace = 50;
            } else if d < -0.12 {
                // audio behind: drop samples
                let drop_n = ((-d).min(1.0) * RATE as f64) as usize;
                if drop_n >= n { written += n as u64; grace = 20; if written >= loop_samples { /* fallthrough to loop below */ } else { continue; } }
                else { chunk.drain(..drop_n * 2); written += drop_n as u64; n -= drop_n; grace = 50; }
            }
        }

        if n > 0 {
            if !write(&mut chunk) { return Ok(()); }
            written += n as u64;
        }

        if written >= loop_samples || (eof && loop_samples == u64::MAX) {
            // loop: seek back, reset decoder/resampler
            let _ = input.seek(0, ..0);
            dec.flush();
            swr = None;
            written = 0; eof = false; grace = 50;
        }
    }
}
