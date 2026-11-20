//! Wallpaper Engine particle systems (particles/*.json): CPU fixed-step simulation, one instanced draw per system.
//! Deterministic (seeded xorshift), prewarmed by the file's `starttime` and by simulating from t=0 to the requested time.
//! Unsupported operators/renderers are ignored with at most one `note:` line per system.
use crate::pkg::Vfs;
use crate::scene::{self, Mat4};
use crate::tex::{self, Fmt, Payload};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

/// `--disable-particles`
pub static DISABLED: AtomicBool = AtomicBool::new(false);

const STEP: f32 = 1.0 / 60.0;
const HARD_CAP: usize = 5000;
const MAX_CATCHUP: f32 = 30.0;
const TAU: f32 = std::f32::consts::TAU;

const VS: &str = "#version 300 es
layout(location=0) in vec4 a_pos; layout(location=1) in vec4 a_col; layout(location=2) in vec4 a_uv;
uniform mat4 mvp; uniform float ratio; out vec2 uv; out vec4 col;
void main(){ vec2 c = vec2(float(gl_VertexID & 1), float(gl_VertexID >> 1)) - 0.5;
 float s = sin(a_pos.w), co = cos(a_pos.w); vec2 q = vec2(c.x, c.y * ratio) * a_pos.z; q = vec2(co * q.x - s * q.y, s * q.x + co * q.y);
 gl_Position = mvp * vec4(a_pos.xy + q, 0.0, 1.0); uv = a_uv.xy + vec2(c.x + 0.5, 0.5 - c.y) * a_uv.zw; col = a_col; }";
const FS: &str = "#version 300 es
precision highp float; in vec2 uv; in vec4 col; out vec4 o; uniform sampler2D t; uniform float overbright;
void main(){ vec4 c = texture(t, uv); o = vec4(c.rgb * col.rgb * overbright, c.a * col.a); }";

// ---------------- rng ----------------
#[derive(Clone)]
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> f32 { self.0 ^= self.0 >> 12; self.0 ^= self.0 << 25; self.0 ^= self.0 >> 27; ((self.0.wrapping_mul(0x2545F4914F6CDD1D) >> 40) as f32) / 16777216.0 }
    fn range(&mut self, a: f32, b: f32) -> f32 { a + (b - a) * self.next() }
}
fn hash01(seed: u32, k: u32) -> f32 {
    let mut x = seed.wrapping_mul(0x9E3779B1) ^ k.wrapping_mul(0x85EBCA6B); x ^= x >> 15; x = x.wrapping_mul(0x2C1B3C6D); x ^= x >> 12; x = x.wrapping_mul(0x297A2D39); x ^= x >> 15;
    (x >> 8) as f32 / 16777216.0
}

// ---------------- definition ----------------
type V3 = [f32; 3];
fn fv(v: Option<&Value>) -> Option<Vec<f32>> {
    let v = v?;
    if let Some(x) = v.get("value") { return scene::floats(x); }
    scene::floats(v)
}
fn num(o: &Value, k: &str, d: f32) -> f32 { fv(o.get(k)).and_then(|v| v.first().copied()).unwrap_or(d) }
fn vec3(o: &Value, k: &str, d: V3) -> V3 {
    match fv(o.get(k)) { Some(a) if a.len() >= 3 => [a[0], a[1], a[2]], Some(a) if a.len() == 2 => [a[0], a[1], d[2]], Some(a) if a.len() == 1 => [a[0]; 3], _ => d }
}

#[derive(Clone, Copy, PartialEq)]
enum Blend { Add, Alpha }

struct Emitter { sphere: bool, origin: V3, dmin: V3, dmax: V3, dirs: V3, sign: V3, rate: f32, inst: f32, smin: f32, smax: f32, acc: f32 }
struct Range { min: f32, max: f32, exp: f32 }
impl Range {
    fn of(o: &Value, dmin: f32, dmax: f32) -> Range { Range { min: num(o, "min", dmin), max: num(o, "max", dmax), exp: num(o, "exponent", 1.0).max(0.01) } }
    fn pick(&self, r: &mut Rng) -> f32 { let x = r.next(); self.min + (self.max - self.min) * if self.exp == 1.0 { x } else { x.powf(self.exp) } }
}
struct Osc { fmin: f32, fmax: f32, smin: f32, smax: f32, pmin: f32, pmax: f32, mask: V3 }
struct Turb { scale: f32, smin: f32, smax: f32, ts: f32, mask: V3 }
struct Ramp { s: f32, e: f32, t0: f32, t1: f32 }
impl Ramp {
    fn at(&self, f: f32) -> f32 { let k = ((f - self.t0) / (self.t1 - self.t0).max(1e-4)).clamp(0.0, 1.0); self.s + (self.e - self.s) * k }
}

struct Def {
    maxcount: usize, emitters: Vec<Emitter>,
    life: Range, size: Range, alpha: Range, vel: ([V3; 2], f32), rot: [V3; 2], angv: [V3; 2], col: [V3; 2], tvel: Option<(f32, f32, f32)>,
    drag: f32, gravity: V3, angforce: V3, fade: (f32, f32), sizech: Vec<Ramp>, alphach: Vec<Ramp>, colch: Vec<([V3; 2], f32, f32)>,
    osc_a: Vec<Osc>, osc_s: Vec<Osc>, osc_p: Vec<Osc>, turb: Vec<Turb>, starttime: f32,
    blend: Blend, overbright: f32, texture: String, seq: bool, seqmul: f32,
}
struct Ov { count: f32, rate: f32, size: f32, speed: f32, life: f32, alpha: f32, color: V3 }

fn parse_def(j: &Value, vfs: &Vfs, ov: &Ov, notes: &mut Vec<String>) -> Option<Def> {
    let mat = scene::load_material(vfs, j.get("material")?.as_str()?)?;
    let arr = |k: &str| j.get(k).and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let mut d = Def {
        maxcount: ((num(j, "maxcount", 100.0) * ov.count).ceil().max(1.0) as usize).min(HARD_CAP), emitters: vec![],
        life: Range { min: 0.0, max: 1.0, exp: 1.0 }, size: Range { min: 0.0, max: 20.0, exp: 1.0 }, alpha: Range { min: 1.0, max: 1.0, exp: 1.0 },
        vel: ([[0.0; 3]; 2], 1.0), rot: [[0.0; 3], [0.0, 0.0, 0.0]], angv: [[0.0; 3]; 2], col: [[1.0; 3], [1.0; 3]], tvel: None,
        drag: 0.0, gravity: [0.0; 3], angforce: [0.0; 3], fade: (0.5, 1.0), sizech: vec![], alphach: vec![], colch: vec![],
        osc_a: vec![], osc_s: vec![], osc_p: vec![], turb: vec![], starttime: num(j, "starttime", 0.0).clamp(0.0, 60.0),
        blend: if mat.blending == "additive" { Blend::Add } else { Blend::Alpha },
        overbright: mat.constants.get("ui_editor_properties_overbright").or(mat.constants.get("overbright")).and_then(|v| fv(Some(v))).and_then(|v| v.first().copied()).unwrap_or(1.0),
        texture: mat.textures.first().cloned().flatten().unwrap_or_default(),
        seq: j.get("animationmode").and_then(|v| v.as_str()) != Some("randomframe"), seqmul: num(j, "sequencemultiplier", 1.0).max(0.01),
    };
    for e in arr("emitter") {
        let name = e.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let sphere = match name { "sphererandom" => true, "boxrandom" => false, o => { notes.push(format!("emitter {o}")); continue } };
        let dmax = if sphere { [num(&e, "distancemax", 0.0); 3] } else { vec3(&e, "distancemax", [0.0; 3]) };
        let dmin = if sphere { [num(&e, "distancemin", 0.0); 3] } else { vec3(&e, "distancemin", [-dmax[0], -dmax[1], -dmax[2]]) };
        let dmax = if sphere { dmax } else { vec3(&e, "distancemax", dmax) };
        d.emitters.push(Emitter { sphere, origin: vec3(&e, "origin", [0.0; 3]), dmin, dmax, dirs: vec3(&e, "directions", [1.0; 3]), sign: vec3(&e, "sign", [0.0; 3]),
            rate: num(&e, "rate", 5.0) * ov.rate * ov.count, inst: num(&e, "instantaneous", 0.0) * ov.count, smin: num(&e, "speedmin", 0.0), smax: num(&e, "speedmax", 0.0), acc: 0.0 });
    }
    for i in arr("initializer") {
        let name = i.get("name").and_then(|v| v.as_str()).unwrap_or("");
        match name {
            "lifetimerandom" => d.life = Range::of(&i, 0.0, 1.0),
            "sizerandom" => d.size = Range::of(&i, 0.0, 20.0),
            "alpharandom" => d.alpha = Range::of(&i, 1.0, 1.0),
            "velocityrandom" => d.vel = ([vec3(&i, "min", [0.0; 3]), vec3(&i, "max", [0.0; 3])], num(&i, "exponent", 1.0)),
            "rotationrandom" => d.rot = [vec3(&i, "min", [0.0; 3]), vec3(&i, "max", [0.0, 0.0, TAU])],
            "angularvelocityrandom" => d.angv = [vec3(&i, "min", [0.0, 0.0, -1.0]), vec3(&i, "max", [0.0, 0.0, 1.0])],
            "colorrandom" => d.col = [vec3(&i, "min", [0.0; 3]).map(|x| x / 255.0), vec3(&i, "max", [255.0; 3]).map(|x| x / 255.0)],
            "turbulentvelocityrandom" => d.tvel = Some((num(&i, "speedmin", 100.0), num(&i, "speedmax", 250.0), num(&i, "scale", 1.0))),
            o => notes.push(format!("initializer {o}")),
        }
    }
    let osc = |o: &Value, dsmin: f32, dsmax: f32| Osc { fmin: num(o, "frequencymin", 0.0), fmax: num(o, "frequencymax", 10.0), smin: num(o, "scalemin", dsmin), smax: num(o, "scalemax", dsmax),
        pmin: num(o, "phasemin", 0.0), pmax: num(o, "phasemax", TAU), mask: vec3(o, "mask", [1.0; 3]) };
    for o in arr("operator") {
        let name = o.get("name").and_then(|v| v.as_str()).unwrap_or("");
        match name {
            "movement" => { d.drag = num(&o, "drag", 0.0); d.gravity = vec3(&o, "gravity", [0.0; 3]); }
            "angularmovement" => d.angforce = vec3(&o, "force", [0.0; 3]),
            "alphafade" => d.fade = (num(&o, "fadeintime", 0.5), num(&o, "fadeouttime", 1.0)),
            "sizechange" => d.sizech.push(Ramp { s: num(&o, "startvalue", 1.0), e: num(&o, "endvalue", 0.0), t0: num(&o, "starttime", 0.0), t1: num(&o, "endtime", 1.0) }),
            "alphachange" => d.alphach.push(Ramp { s: num(&o, "startvalue", 1.0), e: num(&o, "endvalue", 0.0), t0: num(&o, "starttime", 0.0), t1: num(&o, "endtime", 1.0) }),
            "colorchange" => d.colch.push(([vec3(&o, "startvalue", [1.0; 3]), vec3(&o, "endvalue", [0.0; 3])], num(&o, "starttime", 0.0), num(&o, "endtime", 1.0))),
            "oscillatealpha" => d.osc_a.push(osc(&o, 0.0, 1.0)),
            "oscillatesize" => d.osc_s.push(osc(&o, 0.0, 1.0)),
            "oscillateposition" => d.osc_p.push(osc(&o, -1.0, 1.0)),
            "turbulence" => d.turb.push(Turb { scale: num(&o, "scale", 1.0), smin: num(&o, "speedmin", 100.0), smax: num(&o, "speedmax", 250.0), ts: num(&o, "timescale", 1.0), mask: vec3(&o, "mask", [1.0; 3]) }),
            o => notes.push(format!("operator {o}")),
        }
    }
    if mat.combos.get("REFRACT").and_then(|v| fv(Some(v))).and_then(|v| v.first().copied()).unwrap_or(0.0) != 0.0 { notes.push("refraction material (system not drawn)".into()); return None; }
    if arr("renderer").is_empty() { notes.push("no renderer (system not drawn)".into()); return None; }
    for r in arr("renderer") {
        match r.get("name").and_then(|v| v.as_str()).unwrap_or("") { "sprite" => {} o => notes.push(format!("renderer {o} drawn as sprite")) }
    }
    // instance overrides
    d.life.min *= ov.life; d.life.max *= ov.life; d.size.min *= ov.size; d.size.max *= ov.size; d.alpha.min *= ov.alpha; d.alpha.max *= ov.alpha;
    for k in 0..2 { for a in 0..3 { d.vel.0[k][a] *= ov.speed; d.col[k][a] *= ov.color[a]; } }
    if let Some(t) = &mut d.tvel { t.0 *= ov.speed; t.1 *= ov.speed; }
    d.emitters.iter_mut().for_each(|e| { e.smin *= ov.speed; e.smax *= ov.speed; });
    Some(d)
}

// ---------------- simulation ----------------
#[derive(Clone, Copy)]
struct P { pos: V3, vel: V3, rot: f32, angv: f32, age: f32, life: f32, size: f32, col: V3, alpha: f32, rnd: f32, seed: u32 }

struct Sys {
    def: Def, ps: Vec<P>, rng: Rng, t: f32, at: usize, world: Mat4, ratio: f32, frames: Vec<[f32; 4]>, full: [f32; 4],
    tex: u32, vao: u32, vbo: u32, cap_bytes: usize, buf: Vec<f32>, counter: u32,
}

fn spawn(d: &mut Def, ei: usize, rng: &mut Rng, counter: &mut u32) -> P {
    let e = &d.emitters[ei];
    let pos: V3 = if e.sphere {
        let mut v = [0.0f32; 3];
        for a in 0..3 { v[a] = rng.range(-1.0, 1.0) * e.dirs[a]; if e.sign[a] != 0.0 { v[a] = v[a].abs() * e.sign[a].signum(); } }
        let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        let dist = rng.range(e.dmin[0], e.dmax[0]);
        if l > 1e-6 { [e.origin[0] + v[0] / l * dist, e.origin[1] + v[1] / l * dist, e.origin[2] + v[2] / l * dist] } else { e.origin }
    } else {
        let mut p = e.origin;
        for a in 0..3 { let mut x = rng.range(e.dmin[a], e.dmax[a]); if e.sign[a] != 0.0 { x = x.abs() * e.sign[a].signum(); } p[a] += x; }
        p
    };
    let esp = if e.smax != 0.0 || e.smin != 0.0 { rng.range(e.smin, e.smax) } else { 0.0 };
    let mut vel = [0.0f32; 3];
    for a in 0..3 { let x = rng.next(); let x = if d.vel.1 != 1.0 { x.powf(d.vel.1.max(0.01)) } else { x }; vel[a] = d.vel.0[0][a] + (d.vel.0[1][a] - d.vel.0[0][a]) * x; }
    if esp != 0.0 { let l = [pos[0] - e.origin[0], pos[1] - e.origin[1], pos[2] - e.origin[2]]; let n = (l[0] * l[0] + l[1] * l[1] + l[2] * l[2]).sqrt().max(1e-6); for a in 0..3 { vel[a] += l[a] / n * esp; } }
    if let Some((smin, smax, _)) = d.tvel { let a = rng.range(0.0, TAU); let s = rng.range(smin, smax); vel[0] += a.cos() * s; vel[1] += a.sin() * s; }
    let col = [rng.range(d.col[0][0], d.col[1][0]), rng.range(d.col[0][1], d.col[1][1]), rng.range(d.col[0][2], d.col[1][2])];
    *counter = counter.wrapping_add(1);
    P { pos, vel, rot: rng.range(d.rot[0][2], d.rot[1][2]), angv: rng.range(d.angv[0][2], d.angv[1][2]), age: 0.0, life: d.life.pick(rng).max(0.01), size: d.size.pick(rng).max(0.0),
        col, alpha: d.alpha.pick(rng), rnd: rng.next(), seed: *counter }
}

impl Sys {
    fn step(&mut self, dt: f32) {
        let d = &mut self.def;
        let (grav, drag, angf) = (d.gravity, d.drag, d.angforce[2]);
        self.ps.retain_mut(|p| {
            p.age += dt;
            if p.age >= p.life { return false; }
            for (k, tb) in d.turb.iter().enumerate() {
                let sp = tb.smin + (tb.smax - tb.smin) * hash01(p.seed, 100 + k as u32);
                let ph = hash01(p.seed, 200 + k as u32) * TAU; let tt = self.t * tb.ts + ph; let s = tb.scale;
                let nx = (p.pos[1] * s + tt).sin() + (p.pos[0] * s * 1.7 - tt * 0.7).cos() * 0.5;
                let ny = (p.pos[0] * s - tt * 0.9).cos() + (p.pos[1] * s * 1.3 + tt * 0.6).sin() * 0.5;
                p.vel[0] += nx * sp * tb.mask[0] * dt; p.vel[1] += ny * sp * tb.mask[1] * dt;
            }
            let k = (1.0 - drag * dt).max(0.0);
            for a in 0..3 { p.vel[a] = (p.vel[a] + grav[a] * dt) * k; p.pos[a] += p.vel[a] * dt; }
            p.angv += angf * dt; p.rot += p.angv * dt;
            true
        });
        let room = d.maxcount.saturating_sub(self.ps.len());
        let mut left = room;
        for ei in 0..d.emitters.len() {
            d.emitters[ei].acc += d.emitters[ei].rate * dt;
            while d.emitters[ei].acc >= 1.0 && left > 0 { d.emitters[ei].acc -= 1.0; left -= 1; let p = spawn(d, ei, &mut self.rng, &mut self.counter); self.ps.push(p); }
            if left == 0 { d.emitters[ei].acc = d.emitters[ei].acc.min(1.0); }
        }
        self.t += dt;
    }

    fn init(&mut self) {
        // instantaneous bursts at t=0, then the file's starttime prewarm
        for ei in 0..self.def.emitters.len() {
            let n = self.def.emitters[ei].inst.round() as usize;
            for _ in 0..n { if self.ps.len() >= self.def.maxcount { break; } let p = spawn(&mut self.def, ei, &mut self.rng, &mut self.counter); self.ps.push(p); }
        }
        let n = (self.def.starttime / STEP) as usize;
        for _ in 0..n { self.step(STEP); }
        self.t = 0.0;
    }

    fn fill(&mut self) -> usize {
        let d = &self.def;
        self.buf.clear();
        for p in &self.ps {
            let f = (p.age / p.life).clamp(0.0, 1.0);
            let mut a = p.alpha;
            if d.fade.0 > 0.0 && f < d.fade.0 { a *= f / d.fade.0; }
            if d.fade.1 < 1.0 && f > d.fade.1 { a *= ((1.0 - f) / (1.0 - d.fade.1).max(1e-4)).max(0.0); }
            let mut sz = p.size; let mut col = p.col; let mut pos = p.pos;
            for r in &d.sizech { sz *= r.at(f); }
            for r in &d.alphach { a *= r.at(f); }
            for (c, t0, t1) in &d.colch { let k = ((f - t0) / (t1 - t0).max(1e-4)).clamp(0.0, 1.0); for x in 0..3 { col[x] *= c[0][x] + (c[1][x] - c[0][x]) * k; } }
            let osc = |o: &Osc, ix: u32| -> f32 { let fr = o.fmin + (o.fmax - o.fmin) * hash01(p.seed, ix); let ph = o.pmin + (o.pmax - o.pmin) * hash01(p.seed, ix + 7); let s = 0.5 + 0.5 * (p.age * fr * TAU + ph).sin(); o.smin + (o.smax - o.smin) * s };
            for (k, o) in d.osc_a.iter().enumerate() { a *= osc(o, 300 + k as u32 * 11); }
            for (k, o) in d.osc_s.iter().enumerate() { sz *= osc(o, 400 + k as u32 * 11); }
            for (k, o) in d.osc_p.iter().enumerate() { let fr = o.fmin + (o.fmax - o.fmin) * hash01(p.seed, 500 + k as u32); let ph = o.pmin + (o.pmax - o.pmin) * hash01(p.seed, 507 + k as u32);
                let sc = o.smin.max(0.0) + (o.smax - o.smin.max(0.0)) * hash01(p.seed, 515 + k as u32); let w = (p.age * fr * TAU + ph).sin() * sc; for x in 0..3 { pos[x] += w * o.mask[x]; } }
            if a <= 0.001 || sz <= 0.0 { continue; }
            let fr = if self.frames.len() > 1 {
                let n = self.frames.len(); let idx = if d.seq { ((f * d.seqmul).fract() * n as f32) as usize } else { (p.rnd * n as f32) as usize }; self.frames[idx.min(n - 1)]
            } else { self.full };
            self.buf.extend_from_slice(&[pos[0], pos[1], sz, p.rot, col[0], col[1], col[2], a.min(1.0), fr[0], fr[1], fr[2], fr[3]]);
        }
        self.buf.len() / 12
    }
}

// ---------------- public set ----------------
#[derive(Default)]
pub struct Set { seen: std::collections::HashSet<String>, sys: Vec<Sys>, prog: u32, u_mvp: i32, u_ratio: i32, u_over: i32, last: Option<f32>, white: u32, cache: HashMap<String, Option<(u32, Vec<[f32; 4]>, [f32; 4], f32)>> }

fn gl_swizzle(id: u32, fmt: Fmt) {
    let sw: Option<[i32; 4]> = match fmt { Fmt::R8 => Some([gl::ONE as i32, gl::ONE as i32, gl::ONE as i32, gl::RED as i32]), Fmt::Rg88 => Some([gl::RED as i32, gl::RED as i32, gl::RED as i32, gl::GREEN as i32]), _ => None };
    if let Some(s) = sw { unsafe { gl::BindTexture(gl::TEXTURE_2D, id); gl::TexParameteriv(gl::TEXTURE_2D, 0x8E46, s.as_ptr()); } }
}

impl Set {
    pub fn is_empty(&self) -> bool { self.sys.is_empty() }
    pub fn clear(&mut self) { self.sys.clear(); self.last = None; }

    fn white_tex(&mut self) -> u32 {
        if self.white == 0 { unsafe {
            gl::GenTextures(1, &mut self.white); gl::BindTexture(gl::TEXTURE_2D, self.white);
            let px = [255u8; 4]; gl::TexImage2D(gl::TEXTURE_2D, 0, gl::RGBA8 as i32, 1, 1, 0, gl::RGBA, gl::UNSIGNED_BYTE, px.as_ptr() as *const _);
        } }
        self.white
    }

    /// Texture, per-frame uv rects (sheet) and the whole-image uv rect, height/width ratio of one frame.
    fn texture(&mut self, vfs: &Vfs, name: &str, s3tc: bool) -> Option<(u32, Vec<[f32; 4]>, [f32; 4], f32)> {
        if let Some(c) = self.cache.get(name) { return c.clone(); }
        let path = scene::tex_file(name);
        let r = (|| {
            let t = tex::parse(&vfs.read(&path)?, false).ok()?;
            let meta = vfs.read_json(&path.replace(".tex", ".tex-json")).map(|j| tex::parse_meta(&j)).unwrap_or_default();
            let mut frames: Vec<tex::SpriteFrame> = t.frames.clone();
            if frames.len() < 2 {
                if let Some(s) = vfs.read_json(&path.replace(".tex", ".tex-json")).and_then(|j| j.get("spritesheetsequences").and_then(|v| v.as_array()).and_then(|a| a.first().cloned())) {
                    let (n, w, h) = (num(&s, "frames", 1.0) as usize, num(&s, "width", 0.0), num(&s, "height", 0.0));
                    if n > 1 && w >= 1.0 && h >= 1.0 { let per = (t.tex_w as f32 / w + 0.05).floor().max(1.0) as usize; frames = (0..n.min(4096)).map(|i| tex::SpriteFrame { x: (i % per) as f32 * w, y: (i / per) as f32 * h, w, h, dur: 0.1 }).collect(); }
                }
            }
            if std::env::var("WF_DBG").is_ok() { eprintln!("particles: tex {path} {}x{} img {}x{} frames {:?}", t.tex_w, t.tex_h, t.img_w, t.img_h, frames); }
            let gt = tex::upload(&t, 256, 256, &meta, s3tc).ok()?;
            if let Payload::Raw { fmt, .. } = &t.payload { gl_swizzle(gt.id, *fmt); }
            unsafe { gl::BindTexture(gl::TEXTURE_2D, gt.id); for k in [gl::TEXTURE_WRAP_S, gl::TEXTURE_WRAP_T] { gl::TexParameteri(gl::TEXTURE_2D, k, gl::CLAMP_TO_EDGE as i32); } }
            let (tw, th) = (t.tex_w as f32, t.tex_h as f32);
            let uvs: Vec<[f32; 4]> = if frames.len() > 1 { frames.iter().map(|f| [f.x / tw, f.y / th, f.w / tw, f.h / th]).collect() } else { vec![] };
            let ratio = if frames.len() > 1 { frames[0].h / frames[0].w } else { t.img_h.max(1) as f32 / t.img_w.max(1) as f32 };
            Some((gt.id, uvs, [0.0, 0.0, gt.uv[0], gt.uv[1]], ratio))
        })();
        self.cache.insert(name.to_string(), r.clone());
        r
    }

    /// Adds the particle system of scene object `world` (object matrix) drawn after `at` image layers.
    pub fn add(&mut self, vfs: &Vfs, file: &str, ovr: &[(String, Vec<f32>)], world: Mat4, at: usize, s3tc: bool, seed: u64) {
        if DISABLED.load(Ordering::Relaxed) { return; }
        let g = |k: &str| ovr.iter().find(|o| o.0 == k).map(|o| o.1.clone());
        let g1 = |k: &str| g(k).and_then(|v| v.first().copied());
        let mut color = g("colorn").or_else(|| g("color")).filter(|v| v.len() >= 3).map(|v| [v[0], v[1], v[2]]).unwrap_or([1.0; 3]);
        if color.iter().any(|&c| c > 1.5) { color = color.map(|c| c / 255.0); }
        let br = g1("brightness").unwrap_or(1.0); color = color.map(|c| c * br);
        let ov = Ov { count: g1("count").unwrap_or(1.0).max(0.0), rate: g1("rate").unwrap_or(1.0).max(0.0), size: g1("size").unwrap_or(1.0), speed: g1("speed").unwrap_or(1.0), life: g1("lifetime").unwrap_or(1.0).max(0.01), alpha: g1("alpha").unwrap_or(1.0), color };
        let Some(j) = vfs.read_json(file) else { eprintln!("wallforge-render: note: particle {file} not found, skipped"); return };
        let mut notes = vec![];
        let def = parse_def(&j, vfs, &ov, &mut notes);
        if !notes.is_empty() && self.seen.insert(file.to_string()) { notes.dedup(); eprintln!("wallforge-render: note: particle {file}: ignoring {}", notes.join(", ")); }
        let Some(def) = def else { if notes.is_empty() { eprintln!("wallforge-render: note: particle {file}: no usable material, skipped"); } return };
        if self.prog == 0 {
            match crate::engine::link(VS, FS) {
                Ok(p) => unsafe { self.prog = p; self.u_mvp = gl::GetUniformLocation(p, c"mvp".as_ptr()); self.u_ratio = gl::GetUniformLocation(p, c"ratio".as_ptr()); self.u_over = gl::GetUniformLocation(p, c"overbright".as_ptr()); },
                Err(e) => { eprintln!("wallforge-render: note: particle shader: {e}; particles disabled"); return }
            }
        }
        let (texid, frames, full, ratio) = if def.texture.is_empty() { (self.white_tex(), vec![], [0.0, 0.0, 1.0, 1.0], 1.0) } else {
            match self.texture(vfs, &def.texture, s3tc) { Some(t) => t, None => { eprintln!("wallforge-render: note: particle {file}: texture {} not found, skipped", def.texture); return } }
        };
        let (mut vao, mut vbo) = (0, 0);
        unsafe {
            gl::GenVertexArrays(1, &mut vao); gl::BindVertexArray(vao);
            gl::GenBuffers(1, &mut vbo); gl::BindBuffer(gl::ARRAY_BUFFER, vbo);
            for i in 0..3u32 { gl::EnableVertexAttribArray(i); gl::VertexAttribPointer(i, 4, gl::FLOAT, gl::FALSE, 48, (i as usize * 16) as *const _); gl::VertexAttribDivisor(i, 1); }
            gl::BindVertexArray(0);
        }
        let mut s = Sys { def, ps: vec![], rng: Rng(0x9E3779B97F4A7C15 ^ seed.wrapping_mul(0xD1B54A32D192ED03) | 1), t: 0.0, at, world, ratio, frames, full, tex: texid, vao, vbo, cap_bytes: 0, buf: vec![], counter: 0 };
        s.init();
        self.sys.push(s);
    }

    /// Advance all systems to scene time `t` (fixed steps; first call simulates from 0).
    pub fn step(&mut self, t: f32) {
        if self.sys.is_empty() { return; }
        let last = self.last.unwrap_or(0.0).max(t - MAX_CATCHUP);
        if t > last {
            let n = ((t - last) / STEP).floor() as usize;
            for s in &mut self.sys { for _ in 0..n { s.step(STEP); } }
            self.last = Some(last + n as f32 * STEP);
        } else if t < last - 1.0 { self.last = Some(t); }
    }

    /// Draw the systems placed after `at` image layers. Caller restores its own VAO/program state.
    pub fn draw(&mut self, at: usize, view: &Mat4, fbo: u32, out: (i32, i32)) {
        if !self.sys.iter().any(|s| s.at == at) { return; }
        unsafe { gl::BindFramebuffer(gl::FRAMEBUFFER, fbo); gl::Viewport(0, 0, out.0, out.1); gl::Enable(gl::BLEND); gl::UseProgram(self.prog); gl::ActiveTexture(gl::TEXTURE0); }
        for s in self.sys.iter_mut().filter(|s| s.at == at) {
            let n = s.fill();
            if std::env::var("WF_DBG").is_ok() { eprintln!("particles: system at layer {} alive {} drawn {} max {} tex {} ratio {:.2} frames {}", s.at, s.ps.len(), n, s.def.maxcount, s.tex, s.ratio, s.frames.len()); }
            if std::env::var("WF_DBG").map(|v| v == "2").unwrap_or(false) { for k in 0..n.min(3) { eprintln!("  p{k}: {:?}", &s.buf[k * 12..k * 12 + 12]); } eprintln!("  world {:?}", &s.world[..]); }
            if n == 0 { continue; }
            let mvp = scene::mul(view, &s.world);
            unsafe {
                match s.def.blend { Blend::Add => gl::BlendFunc(gl::SRC_ALPHA, gl::ONE), Blend::Alpha => gl::BlendFuncSeparate(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA, gl::ONE, gl::ONE_MINUS_SRC_ALPHA) }
                gl::UniformMatrix4fv(self.u_mvp, 1, gl::FALSE, mvp.as_ptr()); gl::Uniform1f(self.u_ratio, s.ratio); gl::Uniform1f(self.u_over, s.def.overbright);
                gl::BindTexture(gl::TEXTURE_2D, s.tex);
                gl::BindVertexArray(s.vao); gl::BindBuffer(gl::ARRAY_BUFFER, s.vbo);
                let bytes = s.buf.len() * 4;
                if bytes > s.cap_bytes { gl::BufferData(gl::ARRAY_BUFFER, bytes as isize, s.buf.as_ptr() as *const _, gl::STREAM_DRAW); s.cap_bytes = bytes; }
                else { gl::BufferSubData(gl::ARRAY_BUFFER, 0, bytes as isize, s.buf.as_ptr() as *const _); }
                gl::DrawArraysInstanced(gl::TRIANGLE_STRIP, 0, 4, n as i32);
                gl::BindVertexArray(0);
            }
        }
        unsafe { gl::Disable(gl::BLEND); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rng_is_deterministic_and_in_range() {
        let (mut a, mut b) = (Rng(12345), Rng(12345));
        for _ in 0..100 { let x = a.next(); assert_eq!(x, b.next()); assert!((0.0..1.0).contains(&x)); }
    }
    #[test]
    fn ramp_interpolates() {
        let r = Ramp { s: 0.0, e: 1.0, t0: 0.0, t1: 1.0 };
        assert!((r.at(0.5) - 0.5).abs() < 1e-6 && r.at(2.0) == 1.0);
    }
}
