//! `--check`: probe a file/dir without opening a window; print one JSON line.
use crate::video::{Data, Decoder};
use ffmpeg_next as ff;
use ffmpeg_sys_next as sys;
use serde_json::{json, Value};

fn fourcc(f: u32) -> String { f.to_le_bytes().iter().map(|&b| if b.is_ascii_graphic() { b as char } else { ' ' }).collect::<String>().trim().to_string() }

pub fn probe(path: &str) -> Result<Value, String> {
    ff::init().map_err(|e| e.to_string())?;
    unsafe { sys::av_log_set_level(sys::AV_LOG_QUIET) };
    let input = ff::format::input(&path).map_err(|e| format!("open {path}: {e}"))?;
    let st = input.streams().best(ff::media::Type::Video).ok_or("no video stream")?;
    let ctx = ff::codec::context::Context::from_parameters(st.parameters()).map_err(|e| e.to_string())?;
    let (codec, profile, fmt, w, h, bits);
    unsafe {
        let par = st.parameters();
        let p = par.as_ptr();
        codec = ff::codec::Id::from((*p).codec_id).name().to_string();
        profile = (*p).profile;
        let pf: sys::AVPixelFormat = std::mem::transmute((*p).format);
        let d = sys::av_pix_fmt_desc_get(pf);
        bits = if d.is_null() { 0 } else { (*d).comp[0].depth };
        let n = sys::av_get_pix_fmt_name(pf);
        fmt = if n.is_null() { String::new() } else { std::ffi::CStr::from_ptr(n).to_string_lossy().into_owned() };
        w = (*p).width; h = (*p).height;
    }
    drop(ctx);
    let fr: f64 = st.avg_frame_rate().into();
    let tb: f64 = st.time_base().into();
    let dur = if st.duration() > 0 { st.duration() as f64 * tb } else { input.duration() as f64 / 1e6 };
    let audio = input.streams().best(ff::media::Type::Audio).map(|a| {
        let par = a.parameters();
        unsafe {
            let p = par.as_ptr();
            json!({"codec": ff::codec::Id::from((*p).codec_id).name(), "channels": (*p).ch_layout.nb_channels, "rate": (*p).sample_rate})
        }
    });
    drop(input);
    // VAAPI: really open the hardware decoder and map the first frame to DRM PRIME
    let (mut vaapi, mut vaapi_err, mut vaapi_format) = (false, Value::Null, Value::Null);
    match Decoder::open(path, true, 30.0) {
        Ok(mut d) if d.hw => match d.advance(0.0) {
            Some(f) => match &f.data {
                Data::Drm(x) => {
                    let desc = x.desc();
                    if desc.nb_layers == 2 { vaapi = true; vaapi_format = json!(format!("{}+{}", fourcc(desc.layers[0].format), fourcc(desc.layers[1].format))); }
                    else { vaapi_err = json!(format!("{} DRM layers (need 2)", desc.nb_layers)); }
                }
                _ => vaapi_err = json!("software frame"),
            },
            None => vaapi_err = json!("VAAPI decoder produced no frame"),
        },
        Ok(_) => vaapi_err = json!("no VAAPI device"),
        Err(e) => vaapi_err = json!(e),
    }
    Ok(json!({"ok": true, "file": path, "codec": codec, "profile": profile, "pix_fmt": fmt, "bit_depth": bits,
        "width": w, "height": h, "fps": (fr * 100.0).round() / 100.0, "duration": (dur * 1000.0).round() / 1000.0,
        "audio": audio, "vaapi": vaapi, "vaapi_format": vaapi_format, "vaapi_error": vaapi_err}))
}
