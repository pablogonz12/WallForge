//! Text layers: rasterise a string with fontdue into an alpha texture that the engine draws like an image layer.
use serde_json::{Map, Value};
use std::time::Instant;

#[derive(Clone, Debug)]
pub struct TextDef {
    pub value: String, pub script: Option<String>, pub sprops: Map<String, Value>,
    pub font: String, pub pointsize: f32, pub halign: String, pub valign: String, pub padding: f32,
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
    Some(TextDef { value, script, sprops, font: s("font", ""), pointsize: f("pointsize", 16.0), halign: s("horizontalalign", "center"), valign: s("verticalalign", "center"), padding: f("padding", 0.0) })
}

fn load_font(read: &dyn Fn(&str) -> Option<Vec<u8>>, path: &str) -> Option<fontdue::Font> {
    let mut cands: Vec<Vec<u8>> = vec![];
    if !path.is_empty() { if let Some(d) = read(path) { cands.push(d); } }
    if cands.is_empty() {
        if let Ok(o) = std::process::Command::new("fc-match").args(["-f", "%{file}", "sans"]).output() { if let Ok(d) = std::fs::read(String::from_utf8_lossy(&o.stdout).trim()) { cands.push(d); } }
        for p in ["/usr/share/fonts/TTF/DejaVuSans.ttf", "/usr/share/fonts/noto/NotoSans-Regular.ttf", "/usr/share/fonts/liberation/LiberationSans-Regular.ttf", "/usr/share/fonts/dejavu/DejaVuSans.ttf", "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"] { if let Ok(d) = std::fs::read(p) { cands.push(d); } }
    }
    cands.into_iter().find_map(|d| fontdue::Font::from_bytes(d, fontdue::FontSettings::default()).ok())
}

/// Pixels per `pointsize` unit (calibrated against previews).
const PT_PX: f32 = 2.5;

/// Rasterise `text` into a w x h RGBA8 buffer (white, alpha = coverage). `k` = texture pixels per scene unit.
pub fn raster(font: &fontdue::Font, d: &TextDef, text: &str, w: u32, h: u32, k: f32) -> Vec<u8> {
    let mut a = vec![0u8; (w * h) as usize];
    // fonts with a tall line box (ascent+descent > 1 em) are drawn proportionally larger, matching the previews
    let line_em = font.horizontal_line_metrics(100.0).map_or(1.0, |m| (m.ascent - m.descent) / 100.0).clamp(1.0, 2.0);
    let mut px = (d.pointsize * PT_PX * line_em * k).max(1.0);
    // text wider than its box is shrunk to fit instead of being clipped
    let widest = |px: f32| text.split('\n').map(|l| l.chars().map(|c| font.metrics(c, px).advance_width).sum::<f32>()).fold(0.0f32, f32::max);
    let lw = widest(px); if lw > w as f32 && lw > 0.0 { px = (px * w as f32 / lw * 0.98).max(1.0); }
    let lm = font.horizontal_line_metrics(px).unwrap_or(fontdue::LineMetrics { ascent: px * 0.8, descent: -px * 0.2, line_gap: 0.0, new_line_size: px });
    let lh = lm.new_line_size.max(px);
    let lines: Vec<&str> = text.split('\n').collect();
    let total = lh * lines.len() as f32;
    let top = match d.valign.as_str() { "top" => 0.0, "bottom" => h as f32 - total, _ => (h as f32 - total) * 0.5 };
    for (li, line) in lines.iter().enumerate() {
        let glyphs: Vec<(fontdue::Metrics, Vec<u8>)> = line.chars().map(|c| font.rasterize(c, px)).collect();
        let lw: f32 = glyphs.iter().map(|g| g.0.advance_width).sum();
        let mut x = match d.halign.as_str() { "left" => 0.0, "right" => w as f32 - lw, _ => (w as f32 - lw) * 0.5 };
        let base = top + li as f32 * lh + lm.ascent + (lh - (lm.ascent - lm.descent)) * 0.5;
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
    pub fn new(def: TextDef, read: &dyn Fn(&str) -> Option<Vec<u8>>, w: u32, h: u32, k: f32, layer: usize) -> Live {
        let font = load_font(read, &def.font);
        let dynamic = def.script.as_deref().map_or(false, |s| s.contains("new Date"));
        let mut l = Live { def, font, w, h, k, tex: 0, layer, last: String::new(), checked: Instant::now(), dynamic };
        l.last = l.current(); l.draw(); l
    }
    fn current(&self) -> String {
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
