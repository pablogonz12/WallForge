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
enum Blend { Alpha, Additive, Multiply, Screen }

const VS: &str = "#version 300 es
uniform mat4 mvp; uniform vec2 size; uniform float flip; uniform vec2 uvs; uniform vec2 uvo; out vec2 uv;
void main(){ vec2 p = vec2(float(gl_VertexID & 1), float(gl_VertexID >> 1));
 uv = vec2(p.x, flip > 0.5 ? p.y : 1.0 - p.y) * uvs + uvo; gl_Position = mvp * vec4((p - 0.5) * size, 0.0, 1.0); }";
const FS: &str = "#version 300 es
precision highp float; in vec2 uv; out vec4 o; uniform sampler2D t; uniform vec4 tint; uniform float pm;
void main(){ vec4 c = texture(t, uv); o = vec4(c.rgb * tint.rgb, c.a * tint.a); if (pm > 0.5) o.rgb *= o.a; }";

fn compile(kind: u32, src: &str) -> Result<u32, String> {
    unsafe {
        let s = gl::CreateShader(kind);
        let c = CString::new(src).unwrap();
        gl::ShaderSource(s, 1, &c.as_ptr(), std::ptr::null());
        gl::CompileShader(s);
        let mut ok = 0; gl::GetShaderiv(s, gl::COMPILE_STATUS, &mut ok);
        if ok == 0 {
            let mut buf = [0u8; 8192]; let mut n = 0;
            gl::GetShaderInfoLog(s, 8192, &mut n, buf.as_mut_ptr() as *mut _);
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
    fn free(&self) { unsafe { gl::DeleteFramebuffers(1, &self.fbo); gl::DeleteTextures(1, &self.tex); } }
}

impl Layer {
    /// UV scale and offset of the sprite-sheet cell shown at time `t` (whole texture when not a sheet).
    fn cell(&self, t: f32) -> ([f32; 2], [f32; 2]) {
        if self.frames.len() < 2 { return (self.uvs, [0.0, 0.0]); }
        let f = &self.frames[tex::frame_at(&self.frames, t)];
        ([f.w / self.tex_wh.0, f.h / self.tex_wh.1], [f.x / self.tex_wh.0, f.y / self.tex_wh.1])
    }
}

/// Placeholder texture name for a declared mask that the package does not supply: bound as 1x1 black.
const BLACK_MASK: &str = "\u{1}black-mask";

struct Pass {
    copy: bool, // material-less pass: blit "previous" into the target, as Wallpaper Engine does
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
    id: i64,      // scene object id (used to resolve `_rt_imageLayerComposite_<id>_a`)
    rt: Option<Target>, // sampled layers: their image in scene space, at the frame size
    composite: bool, // fullscreen layer whose chain samples other layers: starts transparent, drawn blended
    hidden: bool, // built only as a render-target producer: never composited into the frame
    sampled: bool, // another layer samples this one: its buffers must not come from the shared pool
}

pub struct Engine {
    layers: Vec<Layer>, prog: u32, u_mvp: i32, u_size: i32, u_tint: i32, u_pm: i32, u_flip: i32, u_uvs: i32, u_uvo: i32,
    vao: u32, view: Mat4, pub clear: [f32; 3], scene_wh: (f32, f32),
    pub animated: bool, pub audio_dirty: bool, pub vram: usize, pub notes: Vec<String>, scene_rt: Option<Target>, white: u32, black: u32,
    out: (i32, i32), screen: (i32, i32), main_rt: Option<Target>, scaling: Scaling, vfs: Vfs, scene: Scene, shader_cache: HashMap<String, Option<shader::Translated>>,
    t0: std::time::Instant, time_override: Option<f32>, videos: Vec<crate::video::VideoTex>, texts: Vec<crate::text::Live>, parts: crate::particles::Set,
    prov: HashMap<i64, usize>,          // scene object id -> layer index (render target producers)
    frame_state: Vec<u8>,               // per frame: 0 not rendered, 1 in progress (cycle guard), 2 done
    frame_tex: Vec<Option<(u32, i32, i32)>>,
    audio: Option<crate::audiocap::Capture>,
    audio_tried: bool,
    spec: crate::audiocap::Snapshot,
}

/// `_rt_imageLayerComposite_<id>_a` -> Some(id): another layer's rendered output.
fn rt_layer_id(name: &str) -> Option<i64> {
    let rest = name.strip_prefix("_rt_imageLayerComposite_")?;
    let d: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    d.parse().ok()
}

/// Render-target ids referenced by an object's effect chain (scene overrides, effect passes and their materials).
fn rt_refs(vfs: &Vfs, o: &scene::Obj, out: &mut Vec<i64>) {
    let mut add = |n: &str| { if let Some(id) = rt_layer_id(n) { out.push(id); } };
    for e in &o.effects {
        for p in &e.passes { for t in p.textures.iter().flatten() { add(t); } }
        let Some(d) = scene::load_effect(vfs, &e.file) else { continue };
        for p in &d.passes {
            for t in p.textures.iter().flatten() { add(t); }
            for (n, _) in &p.bind { add(n); }
            if let Some(m) = scene::load_material(vfs, &p.material) { for t in m.textures.iter().flatten() { add(t); } }
        }
    }
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

fn blend_of(s: &str) -> Blend { match s { "additive" => Blend::Additive, "multiply" => Blend::Multiply, "screen" => Blend::Screen, _ => Blend::Alpha } }

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
        let mut e = Engine { layers: vec![], prog, u_mvp: loc(prog, "mvp"), u_size: loc(prog, "size"), u_tint: loc(prog, "tint"), u_pm: loc(prog, "pm"), u_flip: loc(prog, "flip"), u_uvs: loc(prog, "uvs"), u_uvo: loc(prog, "uvo"),
            vao, view: scene::ident(), clear: scene.clear, scene_wh: (scene.width, scene.height), animated: false, audio_dirty: false, vram: 0, notes: vec![], scene_rt: None, white: 0, black: 0,
            out, screen: out, main_rt: None, scaling, vfs, scene, shader_cache: HashMap::new(), t0: std::time::Instant::now(), time_override: None, videos: vec![], texts: vec![], parts: Default::default(), prov: HashMap::new(), frame_state: vec![], frame_tex: vec![],
            audio: None, audio_tried: false, spec: crate::audiocap::Snapshot::ZERO };
        e.rebuild();
        Ok(e)
    }

    pub fn set_time(&mut self, t: Option<f32>) { self.time_override = t; }

    /// Screenshot mode draws its frames back to back; give the capture thread a moment to deliver
    /// the first spectrum so audio-reactive effects are not photographed before they wake up.
    pub fn wait_audio(&mut self, max: std::time::Duration) {
        let Some(a) = &self.audio else { return };
        let t0 = std::time::Instant::now();
        while t0.elapsed() < max {
            self.spec = a.snapshot();
            if self.spec.s64a.iter().any(|&v| v > 0.01) { return; }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    pub fn resize(&mut self, out: (i32, i32)) { if out != self.out { self.out = out; self.rebuild(); } }

    /// Size of the surface we present to (the layer surface); differs from `out` when rendering scaled.
    pub fn set_screen(&mut self, screen: (i32, i32)) { self.screen = (screen.0.max(1), screen.1.max(1)); }


    /// World matrix of a hidden layer (parents may be hidden as well): producers are drawn at their real place.
    fn world_hidden(&self, i: usize, depth: u32) -> Mat4 {
        let o = &self.scene.hidden[i];
        let local = scene::trs(o.origin, o.scale, o.angles[2]);
        if depth > 16 { return local; }
        let Some(p) = o.parent else { return local };
        if let Some(pi) = self.scene.hidden.iter().position(|x| x.id == p) { if pi != i { return scene::mul(&self.world_hidden(pi, depth + 1), &local); } }
        match self.scene.objects.iter().position(|x| x.id == p) { Some(pi) => scene::mul(&world_of(&self.scene.objects, pi, depth + 1), &local), None => local }
    }

    fn white_tex(&mut self) -> u32 {
        if self.white == 0 { unsafe {
            gl::GenTextures(1, &mut self.white); gl::BindTexture(gl::TEXTURE_2D, self.white);
            let px = [255u8; 4];
            gl::TexImage2D(gl::TEXTURE_2D, 0, gl::RGBA8 as i32, 1, 1, 0, gl::RGBA, gl::UNSIGNED_BYTE, px.as_ptr() as *const _);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::NEAREST as i32); gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::NEAREST as i32);
        } }
        self.white
    }

    /// 1x1 black: Wallpaper Engine's unpainted mask for an enabled mask property (mask 0 = keep the pixel).
    fn black_tex(&mut self) -> u32 {
        if self.black == 0 { unsafe {
            gl::GenTextures(1, &mut self.black); gl::BindTexture(gl::TEXTURE_2D, self.black);
            let px = [0u8, 0, 0, 255];
            gl::TexImage2D(gl::TEXTURE_2D, 0, gl::RGBA8 as i32, 1, 1, 0, gl::RGBA, gl::UNSIGNED_BYTE, px.as_ptr() as *const _);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::NEAREST as i32); gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::NEAREST as i32);
        } }
        self.black
    }

    fn note(&mut self, s: String) { eprintln!("wallforge-render: {s}"); self.notes.push(s); }

    /// (Re)build all GPU resources for the current output size (texture mips / FBO sizes depend on it).
    fn rebuild(&mut self) {
        // free the previous layers' buffers once each (the effect pool shares slots between layers)
        {
            let mut seen: std::collections::HashSet<u32> = std::collections::HashSet::new();
            for l in &self.layers {
                let mut all: Vec<Target> = vec![];
                if let Some((a, b)) = l.ping { all.push(a); all.push(b); }
                if let Some(rt) = l.rt { all.push(rt); }
                all.extend(l.named.values().copied());
                for t in all { if t.fbo != 0 && seen.insert(t.fbo) { t.free(); } }
            }
        }
        self.videos.clear(); self.texts.clear(); self.layers.clear(); self.parts.clear(); self.vram = 0; self.animated = false; self.scene_rt = None;
        self.prov.clear(); self.frame_state.clear(); self.frame_tex.clear();
        if let Some(rt) = self.main_rt.take() { rt.free(); }
        let (v, sx, sy) = view_matrix(self.scene_wh.0, self.scene_wh.1, self.out.0, self.out.1, self.scaling);
        self.view = v;
        let s3tc = tex::has_s3tc();
        let mut cache: HashMap<String, Option<GlTex>> = HashMap::new();
        // Effect buffers are only live while one layer renders, so layers share them by (size, slot).
        let mut pool: HashMap<(i32, i32, usize), Target> = HashMap::new();
        // Cross-layer render targets: hidden layers referenced by `_rt_imageLayerComposite_<id>_a` are built too
        // (never composited), and any layer others sample gets exclusive buffers.
        let mut referenced: std::collections::HashSet<i64> = std::collections::HashSet::new();
        let mut muted: Vec<usize> = vec![];
        {
            let mut queue: Vec<i64> = vec![];
            let mut refs = vec![];
            for o in &self.scene.objects { refs.clear(); rt_refs(&self.vfs, o, &mut refs); for id in &refs { referenced.insert(*id); queue.push(*id); } }
            let mut seen: std::collections::HashSet<i64> = std::collections::HashSet::new();
            while let Some(id) = queue.pop() {
                if !seen.insert(id) { continue; }
                let hit = self.scene.hidden.iter().position(|o| o.id == id).map(|j| (true, j))
                    .or_else(|| self.scene.objects.iter().position(|o| o.id == id).map(|j| (false, j)));
                let Some((hid, j)) = hit else { continue };
                if hid && !muted.contains(&j) { muted.push(j); }
                refs.clear();
                rt_refs(&self.vfs, if hid { &self.scene.hidden[j] } else { &self.scene.objects[j] }, &mut refs);
                for id in &refs { referenced.insert(*id); queue.push(*id); }
            }
            muted.sort();
        }
        let n = self.scene.objects.len();
        let mut order: Vec<(bool, usize)> = muted.iter().map(|j| (true, *j)).collect();
        order.extend((0..n).map(|i| (false, i)));
        for (hid, i) in order {
            let o = if hid { self.scene.hidden[i].clone() } else { self.scene.objects[i].clone() };
            if let Kind::Particle { file, ov } = &o.kind {
                let (w, at) = (if hid { scene::ident() } else { world_of(&self.scene.objects, i, 0) }, self.layers.len());
                self.parts.add(&self.vfs, file, ov, w, at, s3tc, i as u64); self.animated |= !self.parts.is_empty(); continue;
            }
            if let Kind::Text(td) = &o.kind { if hid { continue; } let td = (**td).clone(); self.add_text(i, td, sx, sy, &mut cache, s3tc, &mut pool); continue; }
            let (name, model, size_o, scale, color, alpha, blend_o) = {
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
            let up = if matches!(t.payload, tex::Payload::Mp4(_)) { tex::upload_video(&t, px.0 as u32, px.1 as u32, &meta).map(|(g, v)| { self.videos.push(v); self.animated = true; g }) } else { tex::upload(&t, px.0 as u32, px.1 as u32, &meta, s3tc) };
            let gt = match up { Ok(g) => g, Err(e) => { self.note(format!("layer '{name}': {e}, skipped")); continue } };
            frames = t.frames.clone(); tex_wh = (t.tex_w as f32, t.tex_h as f32);
            (size, px, gt)
            };
            if frames.len() > 1 { self.animated = true; }
            self.vram += gt.bytes;
            if std::env::var("WF_DBG").is_ok() { eprintln!("vram: layer '{name}' tex {:.1} MiB ({}x{}) px {:?}", gt.bytes as f64 / 1048576.0, gt.w, gt.h, px); }
            let world = if hid { self.world_hidden(i, 0) } else {
                let w = world_of(&self.scene.objects, i, 0);
                if o.align == [0.0, 0.0] { w } else { scene::mul(&w, &scene::trs([o.align[0] * size[0], o.align[1] * size[1], 0.0], [1.0; 3], 0.0)) }
            };
            let sampled = referenced.contains(&o.id);
            let mut layer = Layer { name: name.clone(), tex: gt.id, size, world, color, alpha, uvs: gt.uv, frames, tex_wh, full, blend: blend_of(blend_o.as_deref().unwrap_or(&def.material.blending)),
                passes: vec![], ping: None, rt: None, named: HashMap::new(), px, id: o.id, hidden: hid, sampled, composite: false };
            if def.material.blending == "additive" { layer.blend = Blend::Additive; }
            self.build_effects(&o, &mut layer, full, &mut cache, s3tc, &mut pool);
            if sampled {
                let rt = Target::new(self.out.0.max(1), self.out.1.max(1));
                self.vram += rt.bytes(); layer.rt = Some(rt);
            }
            if hid || sampled { if std::env::var("WF_DBG").is_ok() { eprintln!("rt: layer {} '{name}' {}x{} hidden={hid} sampled={sampled} passes={}", o.id, px.0, px.1, layer.passes.len()); } }
            let li = self.layers.len();
            self.layers.push(layer);
            if matches!(o.kind, Kind::Image { .. }) { self.prov.insert(o.id, li); }
        }
        if self.layers.iter().any(|l| l.full) {
            let rt = Target::new(self.out.0.max(1), self.out.1.max(1));
            self.vram += rt.bytes(); self.scene_rt = Some(rt);
        }
        for (k, t) in cache { if let Some(t) = t { if std::env::var("WF_DBG").is_ok() { eprintln!("vram: cached {k} {:.1} MiB {}x{}", t.bytes as f64 / 1048576.0, t.w, t.h); } self.vram += t.bytes; } }
        let mut missing: Vec<String> = vec![];
        for l in &self.layers {
            for p in &l.passes { for (_, n) in &p.binds {
                if n == "previous" || l.named.contains_key(n) { continue; }
                match rt_layer_id(n) { Some(id) if !self.prov.contains_key(&id) => missing.push(format!("layer '{}': effect samples render target {n} of a layer that is not rendered", l.name)), None => missing.push(format!("layer '{}': effect samples unsupported buffer {n}", l.name)), _ => {} }
            } }
        }
        for m in missing { self.note(m); }
        // Only pay for capture when some effect actually samples the audio spectrum; never when the
        // user turned audio processing off.
        let wants_audio = self.layers.iter().any(|l| l.passes.iter().any(|p| p.uniforms.iter().any(|(u, _)| u.name.starts_with("g_AudioSpectrum"))));
        if wants_audio && !self.audio_tried && !crate::audiocap::DISABLED.load(std::sync::atomic::Ordering::Relaxed) {
            self.audio_tried = true;
            self.audio = crate::audiocap::Capture::start();
        }
        eprintln!("wallforge-render: scene {}x{} -> {}x{}: {} layers, {} with effects, texture+fbo memory {:.1} MiB, animated={}",
            self.scene_wh.0, self.scene_wh.1, self.out.0, self.out.1, self.layers.len(), self.layers.iter().filter(|l| !l.passes.is_empty()).count(), self.vram as f64 / 1048576.0, self.animated);
    }

    fn build_pass(&mut self, ename: &str, ep: &scene::EffectPass, ov: Option<&(Map<String, Value>, Vec<Option<String>>, Map<String, Value>)>,
                  cache: &mut HashMap<String, Option<GlTex>>, s3tc: bool, want: (u32, u32)) -> Result<Pass, String> {
        if ep.material.is_empty() {
            // Wallpaper Engine allows a pass without a material: it copies the previous buffer into the target.
            return Ok(Pass { copy: true, prog: 0, uniforms: vec![], values: HashMap::new(), uvr: vec![], textures: vec![],
                             binds: vec![(0, "previous".to_string())], target: ep.target.clone(), time: false });
        }
        let mat = scene::load_material(&self.vfs, &ep.material).ok_or_else(|| format!("material {} not found", ep.material))?;
        let mut combos = mat.combos.clone();
        if let Some((_, _, c)) = ov { for (k, v) in c { combos.insert(k.clone(), v.clone()); } }
        let mut present: Vec<usize> = vec![];
        // Mask properties the material's shader declares behind a combo (blurprecise's ENABLEMASK): when that gate is
        // on and the pass brings no mask texture, Wallpaper Engine uses its unpainted (black) mask, so the sampler has
        // to be considered bound — running such a pass unmasked blurs layers WE leaves sharp.
        let masked: Vec<usize> = {
            let vfs = &self.vfs;
            shader::mask_slots(&|n| vfs.read_str(n), &mat.shader).into_iter()
                .filter(|(_, req)| req.iter().all(|(k, v)| combos.get(k).map(|c| c.to_string() == v.to_string()).unwrap_or(false)))
                .map(|(i, _)| i).collect()
        };
        {
            let mut names: std::collections::BTreeMap<usize, bool> = std::collections::BTreeMap::new();
            for (i, t) in mat.textures.iter().enumerate() { names.insert(i, t.is_some()); }
            for (i, t) in ep.textures.iter().enumerate() { if t.is_some() { names.insert(i, true); } }
            if let Some((_, ts, _)) = ov { for (i, x) in ts.iter().enumerate() { if x.is_some() { names.insert(i, true); } } }
            for i in &masked { names.entry(*i).or_insert(true); }
            for (i, b) in names { if b { present.push(i); } }
        }
        let black = self.black_tex();  // 1x1 black: for mask gates and for engine targets we do not render
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
                    Ok(mut t2) => { if std::env::var("WF_DBG").is_ok() { eprintln!("retry frag has float(: {}", t2.frag.contains("float(sampleCount)")); }
                        // HLSL-style implicit conversions: patch the source from the info log and relink
                        let mut r = link(&t2.vert, &t2.frag);
                        for _ in 0..12 { match &r { Err(e) if shader::repair(&mut t2, e) => r = link(&t2.vert, &t2.frag), _ => break } }
                        match r { Ok(p) => Ok((p, Some(t2))), Err(e2) => Err((e2, Some(t2))) } },
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
        // sampler annotation `"default": "pattern/x"` supplies the texture when nothing is bound to the slot
        for u in &tr.uniforms { if let (Some(i), Some(d)) = (u.name.strip_prefix("g_Texture").and_then(|n| n.parse::<usize>().ok()), u.default.as_ref().and_then(|d| d.as_str())) {
            if i > 0 && u.ty == "sampler2D" && !d.is_empty() { if names.len() <= i { names.resize(i + 1, None); } if names[i].is_none() { names[i] = Some(d.to_string()); } }
        } }
        let mut textures = vec![]; let mut uvr: Vec<[f32; 2]> = vec![]; let mut binds = vec![(0usize, "previous".to_string())];
        if let Some(mx) = ep.bind.iter().map(|b| b.1 + 1).max() { if names.len() < mx { names.resize(mx, None); } }
        for i in &masked { if names.get(*i).map_or(true, |n| n.is_none()) { if names.len() <= *i { names.resize(*i + 1, None); } names[*i] = Some(BLACK_MASK.to_string()); } }
        for (i, n) in names.iter().enumerate() {
            let bound = ep.bind.iter().find(|b| b.1 == i).map(|b| b.0.clone());
            if let Some(b) = bound { binds.retain(|x| x.0 != i); binds.push((i, b)); textures.push(None); uvr.push([1.0, 1.0]); continue; }
            match n {
                Some(n) if n == BLACK_MASK => { textures.push(Some((black, (1, 1)))); uvr.push([1.0, 1.0]); }
                Some(n) if n.starts_with("_rt_imageLayerComposite") => {
                    // another layer's rendered output: bound by name at draw time (see Engine::ensure)
                    if rt_layer_id(n).is_none() { return Err(format!("samples render target ({n}) in slot {i}, which is not supported")); }
                    binds.retain(|x| x.0 != i); binds.push((i, n.clone())); textures.push(None); uvr.push([1.0, 1.0]);
                }
                Some(n) if !n.starts_with("_rt_") && !n.contains("_alias_") => {
                    let path = scene::tex_file(n);
                    if !cache.contains_key(&path) {
                        let r = self.vfs.read(&path).and_then(|b| tex::parse(&b, false).ok()).and_then(|t| tex::upload(&t, want.0, want.1, &TexMeta::default(), s3tc).ok());
                        cache.insert(path.clone(), r);
                    }
                    match cache.get(&path).and_then(|x| x.as_ref()) { Some(g) => { textures.push(Some((g.id, (g.w, g.h)))); uvr.push(g.uv) }, None => return Err(format!("texture {path} not found")) }
                }
                // Engine-internal targets we do not render (shadow atlas aliases, ...): a cleared black texture keeps
                // the pass compiling instead of dropping the whole effect chain.
                Some(n) if n.starts_with("materials/_alias_") || n.starts_with("_rt_") => { textures.push(Some((black, (1, 1)))); uvr.push([1.0, 1.0]); }
                _ => { textures.push(None); uvr.push([1.0, 1.0]) },
            }
        }
        Ok(Pass { copy: false, prog, uniforms, values, uvr, textures, binds, target: ep.target.clone(), time: tr.uses_time })
    }

    /// Builds `o`'s effect chain into `layer` and allocates its ping-pong / named buffers.
    /// Shared by image and text layers: the layer's own texture is the chain's input.
    fn build_effects(&mut self, o: &scene::Obj, layer: &mut Layer, full: bool,
                     cache: &mut HashMap<String, Option<GlTex>>, s3tc: bool, pool: &mut HashMap<(i32, i32, usize), Target>) {
        let (name, px, sampled) = (layer.name.clone(), layer.px, layer.sampled);
        let effects: Vec<_> = o.effects.iter().map(|e| (e.file.clone(), e.name.clone(), e.passes.iter().map(|p| (p.constants.clone(), p.textures.clone(), p.combos.clone())).collect::<Vec<_>>())).collect();
        let mut fbo_div: HashMap<String, f32> = HashMap::new();
        let effects: Vec<_> = effects.into_iter().take(std::env::var("WF_MAXFX").ok().and_then(|v| v.parse().ok()).unwrap_or(usize::MAX)).collect();
        let only: Option<usize> = std::env::var("WF_ONLYFX").ok().and_then(|v| v.parse().ok()); let effects: Vec<_> = effects.into_iter().enumerate().filter(|(i, _)| only.map_or(true, |o| o == *i)).map(|x| x.1).collect();
        for (file, ename, ovs) in effects {
            let Some(ed) = scene::load_effect(&self.vfs, &file) else { self.note(format!("layer '{name}': effect {file} not found, skipped")); continue };
            for f in &ed.fbos { fbo_div.insert(f.name.clone(), f.scale.max(1.0)); }
            let mut built = vec![];
            let mut ok = true;
            for (pi, ep) in ed.passes.iter().enumerate() {
                match self.build_pass(&ename, ep, ovs.get(pi), cache, s3tc, (px.0 as u32, px.1 as u32)) { Ok(p) => built.push(p), Err(e) => { self.note(format!("layer '{name}': effect {file} pass {pi}: {e}; effect skipped")); ok = false; break; } }
            }
            if ok { layer.passes.extend(built); }
        }
        layer.composite = full && layer.passes.iter().any(|p| p.binds.iter().any(|(_, n)| n != "previous" && !layer.named.contains_key(n) && rt_layer_id(n).is_some()));
        if layer.passes.is_empty() { return; }
        let (w, h) = (px.0.clamp(1, 4096), px.1.clamp(1, 4096));
        let mut vram = 0usize;
        let mut grab = |slot: usize, tw: i32, th: i32, vram: &mut usize| -> Target {
            if sampled { let t = Target::new(tw, th); *vram += t.bytes(); t }
            else { *pool.entry((tw, th, slot)).or_insert_with(|| { let t = Target::new(tw, th); *vram += t.bytes(); t }) }
        };
        let (a, b) = (grab(0, w as i32, h as i32, &mut vram), grab(1, w as i32, h as i32, &mut vram));
        layer.ping = Some((a, b));
        // Passes may sample buffers declared in the effect's `fbos` even when no pass writes them by name, so every
        // declared buffer needs a target; otherwise the chain looks like it samples an unsupported buffer.
        let declared: Vec<String> = layer.passes.iter().flat_map(|p| p.binds.iter().map(|(_, n)| n.clone())).filter(|n| fbo_div.contains_key(n)).collect();
        for t in &declared {
            if !layer.named.contains_key(t) {
                let d = fbo_div.get(t).copied().unwrap_or(1.0);
                let slot = 2 + layer.named.len();
                let tg = grab(slot, ((w as f32 / d) as i32).max(1), ((h as f32 / d) as i32).max(1), &mut vram);
                if std::env::var("WF_DBG").is_ok() { eprintln!("vram: fbo {t} {}x{}", tg.w, tg.h); }
                layer.named.insert(t.clone(), tg);
            }
        }
        for p in &layer.passes {
            if let Some(t) = &p.target {
                if !layer.named.contains_key(t) {
                    let d = fbo_div.get(t).copied().unwrap_or(1.0);
                    let slot = 2 + layer.named.len();
                    let tg = grab(slot, ((w as f32 / d) as i32).max(1), ((h as f32 / d) as i32).max(1), &mut vram);
                    if std::env::var("WF_DBG").is_ok() { eprintln!("vram: fbo {t} {}x{}", tg.w, tg.h); }
                    layer.named.insert(t.clone(), tg);
                }
            }
            if p.time { self.animated = true; }
        }
        self.vram += vram;
    }

    /// Stage-2 hook: audio spectrum / particle simulation step goes here.
    /// Text layer for scene object `i` (rasterised with the layer's on-screen pixel size).
    fn add_text(&mut self, i: usize, td: crate::text::TextDef, sx: f32, sy: f32,
                cache: &mut HashMap<String, Option<GlTex>>, s3tc: bool, pool: &mut HashMap<(i32, i32, usize), Target>) {
        let o = self.scene.objects[i].clone();
        let Some(size) = o.size else { return };
        let (name, scale, color, alpha, align, obj_id) = (o.name.clone(), o.scale, o.color, o.alpha.clamp(0.0, 1.0), o.align, o.id);
        let px = ((size[0] * scale[0].abs() * sx).ceil().max(1.0) as i32, (size[1] * scale[1].abs() * sy).ceil().max(1.0) as i32);
        let world = world_of(&self.scene.objects, i, 0);
        let world = if align == [0.0, 0.0] { world } else { scene::mul(&world, &scene::trs([align[0] * size[0], align[1] * size[1], 0.0], [1.0; 3], 0.0)) };
        let vfs = &self.vfs;
        let (live, missing) = crate::text::Live::new(td, &|p| vfs.read(p), px.0.clamp(1, 4096) as u32, px.1.clamp(1, 4096) as u32, (sx * scale[0].abs()).max(1e-3), self.layers.len());
        if let Some(f) = missing { if !self.notes.iter().any(|n| n.contains(&f)) { self.note(format!("text '{name}': font '{f}' not found, using a fallback font")); } }
        let mut layer = Layer { name: name.clone(), tex: live.tex, size, world, color, alpha, uvs: [1.0, 1.0], frames: vec![], tex_wh: (1.0, 1.0), full: false, blend: Blend::Alpha, passes: vec![], ping: None, rt: None, named: HashMap::new(), px, id: obj_id, hidden: false, sampled: false, composite: false };
        if !o.effects.is_empty() {
            // A text layer cannot produce a scene-space target for other chains to sample (no `rt`), and
            // hidden producers are not built here: skip such chains instead of rendering them wrong once.
            let mut refs = vec![];
            rt_refs(&self.vfs, &o, &mut refs);
            if refs.is_empty() {
                self.build_effects(&o, &mut layer, false, cache, s3tc, pool);
            } else {
                self.note(format!("text '{name}': effect chain samples another layer's render target, effects skipped"));
            }
        }
        self.texts.push(live); self.layers.push(layer);
    }
    /// Refreshes time-driven text (checked at most once a second); true when something changed and a redraw is needed.
    pub fn text_tick(&mut self) -> bool { let mut c = false; for t in &mut self.texts { if t.refresh() { c = true; } } c }
    /// When the next time-driven text check is due.
    pub fn text_deadline(&self) -> Option<std::time::Instant> { self.texts.iter().filter(|t| t.is_dynamic()).map(|t| t.next_check()).min() }
    pub fn tick(&mut self) {
        let fixed = self.time_override.map(|t| t as f64);
        for v in &mut self.videos { v.update(fixed); }
        if let Some(a) = &self.audio {
            let s = a.snapshot();
            self.audio_dirty = s != self.spec;  // visualisers only redraw when the spectrum moved
            self.spec = s;
        }
    }

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
            if let Some(n) = name.strip_prefix("g_AudioSpectrum") {
                let digits: String = n.chars().take_while(|c| c.is_ascii_digit()).collect();
                let size = digits.parse::<usize>().unwrap_or(16);
                let bands = self.spec.spectrum(size, &n[digits.len()..]);
                if !bands.is_empty() {
                    gl::Uniform1fv(l, bands.len() as i32, bands.as_ptr()); return;
                }
                let z = [0.0f32; 64]; let c = size.min(64).max(1);
                gl::Uniform1fv(l, c as i32, z.as_ptr()); return;
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
        // When `screen != out` the whole frame is composed into an internal-size target, then blitted up.
        let scaled = self.screen != self.out;
        let main_fbo = if scaled {
            match self.main_rt {
                Some(rt) if rt.w == self.out.0 && rt.h == self.out.1 => rt.fbo,
                _ => {
                    if let Some(old) = self.main_rt.take() { old.free(); }
                    let rt = Target::new(self.out.0.max(1), self.out.1.max(1));
                    self.main_rt = Some(rt);
                    rt.fbo
                }
            }
        } else {
            if let Some(old) = self.main_rt.take() { old.free(); }
            0
        };
        let rt_fbo = self.scene_rt.as_ref().map(|r| r.fbo).unwrap_or(main_fbo);
        unsafe {
            gl::BindFramebuffer(gl::FRAMEBUFFER, rt_fbo);
            gl::Viewport(0, 0, self.out.0, self.out.1);
            gl::ClearColor(self.clear[0], self.clear[1], self.clear[2], 1.0);
            gl::Clear(gl::COLOR_BUFFER_BIT);
            gl::BindVertexArray(self.vao);
        }
        self.parts.step(t);
        let nlayers = self.layers.len();
        self.frame_state.clear(); self.frame_state.resize(nlayers, 0);
        self.frame_tex.clear(); self.frame_tex.resize(nlayers, None);
        for li in 0..nlayers {
            // a layer's effect chain leaves its own FBO/viewport bound; particles need the frame target again
            if self.parts.is_empty() == false { unsafe { gl::BindFramebuffer(gl::FRAMEBUFFER, rt_fbo); gl::Viewport(0, 0, self.out.0, self.out.1); } }
            self.draw_parts(li, rt_fbo);
            let (full, has_ping) = (self.layers[li].full, self.layers[li].ping.is_some());
            if self.layers[li].hidden { continue; } // render-target producer: drawn on demand by its consumers
            if full && !has_ping { continue; }
            let src_tex = self.ensure(li, t).0;
            let l = &self.layers[li];
            let mvp = scene::mul(&self.view, &l.world);
            let mvp = if l.full { scene::ident() } else { mvp };
            let lsize = if l.full { [2.0, 2.0] } else { l.size };
            unsafe {
                gl::BindFramebuffer(gl::FRAMEBUFFER, rt_fbo);
                gl::Viewport(0, 0, self.out.0, self.out.1);
                if l.full && !l.composite { gl::Disable(gl::BLEND); } else { gl::Enable(gl::BLEND); }
                match l.blend { Blend::Alpha => gl::BlendFuncSeparate(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA, gl::ONE, gl::ONE_MINUS_SRC_ALPHA), Blend::Additive => gl::BlendFunc(gl::SRC_ALPHA, gl::ONE), Blend::Multiply => gl::BlendFuncSeparate(gl::DST_COLOR, gl::ONE_MINUS_SRC_ALPHA, gl::ZERO, gl::ONE), Blend::Screen => gl::BlendFuncSeparate(gl::ONE_MINUS_DST_COLOR, gl::ONE, gl::ZERO, gl::ONE) }
                gl::UseProgram(self.prog);
                gl::Uniform1f(self.u_pm, if matches!(l.blend, Blend::Multiply | Blend::Screen) { 1.0 } else { 0.0 });
                gl::UniformMatrix4fv(self.u_mvp, 1, gl::FALSE, mvp.as_ptr());
                gl::Uniform2f(self.u_size, lsize[0], lsize[1]);
                gl::Uniform4f(self.u_tint, l.color[0], l.color[1], l.color[2], l.alpha);
                gl::Uniform1f(self.u_flip, 0.0);
                if l.ping.is_some() { gl::Uniform2f(self.u_uvs, 1.0, 1.0); gl::Uniform2f(self.u_uvo, 0.0, 0.0); } else { let (uv, uo) = l.cell(t); gl::Uniform2f(self.u_uvs, uv[0], uv[1]); gl::Uniform2f(self.u_uvo, uo[0], uo[1]); }
                gl::ActiveTexture(gl::TEXTURE0); gl::BindTexture(gl::TEXTURE_2D, src_tex);
                gl::DrawArrays(gl::TRIANGLE_STRIP, 0, 4);
                gl::Uniform1f(self.u_pm, 0.0);
                gl::Disable(gl::BLEND);
            }
        }
        self.draw_parts(self.layers.len(), rt_fbo);
        if let Some(rt) = &self.scene_rt {
            unsafe {
                gl::BindFramebuffer(gl::FRAMEBUFFER, main_fbo);
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
        if scaled {
            unsafe {
                gl::BindFramebuffer(gl::READ_FRAMEBUFFER, main_fbo);
                gl::BindFramebuffer(gl::DRAW_FRAMEBUFFER, 0);
                gl::Viewport(0, 0, self.screen.0, self.screen.1);
                gl::Disable(gl::BLEND);
                gl::BlitFramebuffer(0, 0, self.out.0, self.out.1, 0, 0, self.screen.0, self.screen.1, gl::COLOR_BUFFER_BIT, gl::LINEAR);
                gl::BindFramebuffer(gl::FRAMEBUFFER, 0);
            }
        }
    }

    fn draw_parts(&mut self, at: usize, fbo: u32) { if self.parts.is_empty() { return; } self.parts.draw(at, &self.view, fbo, self.out); unsafe { gl::BindVertexArray(self.vao); } }

    /// Runs the layer's effect chain; returns the texture holding the final image.
    /// Draws a layer's finished image into its scene-space target, so other layers sample it at scene UVs
    /// (a fullscreen composition layer, or a layer covering the scene, sees the producer at its real place).
    fn export_layer(&mut self, li: usize, src: u32, t: f32) -> (u32, i32, i32) {
        let Some(rt) = self.layers[li].rt else { return (src, self.out.0, self.out.1) };
        let l = &self.layers[li];
        let mvp = if l.full { scene::ident() } else { scene::mul(&self.view, &l.world) };
        let lsize = if l.full { [2.0, 2.0] } else { l.size };
        let (uv, uo) = if l.ping.is_some() { ([1.0, 1.0], [0.0, 0.0]) } else { l.cell(t) };
        unsafe {
            gl::BindFramebuffer(gl::FRAMEBUFFER, rt.fbo);
            gl::Viewport(0, 0, rt.w, rt.h);
            gl::ClearColor(0.0, 0.0, 0.0, 0.0); gl::Clear(gl::COLOR_BUFFER_BIT);
            gl::Disable(gl::BLEND);
            gl::UseProgram(self.prog);
            gl::UniformMatrix4fv(self.u_mvp, 1, gl::FALSE, mvp.as_ptr());
            gl::Uniform2f(self.u_size, lsize[0], lsize[1]);
            gl::Uniform4f(self.u_tint, l.color[0], l.color[1], l.color[2], l.alpha);
            gl::Uniform1f(self.u_flip, 0.0);
            gl::Uniform2f(self.u_uvs, uv[0], uv[1]); gl::Uniform2f(self.u_uvo, uo[0], uo[1]);
            gl::ActiveTexture(gl::TEXTURE0); gl::BindTexture(gl::TEXTURE_2D, src);
            gl::DrawArrays(gl::TRIANGLE_STRIP, 0, 4);
        }
        (rt.tex, rt.w, rt.h)
    }

    /// Renders layer `li` at most once per frame; cross-layer producers are rendered first (recursively).
    /// Returns the texture holding the layer's image plus its size.
    fn ensure(&mut self, li: usize, t: f32) -> (u32, i32, i32) {
        if self.frame_state.len() <= li { self.frame_state.resize(li + 1, 0); self.frame_tex.resize(li + 1, None); }
        if let Some(v) = self.frame_tex[li] { return v; }
        if self.frame_state[li] == 1 {
            // sampling cycle: fall back to the producer's own texture
            let l = &self.layers[li];
            return (l.tex, (l.tex_wh.0.max(1.0)) as i32, (l.tex_wh.1.max(1.0)) as i32);
        }
        self.frame_state[li] = 1;
        let l0 = &self.layers[li];
        let src = if l0.full && !l0.composite { self.scene_rt.as_ref().map(|r| r.tex) } else { None };
        let r = self.render_effects(li, t, src);
        let r = if self.layers[li].sampled { self.export_layer(li, r.0, t) } else { r };
        self.frame_state[li] = 2;
        self.frame_tex[li] = Some(r);
        r
    }

    fn render_effects(&mut self, li: usize, t: f32, src: Option<u32>) -> (u32, i32, i32) {
        // layers whose render targets this chain samples must be drawn before the chain starts
        let mut deps: Vec<usize> = vec![];
        {
            let l = &self.layers[li];
            for p in &l.passes { for (_, n) in &p.binds {
                if n == "previous" || l.named.contains_key(n) { continue; }
                if let Some(d) = rt_layer_id(n).and_then(|id| self.prov.get(&id).copied()) { if d != li && !deps.contains(&d) { deps.push(d); } }
            } }
        }
        let mut ext: HashMap<String, (u32, i32, i32)> = HashMap::new();
        for d in deps {
            let v = self.ensure(d, t);
            let did = self.layers[d].id;
            let names: Vec<String> = self.layers[li].passes.iter().flat_map(|p| p.binds.iter().filter(|(_, n)| rt_layer_id(n) == Some(did)).map(|(_, n)| n.clone())).collect();
            for n in names { ext.insert(n, v); }
        }
        let l = &self.layers[li];
        let Some((a, b)) = l.ping else { return (l.tex, (l.tex_wh.0.max(1.0)) as i32, (l.tex_wh.1.max(1.0)) as i32) };
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
        let dbg_n: usize = std::env::var("WF_PASSN").ok().and_then(|v| v.parse().ok()).unwrap_or(usize::MAX); let mut dbg_last: Option<(u32, i32, i32)> = None;
        for p in l.passes.iter().take(dbg_n) {
            let dst = match &p.target { Some(n) => l.named.get(n).copied().unwrap_or(cur.1), None => cur.1 };
            if p.copy {
                unsafe {
                    gl::BindFramebuffer(gl::READ_FRAMEBUFFER, cur.0.fbo);
                    gl::BindFramebuffer(gl::DRAW_FRAMEBUFFER, dst.fbo);
                    gl::BlitFramebuffer(0, 0, cur.0.w, cur.0.h, 0, 0, dst.w, dst.h, gl::COLOR_BUFFER_BIT, gl::LINEAR);
                    gl::BindFramebuffer(gl::FRAMEBUFFER, dst.fbo);
                }
                if p.target.is_none() { cur = (cur.1, cur.0); dbg_last = None; } else { dbg_last = Some((dst.tex, dst.w, dst.h)); }
                continue;
            }
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
                    else if let Some(n) = l.named.get(&b.1) { (n.tex, (n.w as f32, n.h as f32)) }
                    else if let Some(e) = ext.get(&b.1) { (e.0, (e.1 as f32, e.2 as f32)) }
                    else { (cur.0.tex, (cur.0.w as f32, cur.0.h as f32)) }
                } else if let Some(Some((id, (w, h)))) = p.textures.get(unit) { (*id, (*w as f32, *h as f32)) } else { (cur.0.tex, (cur.0.w as f32, cur.0.h as f32)) };
                res.push(r);
                unsafe { gl::ActiveTexture(gl::TEXTURE0 + unit as u32); gl::BindTexture(gl::TEXTURE_2D, id); }
            }
            // vertex-displacing effects (foliagesway, ...) work in scene units: give them a quad of the layer's size and a matching MVP
            let pix = p.uniforms.iter().any(|(u, _)| u.name == "g_ModelViewProjectionMatrix") && l.size[0] > 0.0 && l.size[1] > 0.0;
            let mut emvp = ident;
            if pix {
                let (hw, hh) = (l.size[0] * 0.5, l.size[1] * 0.5);
                let v: [f32; 20] = [-hw, -hh, 0.0, 0.0, 0.0, hw, -hh, 0.0, 1.0, 0.0, -hw, hh, 0.0, 0.0, 1.0, hw, hh, 0.0, 1.0, 1.0];
                emvp[0] = 1.0 / hw; emvp[5] = 1.0 / hh;
                unsafe { let mut vb = 0; gl::GetVertexAttribiv(0, gl::VERTEX_ATTRIB_ARRAY_BUFFER_BINDING, &mut vb); gl::BindBuffer(gl::ARRAY_BUFFER, vb as u32); gl::BufferSubData(gl::ARRAY_BUFFER, 0, 80, v.as_ptr() as *const _); }
            }
            for (u, lc) in &p.uniforms { self.set_uniform(u, *lc, p, &emvp, t, &res); }
            unsafe { gl::DrawArrays(gl::TRIANGLE_STRIP, 0, 4); }
            if pix {
                let v: [f32; 20] = [-1.0, -1.0, 0.0, 0.0, 0.0, 1.0, -1.0, 0.0, 1.0, 0.0, -1.0, 1.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 1.0, 1.0];
                unsafe { gl::BufferSubData(gl::ARRAY_BUFFER, 0, 80, v.as_ptr() as *const _); }
            }
            if p.target.is_none() { cur = (cur.1, cur.0); dbg_last = None; } else { dbg_last = Some((dst.tex, dst.w, dst.h)); }
        }
        if let Some(t) = dbg_last { return t; }
        unsafe { gl::ActiveTexture(gl::TEXTURE0); }
        (cur.0.tex, cur.0.w, cur.0.h)
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

    /// Cross-layer sampling names: `_rt_imageLayerComposite_<id>_a` (and the `_b` variant) map to the scene object id.
    #[test]
    fn render_target_names() {
        assert_eq!(rt_layer_id("_rt_imageLayerComposite_42_a"), Some(42));
        assert_eq!(rt_layer_id("_rt_imageLayerComposite_310_b"), Some(310));
        assert_eq!(rt_layer_id("_rt_imageLayerComposite_7"), Some(7));
        assert_eq!(rt_layer_id("_rt_default"), None);
        assert_eq!(rt_layer_id("g_Texture0"), None);
        assert_eq!(rt_layer_id("_rt_imageLayerComposite_x_a"), None);
    }
}
