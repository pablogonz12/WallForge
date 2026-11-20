//! Text layers: rasterise a string with fontdue into an alpha texture that the engine draws like an image layer.
use serde_json::{Map, Value};
use std::time::Instant;

#[derive(Clone, Debug)]
pub struct TextDef {
    pub value: String, pub script: Option<String>, pub sprops: Map<String, Value>,
    pub font: String, pub pointsize: f32, pub halign: String, pub valign: String, pub padding: f32,
    pub format: Option<String>,
}

/// Parse a scene object of kind text; `resolve` turns `{user, value}` bindings into plain values.
pub fn parse(o: &Value, resolve: &dyn Fn(&Value) -> Value) -> Option<TextDef> {
    let t = o.get("text")?;
    let r = resolve(t);
    let value = match &r { Value::String(s) => s.clone(), Value::Null => String::new(), x => x.to_string() };
    let script = t.get("script").and_then(|s| s.as_str()).map(String::from);
    let mut sprops = Map::new();
    if let Some(m) = t.get("scriptproperties").and_then(|v| v.as_object()) { for (k, v) in m { sprops.insert(k.clone(), resolve(v)); } }
    let s = |k: &str, d: &str| o.get(k).and_then(|v| v.as_str()).unwrap_or(d).to_string();
    let f = |k: &str, d: f32| o.get(k).map(|v| resolve(v)).and_then(|v| v.as_f64()).map(|x| x as f32).unwrap_or(d);
    // The date/time format is an object property in some scene versions and a script property in others.
    let fmt = o.get("format").map(|v| resolve(v)).or_else(|| t.get("format").map(|v| resolve(v))).or_else(|| sprops.get("format").cloned())
        .and_then(|v| v.as_str().map(String::from)).filter(|s| !s.is_empty());
    Some(TextDef { value, script, sprops, font: s("font", ""), pointsize: f("pointsize", 16.0), halign: s("horizontalalign", "center"), valign: s("verticalalign", "center"), padding: f("padding", 0.0), format: fmt })
}

/// Load the layer font; the second value names the font that could not be loaded (custom fonts only).
fn load_font(read: &dyn Fn(&str) -> Option<Vec<u8>>, path: &str) -> (Option<fontdue::Font>, Option<String>) {
    let mut cands: Vec<Vec<u8>> = vec![];
    let mut from_pkg = false;
    if !path.is_empty() { if let Some(d) = read(path) { cands.push(d); from_pkg = true; } }
    if cands.is_empty() {
        if let Ok(o) = std::process::Command::new("fc-match").args(["-f", "%{file}", "sans"]).output() { if let Ok(d) = std::fs::read(String::from_utf8_lossy(&o.stdout).trim()) { cands.push(d); } }
        for p in ["/usr/share/fonts/TTF/DejaVuSans.ttf", "/usr/share/fonts/noto/NotoSans-Regular.ttf", "/usr/share/fonts/liberation/LiberationSans-Regular.ttf", "/usr/share/fonts/dejavu/DejaVuSans.ttf", "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"] { if let Ok(d) = std::fs::read(p) { cands.push(d); } }
    }
    let font = cands.into_iter().find_map(|d| fontdue::Font::from_bytes(d, fontdue::FontSettings::default()).ok());
    // `systemfont_*` are Wallpaper Engine's own system aliases, mapped to a system font on purpose.
    let missing = if path.is_empty() || path.starts_with("systemfont_") || (from_pkg && font.is_some()) { None } else { Some(path.to_string()) };
    (font, missing)
}

/// Pixels per `pointsize` unit. Wallpaper Engine lays text out at 300 dpi (300/72 = 4.167 scene units
/// per point); measured against previews of 3096582889 (FreePixel/MobileFontEDIT, line box ~1 em),
/// 2996464786 (Unibody, line box 1.5 em) and 3047405322 (Quicksand/Anurati) this multiplier keeps the
/// glyphs within ~10% of the preview heights instead of the ~2x-too-small previous 2.5.
const PT_PX: f32 = 3.7;

/// Month and weekday names used by date/time formats (English, matching the stock Wallpaper Engine scripts).
const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

fn day(t: &crate::clock::Tm) -> &'static str { DAYS[(t.wday.rem_euclid(7)) as usize] }
fn month(t: &crate::clock::Tm) -> &'static str { MONTHS[(t.mon.rem_euclid(12)) as usize] }
fn h12(t: &crate::clock::Tm) -> i32 { (t.h + 11) % 12 + 1 }

/// `{...}` placeholders: `{year}`, `{month}`, `{day}`, `{weekday}`, `{hour}`, `{minute}`, `{second}`,
/// `{ampm}` and the pattern forms `{yyyy}` `{MM}` `{dd}` `{HH}` `{mm}` `{ss}`.
fn braces(f: &str, t: &crate::clock::Tm) -> String {
    let (mut o, mut rest) = (String::new(), f);
    while let Some(i) = rest.find('{') {
        o += &rest[..i];
        let Some(j) = rest[i..].find('}') else { o += &rest[i..]; return o };
        let key = &rest[i + 1..i + j];
        let rep = match key {
            "year" | "yyyy" | "YYYY" => Some(t.year.to_string()),
            "yy" => Some(format!("{:02}", t.year % 100)),
            "month" | "MMMM" => Some(month(t).to_string()),
            "MMM" => Some(month(t)[..3].to_string()),
            "MM" => Some(format!("{:02}", t.mon + 1)),
            "M" => Some((t.mon + 1).to_string()),
            "weekday" | "dddd" => Some(day(t).to_string()),
            "day" => Some(t.day.to_string()),
            "ddd" => Some(day(t)[..3].to_string()),
            "dd" => Some(format!("{:02}", t.day)),
            "d" => Some(t.day.to_string()),
            "hour" | "HH" => Some(format!("{:02}", t.h)),
            "H" => Some(t.h.to_string()),
            "hh" => Some(format!("{:02}", h12(t))),
            "h" => Some(h12(t).to_string()),
            "minute" | "mi" | "mm" => Some(format!("{:02}", t.m)),
            "second" | "ss" => Some(format!("{:02}", t.s)),
            "ampm" | "AMPM" => Some(if t.h < 12 { "AM" } else { "PM" }.to_string()),
            _ => None,
        };
        match rep { Some(r) => { o += &r; rest = &rest[i + j + 1..] }, None => { o += &rest[i..i + j + 1]; rest = &rest[i + j + 1..] } }
    }
    o += rest; o
}

/// strftime-style `%` specifiers used by date/time text.
fn strftime(f: &str, t: &crate::clock::Tm) -> String {
    let (mut o, mut it) = (String::new(), f.chars());
    while let Some(c) = it.next() {
        if c != '%' { o.push(c); continue }
        match it.next() {
            Some('A') => o += day(t),
            Some('a') => o += &day(t)[..3],
            Some('B') => o += month(t),
            Some('b') => o += &month(t)[..3],
            Some('d') => o += &format!("{:02}", t.day),
            Some('e') => o += &format!("{:2}", t.day),
            Some('m') => o += &format!("{:02}", t.mon + 1),
            Some('y') => o += &format!("{:02}", t.year % 100),
            Some('Y') => o += &t.year.to_string(),
            Some('H') => o += &format!("{:02}", t.h),
            Some('I') => o += &format!("{:02}", h12(t)),
            Some('M') => o += &format!("{:02}", t.m),
            Some('S') => o += &format!("{:02}", t.s),
            Some('p') => o += if t.h < 12 { "AM" } else { "PM" },
            Some('P') => o += if t.h < 12 { "am" } else { "pm" },
            Some('%') => o.push('%'),
            // unknown specifiers are kept verbatim rather than being dropped or printed raw
            Some(x) => { o.push('%'); o.push(x) },
            None => o.push('%'),
        }
    }
    o
}

/// Cafe/Windows-style date patterns (`yyyy`, `MM`, `dd`, `W`, `HH`, `mm`, `ss`, `a`, ...), `\` escapes the next char.
fn cafe(f: &str, t: &crate::clock::Tm) -> String {
    let cs: Vec<char> = f.chars().collect();
    let (mut o, mut i) = (String::new(), 0usize);
    while i < cs.len() {
        let c = cs[i];
        if c == '\\' { if i + 1 < cs.len() { o.push(cs[i + 1]); } i += 2; continue }
        let run = cs[i..].iter().take_while(|&&x| x == c).count();
        let rep = match c {
            'y' => Some(if run >= 3 { t.year.to_string() } else { format!("{:02}", t.year % 100) }),
            'M' => Some(match run { 1 => (t.mon + 1).to_string(), 2 => format!("{:02}", t.mon + 1), 3 => month(t)[..3].to_string(), _ => month(t).to_string() }),
            'd' => Some(match run { 1 => t.day.to_string(), 2 => format!("{:02}", t.day), 3 => day(t)[..3].to_string(), _ => day(t).to_string() }),
            'W' => Some(if run <= 2 { day(t).to_string() } else { day(t)[..3].to_string() }),
            'H' => Some(if run == 1 { t.h.to_string() } else { format!("{:02}", t.h) }),
            'h' => Some(if run == 1 { h12(t).to_string() } else { format!("{:02}", h12(t)) }),
            'm' => Some(if run == 1 { t.m.to_string() } else { format!("{:02}", t.m) }),
            's' => Some(if run == 1 { t.s.to_string() } else { format!("{:02}", t.s) }),
            't' => Some(if t.h < 12 { "AM" } else { "PM" }.to_string()),
            'a' => Some(if t.h < 12 { "am" } else { "pm" }.to_string()),
            'S' => Some("000".to_string()),
            _ => None,
        };
        match rep { Some(r) => { o += &r; i += run }, None => { for _ in 0..run { o.push(c) } i += run } }
    }
    o
}

/// Expand a Wallpaper Engine date/time `format` for the given time; unknown specifiers are left untouched.
pub fn apply_format(fmt: &str, t: &crate::clock::Tm) -> String {
    if fmt.contains('%') { return strftime(&braces(fmt, t), t) }
    let b = braces(fmt, t);
    // a format with brace placeholders is expanded only through them, so expanded names are not re-tokenised
    if fmt.contains('{') { return b }
    cafe(&b, t)
}

/// Rasterise `text` into a w x h RGBA8 buffer (white, alpha = coverage). `k` = texture pixels per scene unit.
pub fn raster(font: &fontdue::Font, d: &TextDef, text: &str, w: u32, h: u32, k: f32) -> Vec<u8> {
    let mut a = vec![0u8; (w * h) as usize];
    // Wallpaper Engine's `padding` is extra geometry around the glyphs (room for text effects), not an inset of
    // the layer box: scenes store a constant 32 even for 14x23 text boxes, so honouring it as a margin would
    // collapse those layers. Text effects are not applied yet, so it is ignored here.
    let pad = 0.0f32;
    let (pad_x, pad_y) = (pad, pad);
    let (avw, avh) = ((w as f32 - 2.0 * pad_x).max(1.0), (h as f32 - 2.0 * pad_y).max(1.0));
    // fonts with a tall line box (ascent+descent > 1 em) are drawn proportionally larger, matching the previews
    let line_em = font.horizontal_line_metrics(100.0).map_or(1.0, |m| (m.ascent - m.descent) / 100.0).clamp(1.0, 2.0);
    let mut px = (d.pointsize * PT_PX * line_em * k).max(1.0);
    // text wider than its (padded) box is shrunk to fit instead of being clipped
    let widest = |px: f32| text.split('\n').map(|l| l.chars().map(|c| font.metrics(c, px).advance_width).sum::<f32>()).fold(0.0f32, f32::max);
    let lw = widest(px); if lw > avw && lw > 0.0 { px = (px * avw / lw * 0.98).max(1.0); }
    let lines: Vec<&str> = text.split('\n').collect();
    // Vertical extents of the actual glyph ink above/below the baseline, so a line box that is taller
    // than its glyphs cannot push text out of the layer box (Wallpaper Engine sizes boxes to the text).
    let ink = |px: f32| -> (f32, f32) {
        let (mut top, mut bot) = (0.0f32, 0.0f32);
        for l in &lines { for c in l.chars() { let m = font.metrics(c, px); if m.height > 0 { top = top.max(m.ymin as f32 + m.height as f32); bot = bot.min(m.ymin as f32) } } }
        (if top > 0.0 { top } else { px * 0.8 }, bot)
    };
    let block = |px: f32| -> f32 {
        let lm = font.horizontal_line_metrics(px).unwrap_or(fontdue::LineMetrics { ascent: px * 0.8, descent: -px * 0.2, line_gap: 0.0, new_line_size: px });
        let lh = lm.new_line_size.max(px);
        let (it, ib) = ink(px);
        (lines.len().saturating_sub(1)) as f32 * lh + (it - ib)
    };
    // ink taller than the layer box is scaled down until it fits (padding alone must not shrink text)
    for _ in 0..3 {
        let b = block(px);
        if b <= h as f32 * 0.98 { break }
        px = (px * h as f32 * 0.96 / b).max(1.0);
    }
    let lm = font.horizontal_line_metrics(px).unwrap_or(fontdue::LineMetrics { ascent: px * 0.8, descent: -px * 0.2, line_gap: 0.0, new_line_size: px });
    let lh = lm.new_line_size.max(px);
    let (ink_top, _) = ink(px);
    // baseline of the first line: the ink block is placed in the padded box, then the baseline sits `ink_top` below its top
    let base0 = match d.valign.as_str() {
        "top" => pad_y + ink_top,
        "bottom" => h as f32 - pad_y - block(px) + ink_top,
        _ => pad_y + (avh - block(px)) * 0.5 + ink_top,
    };
    for (li, line) in lines.iter().enumerate() {
        let glyphs: Vec<(fontdue::Metrics, Vec<u8>)> = line.chars().map(|c| font.rasterize(c, px)).collect();
        let lw: f32 = glyphs.iter().map(|g| g.0.advance_width).sum();
        let mut x = match d.halign.as_str() { "left" => pad_x, "right" => w as f32 - pad_x - lw, _ => pad_x + (avw - lw) * 0.5 };
        let base = base0 + li as f32 * lh;
        for (m, bmp) in &glyphs {
            let (gx, gy) = ((x + m.xmin as f32).round() as i32, (base - m.height as f32 - m.ymin as f32).round() as i32);
            for yy in 0..m.height as i32 { for xx in 0..m.width as i32 {
                let (tx, ty) = (gx + xx, gy + yy);
                if tx < 0 || ty < 0 || tx >= w as i32 || ty >= h as i32 { continue; }
                let c = bmp[(yy as usize) * m.width + xx as usize]; let i = (ty as u32 * w + tx as u32) as usize;
                a[i] = a[i].max(c);
            } }
            x += m.advance_width;
        }
    }
    let mut out = vec![255u8; (w * h * 4) as usize];
    for (i, c) in a.iter().enumerate() { out[i * 4 + 3] = *c; }
    out
}

pub fn upload(tex: u32, w: u32, h: u32, rgba: &[u8]) -> u32 {
    unsafe {
        let mut t = tex;
        if t == 0 { gl::GenTextures(1, &mut t); }
        gl::BindTexture(gl::TEXTURE_2D, t);
        gl::TexImage2D(gl::TEXTURE_2D, 0, gl::RGBA8 as i32, w as i32, h as i32, 0, gl::RGBA, gl::UNSIGNED_BYTE, rgba.as_ptr() as *const _);
        for (p, v) in [(gl::TEXTURE_MIN_FILTER, gl::LINEAR), (gl::TEXTURE_MAG_FILTER, gl::LINEAR), (gl::TEXTURE_WRAP_S, gl::CLAMP_TO_EDGE), (gl::TEXTURE_WRAP_T, gl::CLAMP_TO_EDGE)] { gl::TexParameteri(gl::TEXTURE_2D, p, v as i32); }
        t
    }
}

/// A text layer's live state.
pub struct Live { pub def: TextDef, font: Option<fontdue::Font>, pub w: u32, pub h: u32, k: f32, pub tex: u32, pub layer: usize, last: String, checked: Instant, dynamic: bool }

impl Live {
    /// Returns the layer and, when a custom font could not be loaded, the name of that font.
    pub fn new(def: TextDef, read: &dyn Fn(&str) -> Option<Vec<u8>>, w: u32, h: u32, k: f32, layer: usize) -> (Live, Option<String>) {
        let (font, missing) = load_font(read, &def.font);
        let dynamic = def.format.is_some() || def.script.as_deref().map_or(false, |s| s.contains("new Date"));
        let mut l = Live { def, font, w, h, k, tex: 0, layer, last: String::new(), checked: Instant::now(), dynamic };
        l.last = l.current(); l.draw(); (l, missing)
    }
    fn current(&self) -> String {
        // A `format` property wins: the stock date scripts cannot be evaluated, but their pattern can.
        if let Some(f) = &self.def.format { return apply_format(f, &crate::clock::now()); }
        if self.dynamic { if let Some(s) = crate::clock::eval(self.def.script.as_deref().unwrap_or(""), &self.def.sprops) { return s; } }
        self.def.value.clone()
    }
    fn draw(&mut self) {
        let Some(f) = self.font.as_ref().filter(|_| !self.last.is_empty()) else { self.tex = upload(self.tex, 1, 1, &[255, 255, 255, 0]); return; };
        let rgba = raster(f, &self.def, &self.last, self.w, self.h, self.k);
        self.tex = upload(self.tex, self.w, self.h, &rgba);
    }
    /// Re-evaluates a time-driven string at most once a second; true when the texture changed.
    pub fn refresh(&mut self) -> bool {
        if !self.dynamic || self.checked.elapsed().as_millis() < 1000 { return false; }
        self.checked = Instant::now();
        let s = self.current();
        if s == self.last { return false; }
        self.last = s; self.draw(); true
    }
    pub fn is_dynamic(&self) -> bool { self.dynamic }
    pub fn next_check(&self) -> Instant { self.checked + std::time::Duration::from_secs(1) }
}

/// The glyph texture lives in the engine's layer too; free it when the layer is rebuilt.
impl Drop for Live {
    fn drop(&mut self) { if self.tex != 0 { unsafe { gl::DeleteTextures(1, &self.tex); } } }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::Tm;
    fn t() -> Tm { Tm { h: 9, m: 5, s: 7, day: 8, mon: 9, year: 2023, wday: 0 } }
    #[test]
    fn strftime_specifiers() {
        assert_eq!(apply_format("%A %d %B %Y %H:%M:%S %p", &t()), "Sunday 08 October 2023 09:05:07 AM");
        // unknown specifiers stay as they are, a literal %% is a percent sign
        assert_eq!(apply_format("%q 100%%", &t()), "%q 100%");
    }
    #[test]
    fn cafe_format_placeholders() {
        assert_eq!(apply_format("W/dd/MM/yyyy", &t()), "Sunday/08/10/2023");
        assert_eq!(apply_format("hh:mm", &t()), "09:05");
        assert_eq!(apply_format("dd MMM yyyy", &t()), "08 Oct 2023");
    }
    #[test]
    fn brace_placeholders() {
        assert_eq!(apply_format("{weekday} {day} {month} {year}", &t()), "Sunday 8 October 2023");
        assert_eq!(apply_format("{yyyy}-{MM}-{dd} {HH}:{mm}", &t()), "2023-10-08 09:05");
        assert_eq!(apply_format("{unknown} {year}", &t()), "{unknown} 2023");
    }
    #[test]
    fn padding_does_not_move_text() {
        // Scenes store a constant padding (32) even for 14x23 boxes, so it is glyph geometry, not a box inset.
        let Some(f) = load_font(&|_| None, "").0 else { eprintln!("no system font available; test skipped"); return };
        let mk = |padding: f32| TextDef { value: "H".into(), script: None, sprops: Map::new(), font: String::new(), pointsize: 10.0, halign: "left".into(), valign: "top".into(), padding, format: None };
        let (w, h) = (200u32, 100u32);
        let col = |rgba: &[u8]| (0..w as usize).find(|x| (0..h as usize).any(|y| rgba[(y * w as usize + *x) * 4 + 3] > 0)).unwrap();
        let a = col(&raster(&f, &mk(0.0), "H", w, h, 1.0));
        let b = col(&raster(&f, &mk(40.0), "H", w, h, 1.0));
        assert!(a <= 6, "the glyph starts at the left edge, got {a}");
        assert_eq!(a, b, "padding must not shift or shrink the text");
    }
}

