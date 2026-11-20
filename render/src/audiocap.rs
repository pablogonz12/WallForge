//! System-audio capture for audio-reactive scene wallpapers.
//! Records the default sink's monitor through PipeWire's PulseAudio server (libpulse-simple, blocking
//! reads on one thread), runs a 1024-point Hann-windowed FFT per channel and publishes 16/32/64-band
//! spectra. Nothing here touches the render loop: the thread writes a small snapshot that `Engine::tick`
//! copies under a mutex, and every failure path logs once and leaves the spectrum at zero.
use std::ffi::{c_char, c_int, c_void, CString};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

/// Renderer was started with `--no-audio-processing`: never open a capture stream.
pub static DISABLED: AtomicBool = AtomicBool::new(false);

const RATE: u32 = 48000;
const N: usize = 1024; // FFT size (21.3 ms at 48 kHz)
const BANDS: usize = 64;
const F_LO: f32 = 43.0;
const F_HI: f32 = 16000.0;
const DECAY: f32 = 0.75; // per read (~21 ms): fast attack, ~0.4 s release to -60 dB

#[repr(C)]
struct SampleSpec { format: c_int, rate: u32, channels: u8 }
#[repr(C)]
struct BufferAttr { maxlength: u32, tlength: u32, prebuf: u32, minreq: u32, fragsize: u32 }

#[link(name = "pulse-simple")]
#[link(name = "pulse")]
extern "C" {
    fn pa_simple_new(server: *const c_char, name: *const c_char, dir: c_int, dev: *const c_char, stream: *const c_char,
        ss: *const SampleSpec, map: *const c_void, attr: *const BufferAttr, err: *mut c_int) -> *mut c_void;
    fn pa_simple_read(s: *mut c_void, data: *mut c_void, bytes: usize, err: *mut c_int) -> c_int;
    fn pa_simple_free(s: *mut c_void);
}

/// One frame's worth of normalised (0..1) spectrum, shared with the renderer. Plain data, `Copy`.
#[derive(Clone, Copy)]
pub struct Snapshot {
    pub s16l: [f32; 16], pub s16r: [f32; 16], pub s16a: [f32; 16],
    pub s32l: [f32; 32], pub s32r: [f32; 32], pub s32a: [f32; 32],
    pub s64l: [f32; 64], pub s64r: [f32; 64], pub s64a: [f32; 64],
    pub power: f32, pub avg: f32,
}

impl Snapshot {
    pub const ZERO: Snapshot = Snapshot {
        s16l: [0.0; 16], s16r: [0.0; 16], s16a: [0.0; 16],
        s32l: [0.0; 32], s32r: [0.0; 32], s32a: [0.0; 32],
        s64l: [0.0; 64], s64r: [0.0; 64], s64a: [0.0; 64],
        power: 0.0, avg: 0.0,
    };
    /// Bands for `g_AudioSpectrum<size>[Left|Right]`; empty side = average of both channels.
    pub fn spectrum(&self, size: usize, side: &str) -> &[f32] {
        match (size, side) {
            (16, "Left") => &self.s16l, (16, "Right") => &self.s16r, (16, "") => &self.s16a,
            (32, "Left") => &self.s32l, (32, "Right") => &self.s32r, (32, "") => &self.s32a,
            (64, "Left") => &self.s64l, (64, "Right") => &self.s64r, (64, "") => &self.s64a,
            _ => &[],
        }
    }
}

pub struct Capture { sh: Arc<Mutex<Snapshot>>, th: Option<JoinHandle<()>>, stop: Arc<AtomicBool> }

impl Capture {
    /// Starts the monitor-capture thread. Returns None (after one log line) when no Pulse source can
    /// be opened; the caller keeps rendering with a zero spectrum and never retries.
    pub fn start() -> Option<Capture> {
        if DISABLED.load(Ordering::Relaxed) { return None; }
        let sh = Arc::new(Mutex::new(Snapshot::ZERO));
        let stop = Arc::new(AtomicBool::new(false));
        let (sh2, stop2) = (sh.clone(), stop.clone());
        let th = std::thread::Builder::new().name("audio-capture".into())
            .spawn(move || run(&sh2, &stop2)).ok()?;
        Some(Capture { sh, th: Some(th), stop })
    }

    pub fn snapshot(&self) -> Snapshot { self.sh.lock().map(|g| *g).unwrap_or(Snapshot::ZERO) }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.th.take() { let _ = t.join(); } // a blocking read returns within ~21 ms
    }
}

fn monitor_device() -> Option<String> {
    let out = Command::new("pactl").args(["get-default-sink"]).output().ok()?;
    if !out.status.success() { return None; }
    let sink = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if sink.is_empty() { None } else { Some(format!("{sink}.monitor")) }
}

unsafe fn open(dev: &str) -> Result<*mut c_void, c_int> {
    let ss = SampleSpec { format: 3 /* PA_SAMPLE_S16LE */, rate: RATE, channels: 2 };
    // fragsize = one read: 1024 stereo frames; maxlength a couple of fragments of headroom
    let attr = BufferAttr { maxlength: (N * 4 * 3) as u32, tlength: 0, prebuf: 0, minreq: 0, fragsize: (N * 4) as u32 };
    let name = CString::new("wallforge audio capture").unwrap_or_default();
    let d = CString::new(dev).unwrap_or_default();
    let mut err = 0;
    let s = pa_simple_new(std::ptr::null(), name.as_ptr(), 2 /* record */, d.as_ptr(), name.as_ptr(), &ss, std::ptr::null(), &attr, &mut err);
    if s.is_null() { Err(err) } else { Ok(s) }
}

fn run(sh: &Mutex<Snapshot>, stop: &AtomicBool) {
    // Prefer PipeWire/Pulse's pseudo-device, then the default sink's monitor; never the microphone.
    let dev = "@DEFAULT_MONITOR@";
    let mut pa = match unsafe { open(dev) } {
        Ok(p) => p,
        Err(e) => match monitor_device().and_then(|d| unsafe { open(&d) }.ok()) {
            Some(p) => p,
            None => {
                eprintln!("wallforge-render: system audio capture unavailable (pulse error {e}); audio-reactive effects stay silent");
                return;
            }
        },
    };
    let mut an = Analyzer::new();
    let mut pcm = vec![0i16; N * 2];
    let bytes = pcm.len() * 2;
    loop {
        if stop.load(Ordering::Relaxed) { break; }
        let mut err = 0;
        if unsafe { pa_simple_read(pa, pcm.as_mut_ptr() as *mut c_void, bytes, &mut err) } < 0 {
            eprintln!("wallforge-render: audio capture stopped (pulse error {err}); audio-reactive effects stay silent");
            break;
        }
        an.push(&pcm);
        if let Ok(mut g) = sh.lock() { *g = an.snapshot(); }
    }
    unsafe { pa_simple_free(pa); }
}

/// Radix-2 in-place complex FFT with precomputed twiddles and bit-reversal table.
struct Fft { rev: [u16; N], cos: [f32; N / 2], sin: [f32; N / 2] }

impl Fft {
    fn new() -> Fft {
        let mut rev = [0u16; N];
        let bits = N.trailing_zeros();
        for (i, r) in rev.iter_mut().enumerate() { *r = (i as u16).reverse_bits() >> (16 - bits); }
        let mut cos = [0.0f32; N / 2];
        let mut sin = [0.0f32; N / 2];
        for k in 0..N / 2 {
            let a = -2.0 * std::f32::consts::PI * k as f32 / N as f32;
            cos[k] = a.cos(); sin[k] = a.sin();
        }
        Fft { rev, cos, sin }
    }

    fn transform(&self, re: &mut [f32; N], im: &mut [f32; N]) {
        for i in 0..N { let j = self.rev[i] as usize; if j > i { re.swap(i, j); im.swap(i, j); } }
        let mut len = 2;
        while len <= N {
            let half = len / 2;
            let step = N / len;
            let mut base = 0;
            while base < N {
                for k in 0..half {
                    let (wr, wi) = (self.cos[k * step], self.sin[k * step]);
                    let (a, b) = (base + k, base + k + half);
                    let tr = wr * re[b] - wi * im[b];
                    let ti = wr * im[b] + wi * re[b];
                    re[b] = re[a] - tr; im[b] = im[a] - ti;
                    re[a] += tr; im[a] += ti;
                }
                base += len;
            }
            len *= 2;
        }
    }
}

struct Analyzer {
    fft: Fft,
    win: [f32; N],
    re: [f32; N], im: [f32; N],
    mag: [f32; N / 2 + 1],
    band: [(u16, u16); BANDS], // first/last FFT bin of each log-spaced band
    l64: [f32; 64], r64: [f32; 64],
}

impl Analyzer {
    fn new() -> Analyzer {
        let mut win = [0.0f32; N];
        for (i, w) in win.iter_mut().enumerate() {
            *w = 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / (N - 1) as f32).cos();
        }
        let binw = RATE as f32 / N as f32;
        let mut band = [(0u16, 0u16); BANDS];
        for (i, b) in band.iter_mut().enumerate() {
            let f0 = F_LO * (F_HI / F_LO).powf(i as f32 / BANDS as f32);
            let f1 = F_LO * (F_HI / F_LO).powf((i + 1) as f32 / BANDS as f32);
            let lo = ((f0 / binw) as usize).max(1).min(N / 2);
            let hi = (((f1 / binw) as usize).max(lo + 1)).min(N / 2 + 1);
            *b = (lo as u16, hi as u16);
        }
        Analyzer { fft: Fft::new(), win, re: [0.0; N], im: [0.0; N], mag: [0.0; N / 2 + 1], band, l64: [0.0; 64], r64: [0.0; 64] }
    }

    /// Windows one channel, transforms it and fills `mag` with amplitudes (1.0 = full-scale sine).
    fn analyse(&mut self, samples: &[i16], offset: usize) {
        for i in 0..N {
            self.re[i] = samples[i * 2 + offset] as f32 * (1.0 / 32768.0) * self.win[i];
            self.im[i] = 0.0;
        }
        self.fft.transform(&mut self.re, &mut self.im);
        for k in 0..=N / 2 {
            self.mag[k] = (self.re[k] * self.re[k] + self.im[k] * self.im[k]).sqrt() * (2.0 / N as f32);
        }
    }

    fn bands(&self) -> [f32; 64] {
        let mut out = [0.0f32; 64];
        for (i, b) in self.band.iter().enumerate() {
            let (lo, hi) = (b.0 as usize, b.1 as usize);
            let mut s = 0.0;
            for k in lo..hi { s += self.mag[k]; }
            let m = s / (hi - lo) as f32;
            // -70 dB..0 dB -> 0..1, then fast attack / slow release
            let v = ((20.0 * (m + 1e-9).log10() + 70.0) / 70.0).clamp(0.0, 1.0);
            out[i] = v;
        }
        out
    }

    fn push(&mut self, pcm: &[i16]) {
        self.analyse(pcm, 0); let l = self.bands();
        self.analyse(pcm, 1); let r = self.bands();
        for i in 0..64 { self.l64[i] = l[i].max(self.l64[i] * DECAY); self.r64[i] = r[i].max(self.r64[i] * DECAY); }
    }

    fn snapshot(&self) -> Snapshot {
        let mut s = Snapshot::ZERO;
        let (mut p, mut a) = (0.0f32, 0.0f32);
        for i in 0..64 {
            let (l, r) = (self.l64[i], self.r64[i]);
            s.s64l[i] = l; s.s64r[i] = r; s.s64a[i] = (l + r) * 0.5;
            p += s.s64a[i]; a += s.s64a[i];
        }
        for i in 0..32 {
            s.s32l[i] = (s.s64l[2 * i] + s.s64l[2 * i + 1]) * 0.5;
            s.s32r[i] = (s.s64r[2 * i] + s.s64r[2 * i + 1]) * 0.5;
            s.s32a[i] = (s.s32l[i] + s.s32r[i]) * 0.5;
        }
        for i in 0..16 {
            s.s16l[i] = (s.s32l[2 * i] + s.s32l[2 * i + 1]) * 0.5;
            s.s16r[i] = (s.s32r[2 * i] + s.s32r[2 * i + 1]) * 0.5;
            s.s16a[i] = (s.s16l[i] + s.s16r[i]) * 0.5;
        }
        s.power = p / 64.0;
        s.avg = a / 64.0;
        s
    }
}
