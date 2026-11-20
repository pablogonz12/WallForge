//! scene.json model: general / camera / objects (image, sound, particle, light), user-property resolution,
//! material + model json lookups.
use crate::pkg::Vfs;
use serde_json::{Map, Value};
use std::collections::{BTreeSet, HashMap};

pub type Mat4 = [f32; 16]; // column-major
pub fn ident() -> Mat4 { let mut m = [0.0; 16]; m[0] = 1.0; m[5] = 1.0; m[10] = 1.0; m[15] = 1.0; m }
pub fn mul(a: &Mat4, b: &Mat4) -> Mat4 {
    let mut o = [0.0; 16];
    for c in 0..4 { for r in 0..4 { for k in 0..4 { o[c * 4 + r] += a[k * 4 + r] * b[c * 4 + k]; } } }
    o
}
pub fn trs(origin: [f32; 3], scale: [f32; 3], angle_z_deg: f32) -> Mat4 {
    let (s, c) = angle_z_deg.to_radians().sin_cos();
    let mut m = ident();
    m[0] = c * scale[0]; m[1] = s * scale[0];
    m[4] = -s * scale[1]; m[5] = c * scale[1];
    m[10] = scale[2];
    m[12] = origin[0]; m[13] = origin[1]; m[14] = origin[2];
    m
}

// ---------------- user properties ----------------

#[derive(Default)]
pub struct Props { map: HashMap<String, Value> }

fn loose(s: &str) -> Value {
    match s { "true" => Value::Bool(true), "false" => Value::Bool(false), _ => s.parse::<f64>().ok().and_then(serde_json::Number::from_f64).map(Value::Number).unwrap_or_else(|| Value::String(s.to_string())) }
}

impl Props {
    /// `project`: project.json, `overrides`: `--set-property key=value`.
    pub fn load(project: &Value, overrides: &[(String, String)]) -> Props {
        let mut map = HashMap::new();
        if let Some(p) = project.pointer("/general/properties").and_then(|v| v.as_object()) {
            for (k, v) in p { if let Some(val) = v.get("value") { map.insert(k.clone(), val.clone()); } }
        }
        for (k, v) in overrides { map.insert(k.clone(), loose(v)); }
        Props { map }
    }
    fn as_str(v: &Value) -> String { match v { Value::String(s) => s.clone(), Value::Bool(b) => (*b as i32).to_string(), Value::Number(n) => n.to_string(), o => o.to_string() } }
    /// Replace `{"user": ..., "value": default}` bindings by their current value (script-less).
    pub fn resolve(&self, v: &Value) -> Value {
        let Some(o) = v.as_object() else { return v.clone() };
        let default = o.get("value").cloned().unwrap_or(Value::Null);
        match o.get("user") {
            Some(Value::String(name)) => self.map.get(name).cloned().unwrap_or(default),
            Some(Value::Object(u)) => {
                let name = u.get("name").and_then(|n| n.as_str()).unwrap_or("");
                match (self.map.get(name), u.get("condition")) {
                    (Some(pv), Some(cond)) => Value::Bool(Self::as_str(pv) == Self::as_str(cond) || (pv.as_bool() == Some(true) && Self::as_str(cond) == "1")),
                    (Some(pv), None) => pv.clone(),
                    _ => default,
                }
            }
            _ => if o.contains_key("value") { self.script_axes(o, default) } else { v.clone() },
        }
    }
    /// Scripts of the form `value.y = scriptProperties.posY;` only overwrite vector components with script properties: honour those
    /// (anything more elaborate keeps the stored default, scripts are not executed).
    fn script_axes(&self, o: &serde_json::Map<String, Value>, default: Value) -> Value {
        let (Some(Value::String(src)), Some(Value::Object(sp)), Some(f)) = (o.get("script"), o.get("scriptproperties"), floats(&default)) else { return default };
        if f.len() < 2 { return default; }
        let mut f = f; let mut hit = false;
        for line in src.lines() {
            let l = line.trim().trim_end_matches(';').replace(' ', "");
            let Some(r) = l.strip_prefix("value.") else { continue };
            let Some((ax, rhs)) = r.split_once('=') else { continue };
            let Some(name) = rhs.strip_prefix("scriptProperties.") else { continue };
            let i = match ax { "x" => 0, "y" => 1, "z" => 2, _ => continue };
            if i >= f.len() || !name.chars().all(|c| c.is_alphanumeric() || c == '_') { continue; }
            if let Some(v) = sp.get(name).and_then(|v| floats(&self.resolve(v))).and_then(|v| v.first().copied()) { f[i] = v; hit = true; }
        }
        if hit { Value::String(f.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(" ")) } else { default }
    }
    pub fn floats(&self, v: Option<&Value>) -> Option<Vec<f32>> { floats(&self.resolve(v?)) }
    /// Like `boolean`, but honours the common `applyUserProperties` visibility script.
    pub fn visible(&self, v: Option<&Value>) -> bool {
        if let Some(x) = v { if let Some(u) = x.get("user").and_then(|u| u.as_str()) {
            if let Some(b) = crate::scriptvis::script_visible(x, self.map.get(u).and_then(|p| p.as_bool())) { return b; } } }
        self.boolean(v, true)
    }
    pub fn boolean(&self, v: Option<&Value>, d: bool) -> bool {
        match v.map(|x| self.resolve(x)) {
            Some(Value::Bool(b)) => b, Some(Value::Number(n)) => n.as_f64().unwrap_or(0.0) != 0.0,
            Some(Value::String(s)) => s == "true" || s == "1", _ => d }
    }
}

/// ids of invisible objects plus all their descendants (an invisible parent hides its children; children may precede parents in the file)
fn hidden_ids(objs: &[Value], props: &Props) -> std::collections::HashSet<i64> {
    let mut h: std::collections::HashSet<i64> = objs.iter().filter(|o| !props.boolean(o.get("visible"), true)).filter_map(|o| o.get("id").and_then(|v| v.as_i64())).collect();
    loop {
        let n = h.len();
        for o in objs { if let (Some(id), Some(p)) = (o.get("id").and_then(|v| v.as_i64()), o.get("parent").and_then(|v| v.as_i64())) { if h.contains(&p) { h.insert(id); } } }
        if h.len() == n { break h; }
    }
}

pub fn floats(v: &Value) -> Option<Vec<f32>> {
    match v {
        Value::Number(n) => Some(vec![n.as_f64()? as f32]),
        Value::Bool(b) => Some(vec![*b as i32 as f32]),
        Value::String(s) => { let p: Vec<f32> = s.split_whitespace().filter_map(|x| x.parse().ok()).collect(); if p.is_empty() { None } else { Some(p) } }
        Value::Array(a) => Some(a.iter().filter_map(|x| x.as_f64().map(|f| f as f32)).collect()),
        _ => None,
    }
}
fn v3(v: Option<Vec<f32>>, d: [f32; 3]) -> [f32; 3] {
    match v { Some(a) if a.len() >= 3 => [a[0], a[1], a[2]], Some(a) if a.len() == 2 => [a[0], a[1], d[2]], Some(a) if a.len() == 1 => [a[0]; 3], _ => d }
}

// ---------------- scene ----------------

#[derive(Clone)]
pub struct PassOv { pub constants: Map<String, Value>, pub textures: Vec<Option<String>>, pub combos: Map<String, Value> }
#[derive(Clone)]
pub struct EffectRef { pub file: String, pub name: String, pub passes: Vec<PassOv> }

#[derive(Clone)]
pub enum Kind { Image { model: String }, Particle { file: String, ov: Vec<(String, Vec<f32>)> }, Sound { files: Vec<String> }, Light, Group, Other(String), Text(Box<crate::text::TextDef>) }

/// WE `alignment`: which point of the layer the origin refers to; returns the quad centre offset in layer sizes.
pub fn align_of(a: &str) -> [f32; 2] {
    let a = a.to_ascii_lowercase();
    [if a.contains("left") { 0.5 } else if a.contains("right") { -0.5 } else { 0.0 }, if a.contains("top") { -0.5 } else if a.contains("bottom") { 0.5 } else { 0.0 }]
}

#[derive(Clone)]
pub struct Obj {
    pub id: i64, pub name: String, pub kind: Kind,
    pub origin: [f32; 3], pub scale: [f32; 3], pub angles: [f32; 3],
    pub size: Option<[f32; 2]>, pub alpha: f32, pub color: [f32; 3],
    pub parent: Option<i64>, pub effects: Vec<EffectRef>, pub blend: Option<String>,
    pub align: [f32; 2], // quad centre offset from the origin in units of the layer size (alignment: top / left / topleft ...)
}

pub struct Scene {
    pub width: f32, pub height: f32, pub clear: [f32; 3],
    pub objects: Vec<Obj>,
    /// Layers hidden with `visible: false`. Not composited, but their render targets can be sampled by other
    /// layers (`_rt_imageLayerComposite_<id>_a`), so the renderer still builds them on demand.
    pub hidden: Vec<Obj>,
    pub fatal: Vec<String>,    // reasons the scene cannot be rendered at all (3D)
    pub warnings: Vec<String>, // features ignored in stage 1
}

pub fn parse(j: &Value, props: &Props) -> Result<Scene, String> {
    let g = j.get("general").ok_or("scene.json: no \"general\"")?;
    let mut fatal = vec![]; let mut warnings = vec![];
    let auto_size = g.get("orthogonalprojection").map_or(false, |o| o.get("width").map_or(true, |v| v.is_null()) || o.get("height").map_or(true, |v| v.is_null()));
    let (mut width, mut height) = match g.get("orthogonalprojection") {
        Some(o) => (props.floats(o.get("width")).and_then(|v| v.first().copied()).unwrap_or(1920.0),
                    props.floats(o.get("height")).and_then(|v| v.first().copied()).unwrap_or(1080.0)),
        None => { fatal.push("perspective (3D) camera".into()); (1920.0, 1080.0) }
    };
    let clear = v3(props.floats(g.get("clearcolor")), [0.0, 0.0, 0.0]);
    let mut objects = vec![];
    let mut hidden_objs = vec![];
    let hidden = hidden_ids(j.get("objects").and_then(|v| v.as_array()).map(|a| a.as_slice()).unwrap_or(&[]), &props);
    for o in j.get("objects").and_then(|v| v.as_array()).ok_or("scene.json: no objects")? {
        let name = o.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let id = o.get("id").and_then(|v| v.as_i64()).unwrap_or(-1);
        let is_hidden = hidden.contains(&id);
        let kind = if let Some(i) = o.get("image").and_then(|v| v.as_str()) { Kind::Image { model: i.to_string() }
            } else if let Some(p) = o.get("particle").and_then(|v| v.as_str()) { Kind::Particle { file: p.to_string(), ov: o.get("instanceoverride").and_then(|v| v.as_object()).map(|m| m.iter().filter_map(|(k, v)| Some((k.clone(), props.floats(Some(v))?))).collect()).unwrap_or_default() }
            } else if let Some(s) = o.get("sound").and_then(|v| v.as_array()) { Kind::Sound { files: s.iter().filter_map(|x| x.as_str().map(String::from)).collect() }
            } else if o.get("text").is_some() && o.get("font").is_some() { match crate::text::parse(o, &|v| props.resolve(v)) { Some(t) => Kind::Text(Box::new(t)), None => Kind::Group }
            } else if o.get("light").is_some() { Kind::Light
            } else if let Some(m) = o.get("model").and_then(|v| v.as_str()) {
                if !is_hidden && (m.ends_with(".mdl") || m.ends_with(".json")) { fatal.push(format!("3D model object '{name}'")); }
                Kind::Other(m.to_string())
            } else { Kind::Group };
        let mut effects = vec![];
        for e in o.get("effects").and_then(|v| v.as_array()).into_iter().flatten() {
            if !props.boolean(e.get("visible"), true) { continue; }
            let Some(file) = e.get("file").and_then(|v| v.as_str()) else { continue };
            let mut passes = vec![];
            for p in e.get("passes").and_then(|v| v.as_array()).into_iter().flatten() {
                let mut constants = Map::new();
                if let Some(c) = p.get("constantshadervalues").and_then(|v| v.as_object()) { for (k, v) in c { constants.insert(k.clone(), props.resolve(v)); } }
                let mut combos = Map::new();
                if let Some(c) = p.get("combos").and_then(|v| v.as_object()) { for (k, v) in c { combos.insert(k.clone(), props.resolve(v)); } }
                let textures = p.get("textures").and_then(|v| v.as_array()).map(|a| a.iter().map(|t| t.as_str().map(String::from)).collect()).unwrap_or_default();
                passes.push(PassOv { constants, textures, combos });
            }
            effects.push(EffectRef { file: file.to_string(), name: e.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string(), passes });
        }
        let size = props.floats(o.get("size")).filter(|s| s.len() >= 2).map(|s| [s[0], s[1]]);
        let ang = v3(props.floats(o.get("angles")), [0.0; 3]);
        if !is_hidden && (ang[0] != 0.0 || ang[1] != 0.0) { warnings.push(format!("object '{name}': x/y rotation ignored")); }
        let obj = Obj {
            id, name, kind,
            origin: v3(props.floats(o.get("origin")), [0.0; 3]),
            scale: v3(props.floats(o.get("scale")), [1.0; 3]),
            angles: ang, size,
            alpha: props.floats(o.get("alpha")).and_then(|v| v.first().copied()).unwrap_or(1.0),
            color: v3(props.floats(o.get("color")), [1.0; 3]),
            parent: o.get("parent").and_then(|v| v.as_i64()),
            effects,
            blend: o.get("colorBlendMode").and_then(|v| v.as_i64().map(|n| n as i32).or_else(|| v.as_str().and_then(|s| s.parse().ok()))).and_then(|n| match n { 2 => Some("multiply"), 7 => Some("screen"), 9 | 31 => Some("additive"), _ => None }).map(String::from),
            align: align_of(o.get("alignment").and_then(|v| v.as_str()).unwrap_or("center")),
        };
        if is_hidden { hidden_objs.push(obj); } else { objects.push(obj); }
    }
    // auto projection (GIF-template scenes: width/height null): the scene is as large as its biggest sized layer
    if auto_size { if let Some(sz) = objects.iter().filter(|o| matches!(o.kind, Kind::Image { .. })).filter_map(|o| o.size.map(|s| [s[0] * o.scale[0].abs(), s[1] * o.scale[1].abs()])).max_by(|a, b| (a[0] * a[1]).total_cmp(&(b[0] * b[1]))) { if sz[0] > 0.0 && sz[1] > 0.0 { width = sz[0]; height = sz[1]; } } }
    Ok(Scene { width, height, clear, objects, hidden: hidden_objs, fatal, warnings })
}

// ---------------- model / material ----------------

#[derive(Clone, Debug)]
pub struct Material {
    pub shader: String, pub blending: String, pub textures: Vec<Option<String>>,
    pub combos: Map<String, Value>, pub constants: Map<String, Value>,
}

pub fn load_material(vfs: &Vfs, path: &str) -> Option<Material> {
    let j = vfs.read_json(path)?;
    let p = j.get("passes")?.as_array()?.first()?;
    Some(Material {
        shader: p.get("shader").and_then(|v| v.as_str()).unwrap_or("genericimagev2").to_string(),
        blending: p.get("blending").and_then(|v| v.as_str()).unwrap_or("translucent").to_string(),
        textures: p.get("textures").and_then(|v| v.as_array()).map(|a| a.iter().map(|t| t.as_str().map(String::from)).collect()).unwrap_or_default(),
        combos: p.get("combos").and_then(|v| v.as_object()).cloned().unwrap_or_default(),
        constants: p.get("constantshadervalues").and_then(|v| v.as_object()).cloned().unwrap_or_default(),
    })
}

pub struct ImageDef { pub material: Material, pub tex_path: Option<String>, pub size_hint: Option<[f32; 2]>, pub puppet: bool }

/// models/x.json -> material -> main texture path ("materials/<name>.tex").
pub fn load_image(vfs: &Vfs, model: &str) -> Option<ImageDef> {
    let mj = vfs.read_json(model)?;
    let mpath = mj.get("material")?.as_str()?;
    let material = load_material(vfs, mpath)?;
    let tex_path = material.textures.first().cloned().flatten().map(|t| tex_file(&t));
    let size_hint = match (mj.get("width").and_then(|v| v.as_f64()), mj.get("height").and_then(|v| v.as_f64())) {
        (Some(w), Some(h)) => Some([w as f32, h as f32]), _ => None };
    Some(ImageDef { material, tex_path, size_hint, puppet: mj.get("puppet").is_some() })
}

pub fn tex_file(name: &str) -> String {
    if name.ends_with(".tex") { name.to_string() } else if name.starts_with("materials/") { format!("{name}.tex") } else { format!("materials/{name}.tex") }
}

/// Effect definition: effects/x/effect.json.
pub struct EffectPass { pub material: String, pub bind: Vec<(String, usize)>, pub target: Option<String>, pub textures: Vec<Option<String>> }
pub struct Fbo { pub name: String, pub scale: f32 }
pub struct EffectDef { pub passes: Vec<EffectPass>, pub fbos: Vec<Fbo> }

pub fn load_effect(vfs: &Vfs, file: &str) -> Option<EffectDef> {
    let j = vfs.read_json(file)?;
    let mut passes = vec![];
    for p in j.get("passes")?.as_array()? {
        passes.push(EffectPass {
            material: p.get("material")?.as_str()?.to_string(),
            bind: p.get("bind").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|b| Some((b.get("name")?.as_str()?.to_string(), b.get("index")?.as_u64()? as usize))).collect()).unwrap_or_default(),
            target: p.get("target").and_then(|v| v.as_str()).map(String::from),
            textures: p.get("textures").and_then(|v| v.as_array()).map(|a| a.iter().map(|t| t.as_str().map(String::from)).collect()).unwrap_or_default(),
        });
    }
    let fbos = j.get("fbos").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|f| Some(Fbo { name: f.get("name")?.as_str()?.to_string(), scale: f.get("scale").and_then(|s| s.as_f64()).unwrap_or(1.0) as f32 })).collect()).unwrap_or_default();
    Some(EffectDef { passes, fbos })
}

/// Feature report for `--check`.
pub fn report(vfs: &Vfs, s: &Scene) -> Value {
    let (mut images, mut particles, mut sounds, mut lights) = (0, 0, 0, 0);
    let mut effects = BTreeSet::new();
    let mut missing = BTreeSet::new();
    let mut unsupported: Vec<String> = s.fatal.clone();
    let mut unsupported_layers: Vec<String> = vec![];
    let mut unsupported_effects = BTreeSet::new();
    for o in &s.objects {
        match &o.kind {
            Kind::Image { model } => {
                images += 1;
                match load_image(vfs, model) { Some(d) => if d.puppet { unsupported.push(format!("puppet animation in '{}'", o.name)); }, None => { missing.insert(model.clone()); } }
            }
            Kind::Particle { .. } => particles += 1,
            Kind::Other(k) => unsupported_layers.push(format!("{} ({})", o.name, k)),
            Kind::Sound { .. } => sounds += 1,
            Kind::Light => lights += 1,
            _ => {}
        }
        for e in &o.effects {
            effects.insert(e.file.clone());
            match load_effect(vfs, &e.file) {
                None => { missing.insert(e.file.clone()); unsupported_effects.insert(e.file.clone()); }
                Some(d) => for p in &d.passes {
                    if let Some(m) = load_material(vfs, &p.material) {
                        for ext in ["vert", "frag"] { let f = format!("shaders/{}.{ext}", m.shader); if !vfs.exists(&f) { missing.insert(f); } }
                    } else { missing.insert(p.material.clone()); }
                }
            }
        }
    }
    let ok = unsupported.is_empty();
    json!({"ok": true, "type": "scene", "width": s.width, "height": s.height, "layers": s.objects.len(), "images": images,
        "effects": effects, "particles": particles, "sounds": sounds, "lights": lights,
        "missing_files": missing, "unsupported": unsupported, "unsupported_layers": unsupported_layers, "unsupported_effects": unsupported_effects, "warnings": s.warnings,
        "supported": ok, "fully_supported": ok && particles == 0 && missing.is_empty() && s.warnings.is_empty(),
        "stage2_pending": {"particles": particles, "sounds": sounds}})
}
use serde_json::json;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn props_and_bindings() {
        let project = json!({"general": {"properties": {"show": {"type": "bool", "value": false}, "tint": {"type": "color", "value": "1 0 0"}, "mode": {"value": "2"}}}});
        let p = Props::load(&project, &[("show".into(), "true".into())]);
        assert!(p.boolean(Some(&json!({"user": "show", "value": false})), false));
        assert_eq!(p.floats(Some(&json!({"user": "tint", "value": "0 0 1"}))).unwrap(), vec![1.0, 0.0, 0.0]);
        assert_eq!(p.resolve(&json!({"user": {"name": "mode", "condition": "2"}, "value": false})), json!(true));
        assert_eq!(p.resolve(&json!({"user": {"name": "mode", "condition": "3"}, "value": true})), json!(false));
        assert_eq!(p.resolve(&json!({"user": "nope", "value": 7})), json!(7));
    }
    #[test]
    fn scene_parse() {
        let j = json!({"general": {"clearcolor": "0.1 0.2 0.3", "orthogonalprojection": {"width": 1920, "height": 1080}},
            "objects": [
              {"id": 1, "name": "bg", "image": "models/bg.json", "origin": "960 540 0", "size": "1920 1080", "alpha": 0.5, "color": "1 0.5 1"},
              {"id": 2, "name": "hidden", "image": "models/x.json", "visible": {"user": "h", "value": false}},
              {"id": 3, "name": "fx", "image": "models/a.json", "parent": 1, "angles": "0 0 45",
               "effects": [{"file": "effects/blur/effect.json", "passes": [{"constantshadervalues": {"amount": {"user": "k", "value": 3}}}]}]},
              {"id": 4, "name": "p", "particle": "particles/p.json"}, {"id": 5, "sound": ["s.mp3"]}]});
        let s = parse(&j, &Props::default()).unwrap();
        assert_eq!((s.width, s.height, s.clear), (1920.0, 1080.0, [0.1, 0.2, 0.3]));
        assert_eq!(s.objects.len(), 4);
        assert_eq!(s.objects[0].alpha, 0.5);
        assert_eq!(s.objects[1].parent, Some(1));
        assert_eq!(s.objects[1].effects[0].passes[0].constants["amount"], json!(3));
        assert!(s.fatal.is_empty());
        let s3 = parse(&json!({"general": {}, "objects": [{"name": "m", "model": "models/x.mdl"}]}), &Props::default()).unwrap();
        assert_eq!(s3.fatal.len(), 2);
    }
    #[test]
    fn matrices() {
        let m = trs([10.0, 20.0, 0.0], [2.0, 2.0, 1.0], 90.0);
        // (1,0) -> scale 2 -> rotate 90 -> (0,2) -> translate
        let (x, y) = (m[0] + m[12], m[1] + m[13]);
        assert!((x - 10.0).abs() < 1e-4 && (y - 22.0).abs() < 1e-4);
        let id = mul(&ident(), &m);
        assert_eq!(id, m);
    }
}
