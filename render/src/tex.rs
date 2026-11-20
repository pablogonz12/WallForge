//! Wallpaper Engine .tex parser + GL upload with minimal VRAM.
//! Layout (public docs: RePKG): "TEXV0005\0" "TEXI0001\0", u32 format, u32 flags, u32 texW, texH, imgW, imgH, u32 unk;
//! "TEXB000n\0", u32 imageCount, [n>=3: u32 freeImageFormat], [n==4: u32 isVideoMp4]; per image: u32 mipCount; per mip:
//! v1: w,h,size,data  v2/v3(/v4 non-mp4): w,h,u32 lz4,u32 decompressedSize,u32 size,data  v4-mp4: 3 params + cstring + 1, then as v3.
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Fmt { Rgba8, Dxt1, Dxt3, Dxt5, Rg88, R8 }

pub struct Mip { pub w: u32, pub h: u32, pub data: Vec<u8> }

pub enum Payload {
    Raw { fmt: Fmt, mips: Vec<Mip> }, // largest first
    Image(Vec<u8>),                   // embedded png/jpeg/gif/bmp file
    Mp4(Vec<u8>),                     // embedded video (not supported by the scene renderer yet)
}

pub struct Tex {
    pub tex_w: u32, pub tex_h: u32, // allocated (power-of-two padded) size
    pub img_w: u32, pub img_h: u32, // logical image size
    pub flags: u32,
    pub payload: Payload,
    pub frame: Option<(u32, u32)>, // sprite-sheet size of frame 0 (TEXS section)
    pub frames: Vec<SpriteFrame>,
}

/// One sprite-sheet cell in pixels (origin top-left) and how long it is shown.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpriteFrame { pub x: f32, pub y: f32, pub w: f32, pub h: f32, pub dur: f32 }

/// Parses the body of a TEXS section (after the magic): u32 count, u32 w, u32 h, then 32-byte records
/// [u32 image, f32 time, f32 x, f32 y, f32 w, f32 wy, f32 hx, f32 h].
pub fn parse_frames(b: &[u8]) -> Vec<SpriteFrame> { parse_frames_at(b, 12) }
/// TEXS0001 has no w/h words after the count: records start at offset 4.
pub fn parse_frames_at(b: &[u8], hdr: usize) -> Vec<SpriteFrame> {
    let u = |o: usize| b.get(o..o + 4).map(|x| u32::from_le_bytes([x[0], x[1], x[2], x[3]]));
    let ints = hdr == 4; // TEXS0001 stores the rectangle as integers
    let f = |o: usize| u(o).map(|x| if ints && o % 32 != (hdr + 4) % 32 { x as f32 } else { f32::from_bits(x) });
    let Some(n) = u(0) else { return vec![] };
    if n == 0 || n > 4096 { return vec![]; }
    let mut v = vec![];
    for k in 0..n as usize {
        let o = hdr + k * 32;
        let (Some(dur), Some(x), Some(y), Some(w), Some(h)) = (f(o + 4), f(o + 8), f(o + 12), f(o + 16), f(o + 28)) else { return vec![] };
        let (w, h) = (w.abs().max(f(o + 20).unwrap_or(0.0).abs()), h.abs().max(f(o + 24).unwrap_or(0.0).abs()));
        if !(w >= 1.0 && h >= 1.0) { return vec![]; }
        v.push(SpriteFrame { x, y, w, h, dur: if dur > 0.0 { dur } else { 0.1 } });
    }
    v
}

/// Frame index for time `t` (looping over the summed durations).
pub fn frame_at(frames: &[SpriteFrame], t: f32) -> usize {
    let total: f32 = frames.iter().map(|f| f.dur).sum();
    if frames.len() < 2 || total <= 0.0 { return 0; }
    let mut r = t.rem_euclid(total);
    for (i, f) in frames.iter().enumerate() { if r < f.dur { return i; } r -= f.dur; }
    frames.len() - 1
}

pub const FLAG_NO_INTERP: u32 = 1;
pub const FLAG_CLAMP: u32 = 2;
pub const FLAG_GIF: u32 = 4;

struct Rd<'a> { b: &'a [u8], p: usize }
impl<'a> Rd<'a> {
    fn u32(&mut self) -> Result<u32, String> {
        let s = self.b.get(self.p..self.p + 4).ok_or("tex: truncated")?;
        self.p += 4;
        Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }
    fn cstr(&mut self) -> Result<String, String> {
        let rest = &self.b[self.p.min(self.b.len())..];
        let n = rest.iter().position(|&c| c == 0).ok_or("tex: unterminated string")?;
        if n > 64 { return Err("tex: implausible magic".into()); }
        self.p += n + 1;
        Ok(String::from_utf8_lossy(&rest[..n]).into_owned())
    }
    fn bytes(&mut self, n: usize) -> Result<&'a [u8], String> {
        let s = self.b.get(self.p..self.p.checked_add(n).ok_or("tex: bad size")?).ok_or("tex: mip data outside file")?;
        self.p += n;
        Ok(s)
    }
}

fn fmt_of(v: u32) -> Result<Fmt, String> {
    Ok(match v { 0 => Fmt::Rgba8, 4 => Fmt::Dxt5, 6 => Fmt::Dxt3, 7 => Fmt::Dxt1, 8 => Fmt::Rg88, 9 => Fmt::R8,
        _ => return Err(format!("tex: unknown format {v}")) })
}

pub fn mip_size(fmt: Fmt, w: u32, h: u32) -> usize {
    let (w, h) = (w as usize, h as usize);
    match fmt {
        Fmt::Rgba8 => w * h * 4, Fmt::Rg88 => w * h * 2, Fmt::R8 => w * h,
        Fmt::Dxt1 => w.div_ceil(4) * h.div_ceil(4) * 8,
        Fmt::Dxt3 | Fmt::Dxt5 => w.div_ceil(4) * h.div_ceil(4) * 16,
    }
}

pub fn sniff_image(d: &[u8]) -> bool {
    d.starts_with(&[0x89, b'P', b'N', b'G']) || d.starts_with(&[0xff, 0xd8]) || d.starts_with(b"GIF8") || d.starts_with(b"BM")
}

pub fn parse(b: &[u8], header_only: bool) -> Result<Tex, String> {
    let mut r = Rd { b, p: 0 };
    if r.cstr()? != "TEXV0005" { return Err("tex: bad TEXV magic".into()); }
    if r.cstr()? != "TEXI0001" { return Err("tex: bad TEXI magic".into()); }
    let format = r.u32()?; let flags = r.u32()?;
    let (tex_w, tex_h, img_w, img_h) = (r.u32()?, r.u32()?, r.u32()?, r.u32()?);
    let _unk = r.u32()?;
    if tex_w == 0 || tex_h == 0 || tex_w > 16384 || tex_h > 16384 { return Err("tex: bad dimensions".into()); }
    let magic = r.cstr()?;
    let ver: u32 = magic.strip_prefix("TEXB").and_then(|s| s.parse().ok()).ok_or("tex: bad TEXB magic")?;
    if !(1..=4).contains(&ver) { return Err(format!("tex: unsupported container {magic}")); }
    let count = r.u32()?;
    if count == 0 || count > 64 { return Err("tex: bad image count".into()); }
    let (mut fif, mut mp4) = (-1i32, false);
    if ver >= 3 { fif = r.u32()? as i32; }
    if ver == 4 { mp4 = r.u32()? == 1; }
    let mut layout = ver.min(3);
    if ver == 4 && mp4 { layout = 4; }
    // some GIF templates store a header size (the image size) smaller than the stored, padded sprite sheet: trust the stored mip size
    let (tex_w, tex_h) = if layout < 4 && !header_only { match (r.b.get(r.p + 4..r.p + 8), r.b.get(r.p + 8..r.p + 12)) {
        (Some(w), Some(h)) => { let (w, h) = (u32::from_le_bytes([w[0], w[1], w[2], w[3]]), u32::from_le_bytes([h[0], h[1], h[2], h[3]])); if w >= tex_w && h >= tex_h && w <= 16384 && h <= 16384 && b.windows(8).any(|x| x == b"TEXS0001") { (w, h) } else { (tex_w, tex_h) } } _ => (tex_w, tex_h) } } else { (tex_w, tex_h) };
    let frames = b.windows(8).position(|w| w == b"TEXS0003" || w == b"TEXS0002" || w == b"TEXS0001").map(|i| parse_frames_at(&b[i + 9..], if &b[i..i + 8] == b"TEXS0001" { 4 } else { 12 })).unwrap_or_default();
    let frames: Vec<SpriteFrame> = if frames.len() > 1 && frames.iter().all(|f| f.x >= 0.0 && f.y >= 0.0 && f.x + f.w <= tex_w as f32 + 0.5 && f.y + f.h <= tex_h as f32 + 0.5) { frames } else { vec![] };
    let frame = frames.first().map(|f| (f.w as u32, f.h as u32));
    if std::env::var("WF_DBG").is_ok() { eprintln!("tex frame={:?} {}x{} img {}x{}", frame, tex_w, tex_h, img_w, img_h); }
    let mk = |payload| Tex { tex_w, tex_h, img_w, img_h, flags, payload, frame, frames: frames.clone() };
    if header_only { return Ok(mk(Payload::Raw { fmt: fmt_of(format).unwrap_or(Fmt::Rgba8), mips: vec![] })); }
    // Only the first image matters for a still texture (more images = gif/sprite frames; frame 0 is used).
    let nmip = r.u32()?;
    if nmip == 0 || nmip > 32 { return Err("tex: bad mip count".into()); }
    let mut mips = Vec::new();
    for _ in 0..nmip {
        if layout == 4 {
            let (p1, p2) = (r.u32()?, r.u32()?);
            if p1 != 1 || p2 != 2 { return Err("tex: unknown v4 mip params".into()); }
            r.cstr()?; // condition json
            if r.u32()? != 1 { return Err("tex: unknown v4 mip param".into()); }
        }
        let (w, h) = (r.u32()?, r.u32()?);
        let (lz4, dsize) = if layout >= 2 { (r.u32()? == 1, r.u32()? as usize) } else { (false, 0) };
        let n = r.u32()? as usize;
        let raw = r.bytes(n)?;
        let data = if lz4 {
            if dsize > 1 << 29 { return Err("tex: implausible decompressed size".into()); }
            lz4_flex::block::decompress(raw, dsize).map_err(|e| format!("tex: lz4: {e}"))?
        } else { raw.to_vec() };
        mips.push(Mip { w, h, data });
        if fif >= 0 || mp4 { break; } // embedded file: only one payload
    }
    if mp4 { return Ok(mk(Payload::Mp4(mips.remove(0).data))); }
    if mips.len() == 1 && mips[0].data.len() > 12 && &mips[0].data[4..8] == b"ftyp" { return Ok(mk(Payload::Mp4(mips.remove(0).data))); }
    if fif >= 0 || (mips.len() == 1 && sniff_image(&mips[0].data)) {
        if sniff_image(&mips[0].data) { return Ok(mk(Payload::Image(mips.remove(0).data))); }
        if fif >= 0 && format != 0 && format != 4 && format != 6 && format != 7 && format != 8 && format != 9 { return Err("tex: unknown embedded payload".into()); }
    }
    let fmt = fmt_of(format)?;
    for m in &mips {
        if m.data.len() < mip_size(fmt, m.w, m.h) { return Err("tex: mip data shorter than its dimensions".into()); }
    }
    Ok(mk(Payload::Raw { fmt, mips }))
}

/// .tex-json metadata that sits next to a texture.
#[derive(Default, Debug)]
pub struct TexMeta { pub clamp: Option<bool>, pub no_interp: Option<bool>, pub no_mip: Option<bool>, pub autosize: bool }
pub fn parse_meta(j: &Value) -> TexMeta {
    let b = |k: &str| j.get(k).and_then(|v| v.as_bool());
    TexMeta { clamp: b("clampuvs"), no_interp: b("nointerpolation"), no_mip: b("nomip"), autosize: b("autosize").unwrap_or(false) }
}

pub fn decode_image(d: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    let im = image::load_from_memory(d).map_err(|e| format!("tex: image payload: {e}"))?.to_rgba8();
    Ok((im.width(), im.height(), im.into_raw()))
}

// ---------------- CPU helpers (fallbacks and VRAM reduction) ----------------

fn c565(c: u16) -> [u8; 3] {
    let (r, g, b) = ((c >> 11) & 31, (c >> 5) & 63, c & 31);
    [((r << 3) | (r >> 2)) as u8, ((g << 2) | (g >> 4)) as u8, ((b << 3) | (b >> 2)) as u8]
}

/// Software BC1/BC2/BC3 decoder, only for GPUs without S3TC or when downscaling a mip-less compressed texture.
pub fn dxt_to_rgba(fmt: Fmt, w: u32, h: u32, d: &[u8]) -> Vec<u8> {
    let (w, h) = (w as usize, h as usize);
    let mut out = vec![0u8; w * h * 4];
    let bs = if fmt == Fmt::Dxt1 { 8 } else { 16 };
    let bw = w.div_ceil(4);
    for (bi, blk) in d.chunks_exact(bs).enumerate() {
        let (bx, by) = (bi % bw, bi / bw);
        let cb = &blk[bs - 8..];
        let (c0, c1) = (u16::from_le_bytes([cb[0], cb[1]]), u16::from_le_bytes([cb[2], cb[3]]));
        let (p0, p1) = (c565(c0), c565(c1));
        let mut pal = [[0u8; 4]; 4];
        pal[0] = [p0[0], p0[1], p0[2], 255]; pal[1] = [p1[0], p1[1], p1[2], 255];
        if c0 > c1 || fmt != Fmt::Dxt1 {
            for i in 0..3 { pal[2][i] = ((2 * p0[i] as u32 + p1[i] as u32) / 3) as u8; pal[3][i] = ((p0[i] as u32 + 2 * p1[i] as u32) / 3) as u8; }
            pal[2][3] = 255; pal[3][3] = 255;
        } else {
            for i in 0..3 { pal[2][i] = ((p0[i] as u32 + p1[i] as u32) / 2) as u8; }
            pal[2][3] = 255; pal[3] = [0, 0, 0, 0];
        }
        let idx = u32::from_le_bytes([cb[4], cb[5], cb[6], cb[7]]);
        let mut alpha = [255u8; 16];
        if fmt == Fmt::Dxt3 {
            for i in 0..16 { let v = (blk[i / 2] >> (4 * (i % 2))) & 15; alpha[i] = v * 17; }
        } else if fmt == Fmt::Dxt5 {
            let (a0, a1) = (blk[0] as u32, blk[1] as u32);
            let mut ap = [0u32; 8]; ap[0] = a0; ap[1] = a1;
            if a0 > a1 { for i in 1..7 { ap[i + 1] = ((7 - i as u32) * a0 + i as u32 * a1) / 7; } }
            else { for i in 1..5 { ap[i + 1] = ((5 - i as u32) * a0 + i as u32 * a1) / 5; } ap[6] = 0; ap[7] = 255; }
            let bits = u64::from_le_bytes([blk[2], blk[3], blk[4], blk[5], blk[6], blk[7], 0, 0]);
            for i in 0..16 { alpha[i] = ap[((bits >> (3 * i)) & 7) as usize] as u8; }
        }
        for i in 0..16 {
            let (x, y) = (bx * 4 + i % 4, by * 4 + i / 4);
            if x >= w || y >= h { continue; }
            let mut c = pal[((idx >> (2 * i)) & 3) as usize];
            if fmt != Fmt::Dxt1 { c[3] = alpha[i]; }
            out[(y * w + x) * 4..][..4].copy_from_slice(&c);
        }
    }
    out
}

/// 2x2 box downscale of an interleaved image with `ch` channels.
pub fn downscale(w: u32, h: u32, ch: usize, d: &[u8]) -> (u32, u32, Vec<u8>) {
    let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
    let mut o = vec![0u8; nw as usize * nh as usize * ch];
    for y in 0..nh as usize { for x in 0..nw as usize { for c in 0..ch {
        let mut s = 0u32;
        for (dy, dx) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
            let (sx, sy) = ((x * 2 + dx).min(w as usize - 1), (y * 2 + dy).min(h as usize - 1));
            s += d[(sy * w as usize + sx) * ch + c] as u32;
        }
        o[(y * nw as usize + x) * ch + c] = ((s + 2) / 4) as u8;
    } } }
    (nw, nh, o)
}

// ---------------- GL upload ----------------

const GL_DXT1: u32 = 0x83F1; // RGBA S3TC DXT1
const GL_DXT3: u32 = 0x83F2;
const GL_DXT5: u32 = 0x83F3;

pub struct GlTex { pub id: u32, pub w: u32, pub h: u32, pub bytes: usize, pub compressed: bool, pub flags: u32, pub src: (u32, u32), pub uv: [f32; 2] }

/// Fraction of a (power-of-two padded) texture that holds the image: UV scale for the quad.
pub fn uv_scale(img: (u32, u32), tex: (u32, u32)) -> [f32; 2] {
    [(img.0.max(1) as f32 / tex.0.max(1) as f32).min(1.0), (img.1.max(1) as f32 / tex.1.max(1) as f32).min(1.0)]
}

pub fn has_s3tc() -> bool {
    unsafe {
        let mut n = 0; gl::GetIntegerv(gl::NUM_EXTENSIONS, &mut n);
        (0..n as u32).any(|i| {
            let p = gl::GetStringi(gl::EXTENSIONS, i);
            !p.is_null() && std::ffi::CStr::from_ptr(p as *const _).to_bytes() == b"GL_EXT_texture_compression_s3tc"
        })
    }
}

/// Upload with the smallest footprint for an on-screen size of about (want_w, want_h) pixels:
/// the first mip not smaller than that is uploaded alone (no mip chain), compressed data stays compressed.
pub fn upload(t: &Tex, want_w: u32, want_h: u32, meta: &TexMeta, s3tc: bool) -> Result<GlTex, String> {
    let (mut want_w, mut want_h) = (want_w.max(1), want_h.max(1));
    if let Some((fw, fh)) = t.frame { want_w = (want_w as f32 * t.tex_w as f32 / fw as f32) as u32; want_h = (want_h as f32 * t.tex_h as f32 / fh as f32) as u32; }
    let mut flags = t.flags;
    if meta.clamp == Some(true) { flags |= FLAG_CLAMP; }
    if meta.no_interp == Some(true) { flags |= FLAG_NO_INTERP; }
    let id;
    let (mut ow, mut oh, mut bytes, mut compressed) = (0u32, 0u32, 0usize, false);
    let gen = |_: &dyn Fn()| unsafe { let mut x = 0; gl::GenTextures(1, &mut x); gl::BindTexture(gl::TEXTURE_2D, x); x };
    let set_params = |_: bool| unsafe {
        let f = if flags & FLAG_NO_INTERP != 0 { gl::NEAREST } else { gl::LINEAR } as i32;
        gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, f);
        gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, f);
        let w = if flags & FLAG_CLAMP != 0 { gl::CLAMP_TO_EDGE } else { gl::REPEAT } as i32;
        gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_S, w);
        gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_T, w);
        gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_BASE_LEVEL, 0);
        gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAX_LEVEL, 0);
    };
    let src = (t.img_w.max(1), t.img_h.max(1));
    let mut uv = [1.0f32, 1.0];
    match &t.payload {
        Payload::Mp4(_) => return Err("tex: embedded video textures are not supported".into()),
        Payload::Raw { fmt, mips } if !mips.is_empty() => {
            // choose level: last one still >= wanted size
            let mut li = 0;
            for (i, m) in mips.iter().enumerate() { if m.w >= want_w && m.h >= want_h { li = i; } }
            let m = &mips[li];
            uv = uv_scale((t.img_w, t.img_h), (mips[0].w, mips[0].h));
            if let Some((fw, fh)) = t.frame { uv = [fw as f32 / t.tex_w as f32, fh as f32 / t.tex_h as f32]; }
            let (mut w, mut h, mut data, mut fm) = (m.w, m.h, m.data.clone(), *fmt);
            let is_dxt = matches!(fm, Fmt::Dxt1 | Fmt::Dxt3 | Fmt::Dxt5);
            // no smaller mip stored and a big reduction wanted: shrink on the CPU (decode DXT first)
            let mut shrink = 0;
            if li == mips.len() - 1 { let (mut cw, mut ch) = (w, h); while cw / 2 >= want_w && ch / 2 >= want_h && cw > 1 && ch > 1 && shrink < 3 { cw /= 2; ch /= 2; shrink += 1; } }
            if is_dxt && (!s3tc || shrink >= 2) {
                data = dxt_to_rgba(fm, w, h, &data); fm = Fmt::Rgba8;
            }
            let ch = match fm { Fmt::Rgba8 => 4, Fmt::Rg88 => 2, Fmt::R8 => 1, _ => 0 };
            if ch > 0 { for _ in 0..shrink { let r = downscale(w, h, ch, &data); w = r.0; h = r.1; data = r.2; } }
            id = gen(&|| ());
            set_params(false);
            unsafe {
                match fm {
                    Fmt::Rgba8 => { gl::TexImage2D(gl::TEXTURE_2D, 0, gl::RGBA8 as i32, w as i32, h as i32, 0, gl::RGBA, gl::UNSIGNED_BYTE, data.as_ptr() as *const _); bytes = w as usize * h as usize * 4; }
                    Fmt::Rg88 => { gl::TexImage2D(gl::TEXTURE_2D, 0, gl::RG8 as i32, w as i32, h as i32, 0, gl::RG, gl::UNSIGNED_BYTE, data.as_ptr() as *const _); bytes = w as usize * h as usize * 2; }
                    Fmt::R8 => {
                        gl::TexImage2D(gl::TEXTURE_2D, 0, gl::R8 as i32, w as i32, h as i32, 0, gl::RED, gl::UNSIGNED_BYTE, data.as_ptr() as *const _);
                        let sw = [gl::RED as i32, gl::RED as i32, gl::RED as i32, gl::RED as i32];
                        gl::TexParameteriv(gl::TEXTURE_2D, 0x8E46, sw.as_ptr()); // TEXTURE_SWIZZLE_RGBA
                        bytes = w as usize * h as usize;
                    }
                    _ => {
                        let ifmt = match fm { Fmt::Dxt1 => GL_DXT1, Fmt::Dxt3 => GL_DXT3, _ => GL_DXT5 };
                        let n = mip_size(fm, w, h);
                        gl::CompressedTexImage2D(gl::TEXTURE_2D, 0, ifmt, w as i32, h as i32, 0, n as i32, data.as_ptr() as *const _);
                        bytes = n; compressed = true;
                    }
                }
            }
            ow = w; oh = h;
        }
        Payload::Raw { .. } => return Err("tex: no image data".into()),
        Payload::Image(d) => {
            let (mut w, mut h, mut px) = decode_image(d)?;
            let mut n = 0;
            while w / 2 >= want_w && h / 2 >= want_h && w > 1 && h > 1 && n < 4 { let r = downscale(w, h, 4, &px); w = r.0; h = r.1; px = r.2; n += 1; }
            id = gen(&|| ());
            set_params(false);
            unsafe { gl::TexImage2D(gl::TEXTURE_2D, 0, gl::RGBA8 as i32, w as i32, h as i32, 0, gl::RGBA, gl::UNSIGNED_BYTE, px.as_ptr() as *const _); }
            bytes = w as usize * h as usize * 4; ow = w; oh = h;
        }
    }
    Ok(GlTex { id, w: ow, h: oh, bytes, compressed, flags, src, uv })
}

/// Embedded video texture: returns the GL texture description and the player that keeps it updated.
pub fn upload_video(t: &Tex, want_w: u32, want_h: u32, meta: &TexMeta) -> Result<(GlTex, crate::video::VideoTex), String> {
    let Payload::Mp4(b) = &t.payload else { return Err("tex: not a video".into()) };
    let mut flags = t.flags;
    if meta.clamp == Some(true) { flags |= FLAG_CLAMP; }
    if meta.no_interp == Some(true) { flags |= FLAG_NO_INTERP; }
    let v = crate::video::VideoTex::open(b, (want_w, want_h), flags & FLAG_CLAMP != 0, flags & FLAG_NO_INTERP != 0)?;
    let (w, h) = v.size();
    Ok((GlTex { id: v.tex, w, h, bytes: w as usize * h as usize * 4, compressed: false, flags, src: (t.img_w.max(1), t.img_h.max(1)), uv: [1.0, 1.0] }, v))
}

// ---------------- synthetic generators (tests + --make-test-scene) ----------------

pub mod synth {
    use super::*;
    fn s(v: &mut Vec<u8>, t: &str) { v.extend(t.as_bytes()); v.push(0); }
    pub fn tex(format: u32, flags: u32, tw: u32, th: u32, iw: u32, ih: u32, container: u32, fif: i32, mips: &[(u32, u32, Vec<u8>, bool)]) -> Vec<u8> {
        let mut v = Vec::new();
        s(&mut v, "TEXV0005"); s(&mut v, "TEXI0001");
        for x in [format, flags, tw, th, iw, ih, 0] { v.extend(x.to_le_bytes()); }
        s(&mut v, &format!("TEXB{container:04}"));
        v.extend(1u32.to_le_bytes());
        if container >= 3 { v.extend((fif as u32).to_le_bytes()); }
        if container == 4 { v.extend(0u32.to_le_bytes()); }
        v.extend((mips.len() as u32).to_le_bytes());
        for (w, h, d, lz) in mips {
            v.extend(w.to_le_bytes()); v.extend(h.to_le_bytes());
            if container >= 2 {
                let c = if *lz { lz4_flex::block::compress(d) } else { d.clone() };
                v.extend((*lz as u32).to_le_bytes()); v.extend((d.len() as u32).to_le_bytes());
                v.extend((c.len() as u32).to_le_bytes()); v.extend(c);
            } else { v.extend((d.len() as u32).to_le_bytes()); v.extend(d); }
        }
        v
    }
    pub fn rgba_gradient(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
        let mut d = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h { for x in 0..w { d.extend(f(x, y)); } }
        d
    }
    pub fn png(w: u32, h: u32, px: &[u8]) -> Vec<u8> {
        let mut out = std::io::Cursor::new(Vec::new());
        image::write_buffer_with_format(&mut out, px, w, h, image::ExtendedColorType::Rgba8, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn sprite_frames_parse_and_loop() {
        let mut b = vec![];
        for v in [2u32, 100, 50] { b.extend(v.to_le_bytes()); }
        for (x, dur) in [(0.0f32, 0.5f32), (100.0, 0.25)] {
            b.extend(0u32.to_le_bytes());
            for f in [dur, x, 0.0, 100.0, 0.0, 0.0, 50.0] { b.extend(f.to_le_bytes()); }
        }
        let fr = parse_frames(&b);
        assert_eq!(fr.len(), 2);
        assert_eq!((fr[1].x, fr[1].w, fr[1].h), (100.0, 100.0, 50.0));
        assert_eq!(frame_at(&fr, 0.0), 0);
        assert_eq!(frame_at(&fr, 0.6), 1);
        assert_eq!(frame_at(&fr, 0.8), 0); // loops after 0.75 s
        assert!(parse_frames(&b[..20]).is_empty());
    }

    use super::*;
    #[test]
    fn rgba_v1_and_v3_lz4() {
        let px = synth::rgba_gradient(8, 4, |x, y| [x as u8 * 30, y as u8 * 60, 7, 255]);
        let t = parse(&synth::tex(0, 3, 8, 4, 8, 4, 1, -1, &[(8, 4, px.clone(), false)]), false).unwrap();
        assert_eq!((t.tex_w, t.img_h, t.flags), (8, 4, 3));
        match &t.payload { Payload::Raw { fmt: Fmt::Rgba8, mips } => assert_eq!(mips[0].data, px), _ => panic!() }
        let t = parse(&synth::tex(0, 0, 8, 4, 8, 4, 3, -1, &[(8, 4, px.clone(), true), (4, 2, px[..32].to_vec(), true)]), false).unwrap();
        match &t.payload { Payload::Raw { mips, .. } => { assert_eq!(mips.len(), 2); assert_eq!(mips[0].data, px); assert_eq!(mips[1].w, 4); } _ => panic!() }
    }
    #[test]
    fn dxt_and_small_formats() {
        // DXT1 block: red (c0) / blue (c1), all indices 0 -> every pixel red
        let blk = [0x00, 0xf8, 0x1f, 0x00, 0, 0, 0, 0];
        let t = parse(&synth::tex(7, 0, 4, 4, 4, 4, 2, -1, &[(4, 4, blk.to_vec(), false)]), false).unwrap();
        let Payload::Raw { fmt, mips } = &t.payload else { panic!() };
        assert_eq!(*fmt, Fmt::Dxt1);
        let rgba = dxt_to_rgba(*fmt, 4, 4, &mips[0].data);
        assert_eq!(&rgba[..4], &[255, 0, 0, 255]);
        // DXT5 with alpha endpoints 255/0, index 0 -> alpha 255
        let mut b5 = vec![255, 0, 0, 0, 0, 0, 0, 0]; b5.extend(blk);
        let rgba = dxt_to_rgba(Fmt::Dxt5, 4, 4, &b5);
        assert_eq!(&rgba[60..64], &[255, 0, 0, 255]);
        let t = parse(&synth::tex(9, 0, 2, 2, 2, 2, 2, -1, &[(2, 2, vec![1, 2, 3, 4], false)]), false).unwrap();
        assert!(matches!(t.payload, Payload::Raw { fmt: Fmt::R8, .. }));
        let t = parse(&synth::tex(8, 0, 2, 1, 2, 1, 2, -1, &[(2, 1, vec![1, 2, 3, 4], false)]), false).unwrap();
        assert!(matches!(t.payload, Payload::Raw { fmt: Fmt::Rg88, .. }));
        assert_eq!(mip_size(Fmt::Dxt5, 5, 5), 64);
    }
    #[test]
    fn embedded_png_and_errors() {
        let px = synth::rgba_gradient(4, 4, |x, y| [x as u8 * 60, y as u8 * 60, 0, 255]);
        let p = synth::png(4, 4, &px);
        let t = parse(&synth::tex(0, 0, 4, 4, 4, 4, 3, 13, &[(4, 4, p.clone(), false)]), false).unwrap();
        let Payload::Image(d) = &t.payload else { panic!() };
        let (w, h, dec) = decode_image(d).unwrap();
        assert_eq!((w, h), (4, 4)); assert_eq!(dec, px);
        assert!(parse(b"junk", false).is_err());
        let mut bad = synth::tex(0, 0, 8, 4, 8, 4, 1, -1, &[(8, 4, vec![0; 128], false)]);
        bad.truncate(bad.len() - 10);
        assert!(parse(&bad, false).is_err());
        assert!(parse(&synth::tex(0, 0, 8, 4, 8, 4, 1, -1, &[(8, 4, vec![0; 8], false)]), false).is_err());
        assert!(parse(&synth::tex(5, 0, 8, 4, 8, 4, 1, -1, &[(8, 4, vec![0; 128], false)]), false).is_err());
    }
    #[test]
    fn meta_and_downscale() {
        let m = parse_meta(&serde_json::json!({"clampuvs": true, "autosize": true}));
        assert_eq!((m.clamp, m.autosize, m.no_mip), (Some(true), true, None));
        let (w, h, d) = downscale(2, 2, 1, &[0, 100, 100, 200]);
        assert_eq!((w, h, d[0]), (1, 1, 100));
    }
}
