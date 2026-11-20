//! Scene renderer: image layers as textured quads in orthographic scene space, multi-pass effects with ping-pong FBOs.
//! Extension points for stage 2: `Layer::kind` (add particle/sound layers), `Engine::tick` (audio-reactive uniforms).
use crate::pkg::Vfs;
use crate::scene::{self, Kind, Mat4, Scene};
use crate::shader::{self, Uniform};
use crate::tex::{self, GlTex, TexMeta};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::ffi::CString;

pub use crate::gfx::Scaling;

#[derive(Clone, Copy, PartialEq)]
enum Blend { Alpha, Additive }

const VS: &str = "#version 300 es
uniform mat4 mvp; uniform vec2 size; uniform float flip; uniform vec2 uvs; uniform vec2 uvo; out vec2 uv;
void main(){ vec2 p = vec2(float(gl_VertexID & 1), float(gl_VertexID >> 1));
 uv = vec2(p.x, flip > 0.5 ? p.y : 1.0 - p.y) * uvs + uvo; gl_Position = mvp * vec4((p - 0.5) * size, 0.0, 1.0); }";
const FS: &str = "#version 300 es
precision highp float; in vec2 uv; out vec4 o; uniform sampler2D t; uniform vec4 tint;
void main(){ vec4 c = texture(t, uv); o = vec4(c.rgb * tint.rgb, c.a * tint.a); }";

fn compile(kind: u32, src: &str) -> Result<u32, String> {
    unsafe {
        let s = gl::CreateShader(kind);
        let c = CString::new(src).unwrap();
        gl::ShaderSource(s, 1, &c.as_ptr(), std::ptr::null());
        gl::CompileShader(s);
        let mut ok = 0; gl::GetShaderiv(s, gl::COMPILE_STATUS, &mut ok);
        if ok == 0 {
            let mut buf = [0u8; 2048]; let mut n = 0;
            gl::GetShaderInfoLog(s, 2048, &mut n, buf.as_mut_ptr() as *mut _);
            gl::DeleteShader(s);
            return Err(String::from_utf8_lossy(&buf[..n as usize]).trim().to_string());
        }
        Ok(s)
    }
}
pub fn link(vs: &str, fs: &str) -> Result<u32, String> {
    unsafe {
        let v = compile(gl::VERTEX_SHADER, vs).map_err(|e| format!("vertex: {e}"))?;
        let f = compile(gl::FRAGMENT_SHADER, fs).map_err(|e| format!("fragment: {e}"))?;
        let p = gl::CreateProgram();
        gl::AttachShader(p, v); gl::AttachShader(p, f);
        gl::BindAttribLocation(p, 0, c"a_Position".as_ptr());
        gl::BindAttribLocation(p, 1, c"a_TexCoord".as_ptr());
        gl::LinkProgram(p);
        gl::DeleteShader(v); gl::DeleteShader(f);
        let mut ok = 0; gl::GetProgramiv(p, gl::LINK_STATUS, &mut ok);
        if ok == 0 {
            let mut buf = [0u8; 2048]; let mut n = 0;
            gl::GetProgramInfoLog(p, 2048, &mut n, buf.as_mut_ptr() as *mut _);
            return Err(format!("link: {}", String::from_utf8_lossy(&buf[..n as usize]).trim()));
        }
        Ok(p)
    }
}
fn loc(p: u32, n: &str) -> i32 { let c = CString::new(n).unwrap(); unsafe { gl::GetUniformLocation(p, c.as_ptr()) } }

#[derive(Clone, Copy)]
struct Target { tex: u32, fbo: u32, w: i32, h: i32 }
impl Target {
    fn new(w: i32, h: i32) -> Target {
        unsafe {
            let (mut t, mut f) = (0, 0);
            gl::GenTextures(1, &mut t); gl::BindTexture(gl::TEXTURE_2D, t);
            gl::TexStorage2D(gl::TEXTURE_2D, 1, gl::RGBA8, w, h);
            for (k, v) in [(gl::TEXTURE_MIN_FILTER, gl::LINEAR), (gl::TEXTURE_MAG_FILTER, gl::LINEAR), (gl::TEXTURE_WRAP_S, gl::CLAMP_TO_EDGE), (gl::TEXTURE_WRAP_T, gl::CLAMP_TO_EDGE)] {
                gl::TexParameteri(gl::TEXTURE_2D, k, v as i32);
            }
            gl::GenFramebuffers(1, &mut f); gl::BindFramebuffer(gl::FRAMEBUFFER, f);
            gl::FramebufferTexture2D(gl::FRAMEBUFFER, gl::COLOR_ATTACHMENT0, gl::TEXTURE_2D, t, 0);
            Target { tex: t, fbo: f, w, h }
        }
    }
    fn bytes(&self) -> usize { self.w as usize * self.h as usize * 4 }
}

impl Layer {
    /// UV scale and offset of the sprite-sheet cell shown at time `t` (whole texture when not a sheet).
    fn cell(&self, t: f32) -> ([f32; 2], [f32; 2]) {
        if self.frames.len() < 2 { return (self.uvs, [0.0, 0.0]); }
        let f = &self.frames[tex::frame_at(&self.frames, t)];
        ([f.w / self.tex_wh.0, f.h / self.tex_wh.1], [f.x / self.tex_wh.0, f.y / self.tex_wh.1])
    }
}

struct Pass {
    prog: u32,
    uniforms: Vec<(Uniform, i32)>,
    values: HashMap<String, Vec<f32>>, // material key -> value
    uvr: Vec<[f32; 2]>, // unit -> image fraction of the (padded) texture, for g_TextureNResolution.zw
    textures: Vec<Option<(u32, (u32, u32))>>, // unit -> static texture (None: previous / bound by name)
    binds: Vec<(usize, String)>,               // unit -> named source ("previous" or fbo name)
    target: Option<String>,
    time: bool,
}

struct Layer {
    name: String, tex: u32, size: [f32; 2], world: Mat4, color: [f32; 3], alpha: f32, blend: Blend,
    uvs: [f32; 2], frames: Vec<tex::SpriteFrame>, tex_wh: (f32, f32), full: bool, passes: Vec<Pass>, ping: Option<(Target, Target)>, named: HashMap<String, Target>, px: (i32, i32),
}

pub struct Engine {
    layers: Vec<Layer>, prog: u32, u_mvp: i32, u_size: i32, u_tint: i32, u_flip: i32, u_uvs: i32, u_uvo: i32,
    vao: u32, view: Mat4, pub clear: [f32; 3], scene_wh: (f32, f32),
    pub animated: bool, pub vram: usize, pub notes: Vec<String>, scene_rt: Option<Target>, white: u32,
    out: (i32, i32), scaling: Scaling, vfs: Vfs, scene: Scene, shader_cache: HashMap<String, Option<shader::Translated>>,
    t0: std::time::Instant, time_override: Option<f32>,
}

pub fn view_matrix(sw: f32, sh: f32, ow: i32, oh: i32, scaling: Scaling) -> (Mat4, f32, f32) {
    let (ow, oh) = (ow as f32, oh as f32);
    let (fx, fy) = (ow / sw, oh / sh);
    let (sx, sy) = match scaling { Scaling::Stretch => (fx, fy), Scaling::Fill => { let f = fx.max(fy); (f, f) } Scaling::Fit => { let f = fx.min(fy); (f, f) } };
    let mut m = scene::ident();
    m[0] = 2.0 * sx / ow; m[5] = 2.0 * sy / oh;
    m[12] = -sw / 2.0 * m[0]; m[13] = -sh / 2.0 * m[5];
    (m, sx, sy)
}

fn world_of(objs: &[scene::Obj], i: usize, depth: u32) -> Mat4 {
    let o = &objs[i];
    let local = scene::trs(o.origin, o.scale, o.angles[2]);
    if depth > 16 { return local; }
    match o.parent.and_then(|p| objs.iter().position(|x| x.id == p)) {
        Some(pi) if pi != i => scene::mul(&world_of(objs, pi, depth + 1), &local),
        _ => local,
    }
}

fn blend_of(s: &str) -> Blend { match s { "additive" => Blend::Additive, _ => Blend::Alpha } }

impl Engine {
    pub fn new(vfs: Vfs, scene: Scene, out: (i32, i32), scaling: Scaling) -> Result<Engine, String> {
        let prog = link(VS, FS)?;
        let mut vao = 0; let mut vbo = 0;
        unsafe {
            gl::GenVertexArrays(1, &mut vao); gl::BindVertexArray(vao);
            let v: [f32; 20] = [-1.0, -1.0, 0.0, 0.0, 0.0, 1.0, -1.0, 0.0, 1.0, 0.0, -1.0, 1.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 1.0, 1.0];
            gl::GenBuffers(1, &mut vbo); gl::BindBuffer(gl::ARRAY_BUFFER, vbo);
            gl::BufferData(gl::ARRAY_BUFFER, 80, v.as_ptr() as *const _, gl::STATIC_DRAW);
            gl::EnableVertexAttribArray(0); gl::VertexAttribPointer(0, 3, gl::FLOAT, gl::FALSE, 20, std::ptr::null());
            gl::EnableVertexAttribArray(1); gl::VertexAttribPointer(1, 2, gl::FLOAT, gl::FALSE, 20, 12 as *const _);
            gl::Disable(gl::DEPTH_TEST); gl::Disable(gl::DITHER); gl::PixelStorei(gl::UNPACK_ALIGNMENT, 1);
        }
        let mut e = Engine { layers: vec![], prog, u_mvp: loc(prog, "mvp"), u_size: loc(prog, "size"), u_tint: loc(prog, "tint"), u_flip: loc(prog, "flip"), u_uvs: loc(prog, "uvs"), u_uvo: loc(prog, "uvo"),
            vao, view: scene::ident(), clear: scene.clear, scene_wh: (scene.width, scene.height), animated: false, vram: 0, notes: vec![], scene_rt: None, white: 0,
            out, scaling, vfs, scene, shader_cache: HashMap::new(), t0: std::time::Instant::now(), time_override: None };
        e.rebuild();
        Ok(e)
    }

    pub fn set_time(&mut self, t: Option<f32>) { self.time_override = t; }

    pub fn resize(&mut self, out: (i32, i32)) { if out != self.out { self.out = out; self.rebuild(); } }

    fn white_tex(&mut self) -> u32 {
        if self.white == 0 { unsafe {
            gl::GenTextures(1, &mut self.white); gl::BindTexture(gl::TEXTURE_2D, self.white);
            let px = [255u8; 4];
            gl::TexImage2D(gl::TEXTURE_2D, 0, gl::RGBA8 as i32, 1, 1, 0, gl::RGBA, gl::UNSIGNED_BYTE, px.as_ptr() as *const _);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::NEAREST as i32); gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::NEAREST as i32);
        } }
        self.white
    }

    fn note(&mut self, s: String) { eprintln!("wallforge-render: {s}"); self.notes.push(s); }

    /// (Re)build all GPU resources for the current output size (texture mips / FBO sizes depend on it).
    fn rebuild(&mut self) {
        self.layers.clear(); self.vram = 0; self.animated = false; self.scene_rt = None;
        let (v, sx, sy) = view_matrix(self.scene_wh.0, self.scene_wh.1, self.out.0, self.out.1, self.scaling);
        self.view = v;
        let s3tc = tex::has_s3tc();
        let mut cache: HashMap<String, Option<GlTex>> = HashMap::new();
        // Effect buffers are only live while one layer renders, so layers share them by (size, slot).
        let mut pool: HashMap<(i32, i32, usize), Target> = HashMap::new();
        let n = self.scene.objects.len();
        for i in 0..n {
            let (name, model, size_o, scale, color, alpha, blend_o) = {
                let o = &self.scene.objects[i];
                let Kind::Image { model } = &o.kind else { continue };
                (o.name.clone(), model.clone(), o.size, o.scale, o.color, o.alpha, o.blend.clone())
            };
            let Some(def) = scene::load_image(&self.vfs, &model) else { self.note(format!("layer '{name}': model/material '{model}' not found, skipped")); continue };
            let solid = def.tex_path.is_none() && (def.material.shader.contains("solid") || def.material.shader.contains("flat"));
            let tp = match def.tex_path.clone() { Some(t) => t, None if solid => String::new(), None => { self.note(format!("layer '{name}': no texture, skipped")); continue } };
            let full = tp.ends_with("_rt_FullFrameBuffer.tex");
            let mut frames: Vec<tex::SpriteFrame> = vec![]; let mut tex_wh = (1.0f32, 1.0f32);
            let (size, px, gt) = if solid {
                let sz = size_o.or(def.size_hint).unwrap_or([self.scene_wh.0, self.scene_wh.1]);
                let px = ((sz[0] * scale[0].abs() * sx).ceil().max(1.0) as i32, (sz[1] * scale[1].abs() * sy).ceil().max(1.0) as i32);
                (sz, px, GlTex { id: self.white_tex(), w: 1, h: 1, bytes: 4, compressed: false, flags: 0, src: (1, 1), uv: [1.0, 1.0] })
            } else if full {
                ([self.scene_wh.0, self.scene_wh.1], self.out, GlTex { id: 0, w: 0, h: 0, bytes: 0, compressed: false, flags: 0, src: (0, 0), uv: [1.0, 1.0] })
            } else {
            let Some(bytes) = self.vfs.read(&tp) else { self.note(format!("layer '{name}': texture {tp} not found, skipped")); continue };
            let t = match tex::parse(&bytes, false) { Ok(t) => t, Err(e) => { self.note(format!("layer '{name}': {e}, skipped")); continue } };
            let meta = self.vfs.read_json(&tp.replace(".tex", ".tex-json")).map(|j| tex::parse_meta(&j)).unwrap_or_default();
            let size = size_o.or(def.size_hint).unwrap_or([t.img_w as f32, t.img_h as f32]);
            let px = ((size[0] * scale[0].abs() * sx).ceil().max(1.0) as i32, (size[1] * scale[1].abs() * sy).ceil().max(1.0) as i32);
            let gt = match tex::upload(&t, px.0 as u32, px.1 as u32, &meta, s3tc) { Ok(g) => g, Err(e) => { self.note(format!("layer '{name}': {e}, skipped")); continue } };
            frames = t.frames.clone(); tex_wh = (t.tex_w as f32, t.tex_h as f32);
            (size, px, gt)
            };
            if frames.len() > 1 { self.animated = true; }
            self.vram += gt.bytes;
            if std::env::var("WF_DBG").is_ok() { eprintln!("vram: layer '{name}' tex {:.1} MiB ({}x{}) px {:?}", gt.bytes as f64 / 1048576.0, gt.w, gt.h, px); }
            let world = world_of(&self.scene.objects, i, 0);
            let mut layer = Layer { name: name.clone(), tex: gt.id, size, world, color, alpha, uvs: gt.uv, frames, tex_wh, full, blend: blend_of(blend_o.as_deref().unwrap_or(&def.material.blending)),
                passes: vec![], ping: None, named: HashMap::new(), px };
            if def.material.blending == "additive" { layer.blend = Blend::Additive; }
            // effects
            let effects: Vec<_> = self.scene.objects[i].effects.iter().map(|e| (e.file.clone(), e.name.clone(), e.passes.iter().map(|p| (p.constants.clone(), p.textures.clone(), p.combos.clone())).collect::<Vec<_>>())).collect();
            let mut fbo_div: HashMap<String, f32> = HashMap::new();
            let effects: Vec<_> = effects.into_iter().take(std::env::var("WF_MAXFX").ok().and_then(|v| v.parse().ok()).unwrap_or(usize::MAX)).collect();
            let only: Option<usize> = std::env::var("WF_ONLYFX").ok().and_then(|v| v.parse().ok()); let effects: Vec<_> = effects.into_iter().enumerate().filter(|(i, _)| only.map_or(true, |o| o == *i)).map(|x| x.1).collect();
            for (file, ename, ovs) in effects {
                let Some(ed) = scene::load_effect(&self.vfs, &file) else { self.note(format!("layer '{name}': effect {file} not found, skipped")); continue };
                for f in &ed.fbos { fbo_div.insert(f.name.clone(), f.scale.max(1.0)); }
                let mut built = vec![];
                let mut ok = true;
                for (pi, ep) in ed.passes.iter().enumerate() {
                    match self.build_pass(&ename, ep, ovs.get(pi), &mut cache, s3tc, (px.0 as u32, px.1 as u32)) { Ok(p) => built.push(p), Err(e) => { self.note(format!("layer '{name}': effect {file} pass {pi}: {e}; effect skipped")); ok = false; break; } }
                }
                if ok { layer.passes.extend(built); }
            }
            if !layer.passes.is_empty() {
                let (w, h) = (px.0.clamp(1, 4096), px.1.clamp(1, 4096));
                let (w, h) = (w, h);
                let mut grab = |slot: usize, tw: i32, th: i32, vram: &mut usize| -> Target {
                    *pool.entry((tw, th, slot)).or_insert_with(|| { let t = Target::new(tw, th); *vram += t.bytes(); t })
                };
                let (a, b) = (grab(0, w as i32, h as i32, &mut self.vram), grab(1, w as i32, h as i32, &mut self.vram));
                layer.ping = Some((a, b));
                for p in &layer.passes {
                    if let Some(t) = &p.target {
                        if !layer.named.contains_key(t) {
                            let d = fbo_div.get(t).copied().unwrap_or(1.0);
                            let slot = 2 + layer.named.len();
                            let tg = grab(slot, ((w as f32 / d) as i32).max(1), ((h as f32 / d) as i32).max(1), &mut self.vram);
                            if std::env::var("WF_DBG").is_ok() { eprintln!("vram: fbo {t} {}x{}", tg.w, tg.h); }
                            layer.named.insert(t.clone(), tg);
                        }
                    }
                    if p.time { self.animated = true; }
                }
            }
            self.layers.push(layer);
        }
        if self.layers.iter().any(|l| l.full) {
            let rt = Target::new(self.out.0.max(1), self.out.1.max(1));
            self.vram += rt.bytes(); self.scene_rt = Some(rt);
        }
        for (k, t) in cache { if let Some(t) = t { if std::env::var("WF_DBG").is_ok() { eprintln!("vram: cached {k} {:.1} MiB {}x{}", t.bytes as f64 / 1048576.0, t.w, t.h); } self.vram += t.bytes; } }
        eprintln!("wallforge-render: scene {}x{} -> {}x{}: {} layers, {} with effects, texture+fbo memory {:.1} MiB, animated={}",
            self.scene_wh.0, self.scene_wh.1, self.out.0, self.out.1, self.layers.len(), self.layers.iter().filter(|l| !l.passes.is_empty()).count(), self.vram as f64 / 1048576.0, self.animated);
    }

    fn build_pass(&mut self, ename: &str, ep: &scene::EffectPass, ov: Option<&(Map<String, Value>, Vec<Option<String>>, Map<String, Value>)>,
                  cache: &mut HashMap<String, Option<GlTex>>, s3tc: bool, want: (u32, u32)) -> Result<Pass, String> {
        let mat = scene::load_material(&self.vfs, &ep.material).ok_or_else(|| format!("material {} not found", ep.material))?;
        let mut combos = mat.combos.clone();
        if let Some((_, _, c)) = ov { for (k, v) in c { combos.insert(k.clone(), v.clone()); } }
        let mut present: Vec<usize> = vec![];
        {
            let mut names: std::collections::BTreeMap<usize, bool> = std::collections::BTreeMap::new();
            for (i, t) in mat.textures.iter().enumerate() { names.insert(i, t.is_some()); }
            for (i, t) in ep.textures.iter().enumerate() { if t.is_some() { names.insert(i, true); } }
            if let Some((_, ts, _)) = ov { for (i, x) in ts.iter().enumerate() { if x.is_some() { names.insert(i, true); } } }
            for (i, b) in names { if b { present.push(i); } }
        }
        let key = format!("{}|{}|{:?}", mat.shader, Value::Object(combos.clone()), present);
        if !self.shader_cache.contains_key(&key) {
            let vfs = &self.vfs;
            let r = shader::translate(&|n| vfs.read_str(n), &mat.shader, &combos, &present, false);
            if let Err(e) = &r { eprintln!("wallforge-render: shader {}: {e}", mat.shader); }
            self.shader_cache.insert(key.clone(), r.ok());
        }
        let tr = self.shader_cache.get(&key).and_then(|t| t.as_ref()).ok_or_else(|| format!("shader '{}' unavailable", mat.shader))?;
        if let Ok(d) = std::env::var("WF_DUMP_ALL") { let n = mat.shader.replace('/', "_"); let _ = std::fs::create_dir_all(&d); let _ = std::fs::write(format!("{d}/{n}.frag"), &tr.frag); let _ = std::fs::write(format!("{d}/{n}.vert"), &tr.vert); }
        let retry = match link(&tr.vert, &tr.frag) {
            Ok(p) => Ok((p, None)),
            Err(e1) => {
                let vfs = &self.vfs;
                match shader::translate(&|n| vfs.read_str(n), &mat.shader, &combos, &present, true) {
                    Ok(t2) => { if std::env::var("WF_DBG").is_ok() { eprintln!("retry frag has float(: {}", t2.frag.contains("float(sampleCount)")); } match link(&t2.vert, &t2.frag) { Ok(p) => Ok((p, Some(t2))), Err(e2) => Err((e2, Some(t2))) } },
                    Err(e) => { eprintln!("wallforge-render: floatify retry translate failed: {e}"); Err((e1, None)) }
                }
            }
        };
        let (prog, tr_fix) = match retry { Ok(x) => x, Err((e, t2)) => {
            let tr = t2.as_ref().unwrap_or(tr);
            if let Ok(d) = std::env::var("WF_DUMP_SHADERS") {
                let n = mat.shader.replace('/', "_");
                let _ = std::fs::create_dir_all(&d);
                let _ = std::fs::write(format!("{d}/{n}.vert"), &tr.vert);
                let _ = std::fs::write(format!("{d}/{n}.frag"), &tr.frag);
                let _ = std::fs::write(format!("{d}/{n}.err"), &e);
            }
            return Err(format!("shader '{}' {}", mat.shader, e.lines().next().unwrap_or("")));
        } };
        let tr = tr_fix.as_ref().unwrap_or(tr);
        let uniforms: Vec<(Uniform, i32)> = tr.uniforms.iter().map(|u| (u.clone(), loc(prog, &u.name))).filter(|(_, l)| *l >= 0).collect();
        let mut values: HashMap<String, Vec<f32>> = HashMap::new();
        for (k, v) in mat.constants.iter().chain(ov.map(|o| &o.0).into_iter().flatten()) {
            if let Some(f) = scene::floats(v) { values.insert(k.clone(), f); }
        }
        // textures: material list < effect.json list < scene override
        let mut names: Vec<Option<String>> = mat.textures.clone();
        for (i, t) in ep.textures.iter().enumerate() { if names.len() <= i { names.resize(i + 1, None); } if t.is_some() { names[i] = t.clone(); } }
        if let Some((_, ts, _)) = ov { for (i, t) in ts.iter().enumerate() { if names.len() <= i { names.resize(i + 1, None); } if t.is_some() { names[i] = t.clone(); } } }
        let mut textures = vec![]; let mut uvr: Vec<[f32; 2]> = vec![]; let mut binds = vec![(0usize, "previous".to_string())];
        if let Some(mx) = ep.bind.iter().map(|b| b.1 + 1).max() { if names.len() < mx { names.resize(mx, None); } }
        for (i, n) in names.iter().enumerate() {
            let bound = ep.bind.iter().find(|b| b.1 == i).map(|b| b.0.clone());
            if let Some(b) = bound { binds.retain(|x| x.0 != i); binds.push((i, b)); textures.push(None); uvr.push([1.0, 1.0]); continue; }
            match n {
                Some(n) if !n.starts_with("_rt_") => {
                    let path = scene::tex_file(n);
                    if !cache.contains_key(&path) {
                        let r = self.vfs.read(&path).and_then(|b| tex::parse(&b, false).ok()).and_then(|t| tex::upload(&t, want.0, want.1, &TexMeta::default(), s3tc).ok());
                        cache.insert(path.clone(), r);
                    }
                    match cache.get(&path).and_then(|x| x.as_ref()) { Some(g) => { textures.push(Some((g.id, (g.w, g.h)))); uvr.push(g.uv) }, None => return Err(format!("texture {path} not found")) }
                }
                _ => { textures.push(None); uvr.push([1.0, 1.0]) },
            }
        }
        Ok(Pass { prog, uniforms, values, uvr, textures, binds, target: ep.target.clone(), time: tr.uses_time })
    }

    /// Stage-2 hook: audio spectrum / particle simulation step goes here.
    pub fn tick(&mut self) {}

    fn set_uniform(&self, u: &Uniform, l: i32, p: &Pass, mvp: &Mat4, t: f32, tex_res: &[(f32, f32)]) {
        let name = u.name.as_str();
        unsafe {
            if let Some(n) = name.strip_prefix("g_Texture") {
                if let Some(n) = n.strip_suffix("Resolution") {
                    if let Ok(i) = n.parse::<usize>() { let (w, h) = tex_res.get(i).copied().unwrap_or((1.0, 1.0)); let f = p.uvr.get(i).copied().unwrap_or([1.0, 1.0]); gl::Uniform4f(l, w, h, w * f[0], h * f[1]); }
                    return;
                }
                if let Ok(i) = n.parse::<i32>() { gl::Uniform1i(l, i); return; }
            }
            match name {
                "g_ModelViewProjectionMatrix" | "g_EffectTextureProjectionMatrix" | "g_ViewProjectionMatrix" | "g_ModelMatrix" => {
                    let m = if name == "g_ModelViewProjectionMatrix" { *mvp } else { scene::ident() };
                    gl::UniformMatrix4fv(l, 1, gl::FALSE, m.as_ptr()); return;
                }
                "g_EffectTextureProjectionMatrixInverse" => { gl::UniformMatrix4fv(l, 1, gl::FALSE, scene::ident().as_ptr()); return; }
                "g_Time" => { gl::Uniform1f(l, t); return; }
                "g_ScreenResolution" | "g_Screen" => { gl::Uniform3f(l, self.out.0 as f32, self.out.1 as f32, 1.0); return; }
                "g_Alpha" => { gl::Uniform1f(l, 1.0); return; }
                "g_Color" => { gl::Uniform3f(l, 1.0, 1.0, 1.0); return; }
                _ => {}
            }
            let val: Option<Vec<f32>> = u.material.as_ref().and_then(|k| p.values.get(k).cloned()).or_else(|| u.default.as_ref().and_then(scene::floats));
            let Some(v) = val else { return };
            let g = |i: usize| v.get(i).copied().unwrap_or(*v.last().unwrap_or(&0.0));
            match u.ty.as_str() {
                "float" => gl::Uniform1f(l, g(0)), "int" | "bool" => gl::Uniform1i(l, g(0) as i32),
                "vec2" => gl::Uniform2f(l, g(0), g(1)), "vec3" => gl::Uniform3f(l, g(0), g(1), g(2)), "vec4" => gl::Uniform4f(l, g(0), g(1), g(2), g(3)),
                _ => {}
            }
        }
    }

    pub fn draw(&mut self) {
        let t = self.time_override.unwrap_or_else(|| self.t0.elapsed().as_secs_f32());
        let rt_fbo = self.scene_rt.as_ref().map(|r| r.fbo).unwrap_or(0);
        unsafe {
            gl::BindFramebuffer(gl::FRAMEBUFFER, rt_fbo);
            gl::Viewport(0, 0, self.out.0, self.out.1);
            gl::ClearColor(self.clear[0], self.clear[1], self.clear[2], 1.0);
            gl::Clear(gl::COLOR_BUFFER_BIT);
            gl::BindVertexArray(self.vao);
        }
        for li in 0..self.layers.len() {
            let l = &self.layers[li];
            if l.full && l.ping.is_none() { continue; }
            let src_tex = if l.full { self.render_effects(li, t, self.scene_rt.as_ref().map(|r| r.tex)) } else { self.render_effects(li, t, None) };
            let l = &self.layers[li];
            let mvp = scene::mul(&self.view, &l.world);
            let mvp = if l.full { scene::ident() } else { mvp };
            let lsize = if l.full { [2.0, 2.0] } else { l.size };
            unsafe {
                gl::BindFramebuffer(gl::FRAMEBUFFER, rt_fbo);
                gl::Viewport(0, 0, self.out.0, self.out.1);
                if l.full { gl::Disable(gl::BLEND); } else { gl::Enable(gl::BLEND); }
                match l.blend { Blend::Alpha => gl::BlendFuncSeparate(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA, gl::ONE, gl::ONE_MINUS_SRC_ALPHA), Blend::Additive => gl::BlendFunc(gl::SRC_ALPHA, gl::ONE) }
                gl::UseProgram(self.prog);
                gl::UniformMatrix4fv(self.u_mvp, 1, gl::FALSE, mvp.as_ptr());
                gl::Uniform2f(self.u_size, lsize[0], lsize[1]);
                gl::Uniform4f(self.u_tint, l.color[0], l.color[1], l.color[2], l.alpha);
                gl::Uniform1f(self.u_flip, 0.0);
                if l.ping.is_some() { gl::Uniform2f(self.u_uvs, 1.0, 1.0); gl::Uniform2f(self.u_uvo, 0.0, 0.0); } else { let (uv, uo) = l.cell(t); gl::Uniform2f(self.u_uvs, uv[0], uv[1]); gl::Uniform2f(self.u_uvo, uo[0], uo[1]); }
                gl::ActiveTexture(gl::TEXTURE0); gl::BindTexture(gl::TEXTURE_2D, src_tex);
                gl::DrawArrays(gl::TRIANGLE_STRIP, 0, 4);
                gl::Disable(gl::BLEND);
            }
        }
        if let Some(rt) = &self.scene_rt {
            unsafe {
                gl::BindFramebuffer(gl::FRAMEBUFFER, 0);
                gl::Viewport(0, 0, self.out.0, self.out.1);
                gl::Disable(gl::BLEND);
                gl::UseProgram(self.prog);
                gl::UniformMatrix4fv(self.u_mvp, 1, gl::FALSE, scene::ident().as_ptr());
                gl::Uniform2f(self.u_size, 2.0, 2.0);
                gl::Uniform4f(self.u_tint, 1.0, 1.0, 1.0, 1.0);
                gl::Uniform1f(self.u_flip, 1.0); gl::Uniform2f(self.u_uvs, 1.0, 1.0); gl::Uniform2f(self.u_uvo, 0.0, 0.0);
                gl::ActiveTexture(gl::TEXTURE0); gl::BindTexture(gl::TEXTURE_2D, rt.tex);
                gl::DrawArrays(gl::TRIANGLE_STRIP, 0, 4);
            }
        }
    }

    /// Runs the layer's effect chain; returns the texture holding the final image.
    fn render_effects(&self, li: usize, t: f32, src: Option<u32>) -> u32 {
        let l = &self.layers[li];
        let Some((a, b)) = &l.ping else { return l.tex };
        unsafe {
            gl::Disable(gl::BLEND);
            gl::BindFramebuffer(gl::FRAMEBUFFER, a.fbo);
            gl::Viewport(0, 0, a.w, a.h);
            gl::ClearColor(0.0, 0.0, 0.0, 0.0); gl::Clear(gl::COLOR_BUFFER_BIT);
            gl::UseProgram(self.prog);
            gl::UniformMatrix4fv(self.u_mvp, 1, gl::FALSE, scene::ident().as_ptr());
            gl::Uniform2f(self.u_size, 2.0, 2.0);
            gl::Uniform4f(self.u_tint, 1.0, 1.0, 1.0, 1.0);
            gl::Uniform1f(self.u_flip, if src.is_some() { 0.0 } else { 1.0 }); { let (uv, uo) = l.cell(t); gl::Uniform2f(self.u_uvs, uv[0], uv[1]); gl::Uniform2f(self.u_uvo, uo[0], uo[1]); }
            gl::ActiveTexture(gl::TEXTURE0); gl::BindTexture(gl::TEXTURE_2D, src.unwrap_or(l.tex));
            gl::DrawArrays(gl::TRIANGLE_STRIP, 0, 4);
        }
        // the fullscreen effect quad: vertex positions are [-1,1] already, so the MVP is the identity
        let mut cur = (a, b); // cur.0 holds "previous", cur.1 is the destination
        let ident = scene::ident();
        let dbg_n: usize = std::env::var("WF_PASSN").ok().and_then(|v| v.parse().ok()).unwrap_or(usize::MAX); let mut dbg_last: Option<u32> = None;
        for p in l.passes.iter().take(dbg_n) {
            let dst = match &p.target { Some(n) => l.named.get(n).unwrap_or(cur.1), None => cur.1 };
            unsafe {
                gl::BindFramebuffer(gl::FRAMEBUFFER, dst.fbo);
                gl::Viewport(0, 0, dst.w, dst.h);
                gl::Clear(gl::COLOR_BUFFER_BIT);
                gl::UseProgram(p.prog);
            }
            let mut res: Vec<(f32, f32)> = vec![];
            let n_units = p.textures.len().max(p.binds.iter().map(|b| b.0 + 1).max().unwrap_or(1));
            for unit in 0..n_units {
                let (id, r) = if let Some(b) = p.binds.iter().find(|b| b.0 == unit) {
                    if b.1 == "previous" { (cur.0.tex, (cur.0.w as f32, cur.0.h as f32)) }
                    else if let Some(n) = l.named.get(&b.1) { (n.tex, (n.w as f32, n.h as f32)) } else { (cur.0.tex, (cur.0.w as f32, cur.0.h as f32)) }
                } else if let Some(Some((id, (w, h)))) = p.textures.get(unit) { (*id, (*w as f32, *h as f32)) } else { (cur.0.tex, (cur.0.w as f32, cur.0.h as f32)) };
                res.push(r);
                unsafe { gl::ActiveTexture(gl::TEXTURE0 + unit as u32); gl::BindTexture(gl::TEXTURE_2D, id); }
            }
            for (u, lc) in &p.uniforms { self.set_uniform(u, *lc, p, &ident, t, &res); }
            unsafe { gl::DrawArrays(gl::TRIANGLE_STRIP, 0, 4); }
            if p.target.is_none() { cur = (cur.1, cur.0); dbg_last = None; } else { dbg_last = Some(dst.tex); }
        }
        if let Some(t) = dbg_last { return t; }
        unsafe { gl::ActiveTexture(gl::TEXTURE0); }
        cur.0.tex
    }
}

#[cfg(test)]
mod layout_tests {
    use super::*;
    /// Regression: a 3840x2160 image stored in a 4096x4096 power-of-two texture must sample only the image area,
    /// and an object the size of the scene must cover the whole output in clip space.
    #[test]
    fn padded_texture_and_fullscreen_layout() {
        let uv = tex::uv_scale((3840, 2160), (4096, 4096));
        assert!((uv[0] - 0.9375).abs() < 1e-6 && (uv[1] - 0.52734375).abs() < 1e-6);
        assert_eq!(tex::uv_scale((4096, 4096), (4096, 4096)), [1.0, 1.0]);
        for (ow, oh) in [(2560, 1440), (1280, 720), (1920, 1080)] {
            let (m, _, _) = view_matrix(3840.0, 2160.0, ow, oh, Scaling::Fill);
            // scene corners (0,0) and (w,h) map to clip -1 and +1 on both axes (column-major 4x4)
            let (x0, y0) = (m[12], m[13]);
            let (x1, y1) = (m[0] * 3840.0 + m[12], m[5] * 2160.0 + m[13]);
            assert!((x0 + 1.0).abs() < 1e-4 && (y0 + 1.0).abs() < 1e-4, "{ow}x{oh}: {x0} {y0}");
            assert!((x1 - 1.0).abs() < 1e-4 && (y1 - 1.0).abs() < 1e-4, "{ow}x{oh}: {x1} {y1}");
        }
    }
}
