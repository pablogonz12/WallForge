//! Wallpaper Engine shader dialect (GLSL 1.20-style .vert/.frag with `#include`, `// {"material":..}` annotations,
//! `[COMBO]` comments and HLSL-ish helper macros) -> GLSL ES 3.00.
//! Known limit: the dialect relies on desktop-GLSL implicit int->float conversions in places; ES 3.00 is strict, so a
//! shader using them fails to compile (reported per effect, the effect is then skipped).
use serde_json::{Map, Value};
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct Uniform { pub name: String, pub ty: String, pub material: Option<String>, pub default: Option<Value> }

#[derive(Debug)]
pub struct Translated { pub vert: String, pub frag: String, pub uniforms: Vec<Uniform>, pub uses_time: bool, pub combos: BTreeMap<String, String> }

const PRE: &str = "#version 300 es
precision highp float;
precision highp int;
precision highp sampler2D;
#define attribute in
#define mul(x, y) ((y) * (x))
#define saturate(x) clamp(x, 0.0, 1.0)
#define frac(x) fract(x)
#define lerp(a, b, t) mix(a, b, t)
#define atan2(y, x) atan(y, x)
#define fmod(x, y) ((x) - (y) * floor((x) / (y)))
#define ddx(x) dFdx(x)
#define ddy(x) dFdy(x)
#define CAST2(x) (vec2(x))
#define CAST3(x) (vec3(x))
#define CAST4(x) (vec4(x))
#define CAST3X3(x) (mat3(x))
#define sample wf_sample
#define input wf_input
#define output wf_output
#define filter wf_filter
#define common wf_common
#define active wf_active
#define partition wf_partition
#define texture2D wf_tex
#define texSample2D wf_tex
#define texSample2DLod textureLod
#define texture2DLod textureLod
float wf_max(float a, float b) { return max(a, b); }
vec2 wf_max(vec2 a, vec2 b) { return max(a, b); }
vec3 wf_max(vec3 a, vec3 b) { return max(a, b); }
vec4 wf_max(vec4 a, vec4 b) { return max(a, b); }
vec2 wf_max(vec2 a, float b) { return max(a, vec2(b)); }
vec2 wf_max(float a, vec2 b) { return max(vec2(a), b); }
vec3 wf_max(vec3 a, float b) { return max(a, vec3(b)); }
vec3 wf_max(float a, vec3 b) { return max(vec3(a), b); }
vec4 wf_max(vec4 a, float b) { return max(a, vec4(b)); }
vec4 wf_max(float a, vec4 b) { return max(vec4(a), b); }
#define max wf_max
float wf_min(float a, float b) { return min(a, b); }
vec2 wf_min(vec2 a, vec2 b) { return min(a, b); }
vec3 wf_min(vec3 a, vec3 b) { return min(a, b); }
vec4 wf_min(vec4 a, vec4 b) { return min(a, b); }
vec2 wf_min(vec2 a, float b) { return min(a, vec2(b)); }
vec2 wf_min(float a, vec2 b) { return min(vec2(a), b); }
vec3 wf_min(vec3 a, float b) { return min(a, vec3(b)); }
vec3 wf_min(float a, vec3 b) { return min(vec3(a), b); }
vec4 wf_min(vec4 a, float b) { return min(a, vec4(b)); }
vec4 wf_min(float a, vec4 b) { return min(vec4(a), b); }
#define min wf_min
float wf_pow(float a, float b) { return pow(a, b); }
vec2 wf_pow(vec2 a, vec2 b) { return pow(a, b); }
vec3 wf_pow(vec3 a, vec3 b) { return pow(a, b); }
vec4 wf_pow(vec4 a, vec4 b) { return pow(a, b); }
vec2 wf_pow(vec2 a, float b) { return pow(a, vec2(b)); }
vec3 wf_pow(vec3 a, float b) { return pow(a, vec3(b)); }
vec4 wf_pow(vec4 a, float b) { return pow(a, vec4(b)); }
#define pow wf_pow
float wf_mix(float a, float b, float t) { return mix(a, b, t); }
vec2 wf_mix(vec2 a, vec2 b, vec2 t) { return mix(a, b, t); }
vec3 wf_mix(vec3 a, vec3 b, vec3 t) { return mix(a, b, t); }
vec4 wf_mix(vec4 a, vec4 b, vec4 t) { return mix(a, b, t); }
vec2 wf_mix(vec2 a, vec2 b, float t) { return mix(a, b, t); }
vec3 wf_mix(vec3 a, vec3 b, float t) { return mix(a, b, t); }
vec4 wf_mix(vec4 a, vec4 b, float t) { return mix(a, b, t); }
vec3 wf_mix(vec4 a, vec3 b, float t) { return mix(a.rgb, b, t); }
vec2 wf_mix(vec2 a, vec2 b, vec3 t) { return mix(a, b, t.xy); }
vec2 wf_mix(vec2 a, vec2 b, vec4 t) { return mix(a, b, t.xy); }
vec3 wf_mix(vec3 a, vec3 b, vec4 t) { return mix(a, b, t.xyz); }
vec3 wf_mix(vec3 a, vec4 b, float t) { return mix(a, b.rgb, t); }
vec3 wf_mix(vec3 a, vec4 b, vec3 t) { return mix(a, b.rgb, t); }
#define mix wf_mix
float wf_mod(float a, float b) { return a - b * trunc(a / b); }
vec2 wf_mod(vec2 a, vec2 b) { return a - b * trunc(a / b); }
vec3 wf_mod(vec3 a, vec3 b) { return a - b * trunc(a / b); }
vec4 wf_mod(vec4 a, vec4 b) { return a - b * trunc(a / b); }
vec2 wf_mod(vec2 a, float b) { return a - b * trunc(a / b); }
vec3 wf_mod(vec3 a, float b) { return a - b * trunc(a / b); }
vec4 wf_mod(vec4 a, float b) { return a - b * trunc(a / b); }
float wf_mod(float a, int b) { return wf_mod(a, float(b)); }
float wf_mod(float a, uint b) { return wf_mod(a, float(b)); }
float wf_mod(int a, float b) { return wf_mod(float(a), b); }
int wf_mod(int a, int b) { return a % b; }
uint wf_mod(uint a, uint b) { return a % b; }
vec4 wf_tex(sampler2D s, vec2 c) { return texture(s, c); }
vec4 wf_tex(sampler2D s, vec4 c) { return texture(s, c.xy); }
";

fn expand(read: &dyn Fn(&str) -> Option<String>, src: &str, depth: u32, out: &mut String) -> Result<(), String> {
    if depth > 8 { return Err("#include nesting too deep".into()); }
    for line in src.lines() {
        let t = line.trim_start();
        if let Some(rest) = t.strip_prefix("#include") {
            let name = rest.trim().trim_matches(|c| c == '"' || c == '<' || c == '>');
            let inc = read(&format!("shaders/{name}")).ok_or_else(|| format!("missing include '{name}'"))?;
            expand(read, &inc, depth + 1, out)?;
        } else if t.starts_with("#require") {
            // Wallpaper Engine module includes (LightingV1, ImageProcessor, ...) come from its own shader library.
            // We do not have those modules; drop the directive and let the compiler tell us if a symbol is missing.
            out.push_str("// #require dropped\n");
        } else { out.push_str(line); out.push('\n'); }
    }
    Ok(())
}

fn json_tail(line: &str) -> Option<Value> {
    let i = line.find("//")?;
    let c = line[i + 2..].trim();
    let c = c.strip_prefix("[COMBO]").unwrap_or(c).trim();
    serde_json::from_str(c).ok()
}

fn val_str(v: &Value) -> String {
    match v { Value::Bool(b) => (*b as i32).to_string(), Value::Number(n) => n.to_string(), Value::String(s) => s.clone(), o => o.to_string() }
}

/// Opaque mask slots gated by a combo: `g_TextureN` annotated `"mode":"opacitymask"` with a `require` (blurprecise's
/// `ENABLEMASK`). Wallpaper Engine only offers such a property while its gate is on, and an unpainted mask is black
/// (paintdefaultcolor `0 0 0 1`), so a pass that turns the gate on without supplying a mask behaves as "mask = 0".
pub fn mask_slots(read: &dyn Fn(&str) -> Option<String>, shader: &str) -> Vec<(usize, Map<String, Value>)> {
    let mut out = vec![];
    let Some(raw) = read(&format!("shaders/{shader}.frag")) else { return out };
    let mut src = String::new();
    if expand(read, &raw, 0, &mut src).is_err() { return out; }
    for line in src.lines() {
        let t = line.trim_start();
        let Some(rest) = t.strip_prefix("uniform ") else { continue };
        let Some(j) = json_tail(t) else { continue };
        if j.get("mode").and_then(|m| m.as_str()) != Some("opacitymask") { continue; }
        let Some(req) = j.get("require").and_then(|r| r.as_object()) else { continue; };
        let decl = rest.split("//").next().unwrap_or("").trim().trim_end_matches(';').trim();
        let name = decl.split_whitespace().nth(1).unwrap_or("").split('[').next().unwrap_or("");
        let Ok(slot) = name.strip_prefix("g_Texture").unwrap_or("").parse::<usize>() else { continue };
        out.push((slot, req.clone()));
    }
    out
}

fn stage(read: &dyn Fn(&str) -> Option<String>, path: &str, is_frag: bool, over: &Map<String, Value>, uniforms: &mut Vec<Uniform>,
         combos: &mut BTreeMap<String, String>, present: &[usize], floatify: bool) -> Result<String, String> {
    let raw = read(path).ok_or_else(|| format!("missing shader {path}"))?;
    let mut src = String::new();
    expand(read, &raw, 0, &mut src)?;
    for l in src.lines() {
        let t = l.trim_start();
        if t.contains("[COMBO]") {
            if let Some(j) = json_tail(t) {
                if let Some(n) = j.get("combo").and_then(|c| c.as_str()) {
                    let d = j.get("default").map(val_str).unwrap_or_else(|| "0".into());
                    combos.entry(n.to_string()).or_insert(d);
                }
            }
        } else if let Some(r) = t.strip_prefix("uniform ") {
            let decl = r.split("//").next().unwrap_or("").trim().trim_end_matches(';').trim();
            let mut it = decl.split_whitespace();
            let (Some(ty), Some(name)) = (it.next(), it.next()) else { continue };
            let name = name.split('[').next().unwrap_or(name).to_string();
            let j = json_tail(t);
            // sampler annotation {"combo":"X"}: X is 1 when a texture is bound to that slot
            if let (Some(cn), Some(slot)) = (j.as_ref().and_then(|j| j.get("combo")).and_then(|c| c.as_str()), name.strip_prefix("g_Texture").and_then(|n| n.parse::<usize>().ok())) {
                combos.insert(cn.to_string(), if present.contains(&slot) { "1".into() } else { "0".into() });
            }
            let u = Uniform { name: name.clone(), ty: ty.to_string(),
                material: j.as_ref().and_then(|j| j.get("material")).and_then(|m| m.as_str()).map(String::from),
                default: j.as_ref().and_then(|j| j.get("default")).cloned() };
            if !uniforms.iter().any(|x| x.name == name) { uniforms.push(u); }
        }
    }
    let mut o = String::from(PRE);
    o.push_str(if is_frag { "#define varying in\nout vec4 wf_FragColor;\n#define gl_FragColor wf_FragColor\n" } else { "#define varying out\n" });
    // combos: defaults found in this stage, overridden by the material/effect values
    let mut all = combos.clone();
    for (k, v) in over { all.insert(k.clone(), val_str(v)); }
    // C preprocessor semantics: unknown identifiers in #if/#elif evaluate to 0 (GLSL ES makes them an error)
    for k in undefined_in_conditions(&src) { if !all.contains_key(&k) { all.insert(k, "0".into()); } }
    for (k, v) in &all { o.push_str(&format!("#define {k} {v}\n")); }
    let src = hoist_uniforms(&src);
    o.push_str(&if floatify { floatify_src(&src) } else { src });
    Ok(o)
}

/// Moves unconditional single-line `uniform` declarations to the top (WE's compiler accepts use-before-declaration).
pub fn hoist_uniforms(src: &str) -> String {
    let (mut head, mut body) = (String::new(), String::new());
    let mut depth = 0;
    for line in src.lines() {
        let t = line.trim_start();
        if t.starts_with("#if") { depth += 1; } else if t.starts_with("#endif") { depth -= 1; }
        let dst = if depth == 0 && t.starts_with("uniform ") && t.contains(';') { &mut head } else { &mut body };
        dst.push_str(line); dst.push('\n');
    }
    head.push_str(&body);
    head
}

/// Identifiers used in `#if`/`#elif` expressions that are neither defined in the source nor keywords.
pub fn undefined_in_conditions(src: &str) -> Vec<String> {
    let mut defined: Vec<String> = vec![];
    for l in src.lines() {
        let t = l.trim_start();
        if let Some(r) = t.strip_prefix('#') { let r = r.trim_start(); if let Some(d) = r.strip_prefix("define") { if let Some(n) = d.split_whitespace().next() { defined.push(n.split('(').next().unwrap_or(n).to_string()); } } }
    }
    let mut out: Vec<String> = vec![];
    for l in src.lines() {
        let t = l.trim_start();
        let Some(r) = t.strip_prefix('#') else { continue };
        let r = r.trim_start();
        let expr = if let Some(e) = r.strip_prefix("if ").or_else(|| r.strip_prefix("if(")) { e } else if let Some(e) = r.strip_prefix("elif") { e } else { continue };
        let expr = expr.split("//").next().unwrap_or("");
        let b = expr.as_bytes();
        let mut i = 0;
        let mut after_defined = false;
        while i < b.len() {
            if b[i].is_ascii_alphabetic() || b[i] == b'_' {
                let st = i;
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') { i += 1; }
                let id = &expr[st..i];
                if id == "defined" { after_defined = true; continue; }
                if after_defined { after_defined = false; continue; }
                if !defined.iter().any(|d| d == id) && !out.iter().any(|d| d == id) { out.push(id.to_string()); }
            } else if b[i].is_ascii_digit() {
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'.') { i += 1; }
            } else { if !(b[i] == b'(' || b[i] == b' ') { after_defined = false; } i += 1; }
        }
    }
    out
}

/// Desktop GLSL converts int -> float implicitly (`2 * x`, `smoothstep(0, 1, x)`); ES 3.00 does not. Used as a retry for
/// shaders that fail to compile: integer literals outside integer-looking contexts become float literals.
pub fn floatify_src(src: &str) -> String {
    let mut out = String::with_capacity(src.len() + 64);
    // `const int N = ..;` constants used in float arithmetic (`N - 1.0`) are wrapped in float(..)
    let mut int_consts: Vec<String> = vec![];
    for l in src.lines() {
        if let Some(r) = l.trim_start().strip_prefix("const int ") { if let Some(n) = r.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).next() { if !n.is_empty() { int_consts.push(n.to_string()); } } }
    }
    for l in src.lines() {
        if let Some(r) = l.trim_start().strip_prefix("for") { if let Some(r) = r.trim_start().strip_prefix('(').map(|r| r.trim_start()).and_then(|r| r.strip_prefix("int ")) {
            if let Some(n) = r.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).next() { if !n.is_empty() { int_consts.push(n.to_string()); } }
        } }
    }
    for line in src.lines() {
        let t = line.trim_start();
        // `float n = int(x);` is an error in GLSL ES; WE meant an int (it is later used as a loop bound)
        let fixed;
        let line = { let tl = line.trim_start(); if tl.starts_with("float ") && tl.contains("= int(") && !tl.contains(',') { fixed = line.replacen("float ", "int ", 1); fixed.as_str() } else { line } };
        let intish = t.starts_with('#') || has_word(line, "int") || line.contains("ivec") || line.contains("uint") || line.contains("switch") || line.contains("case ");
        if intish { out.push_str(line); out.push('\n'); continue; }
        let b = line.as_bytes();
        let mut i = 0;
        let mut bracket = 0i32;
        let code_end = line.find("//").unwrap_or(line.len());
        while i < b.len() {
            let c = b[i];
            if i >= code_end { out.push(c as char); i += 1; continue; }
            if c == b'[' { bracket += 1; } else if c == b']' { bracket -= 1; }
            if c.is_ascii_alphabetic() || c == b'_' {
                let st = i;
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') { i += 1; }
                let id = &line[st..i];
                let after = line[i..].trim_start();
                let prev_dot = st > 0 && b[st - 1] == b'.';
                if bracket == 0 && !prev_dot && !t.starts_with("for") && !line.contains("for (") && !after.starts_with("++") && !after.starts_with("--") && !after.starts_with("+=") && !after.starts_with("=") && !after.starts_with("<") && !after.starts_with(">") && int_consts.iter().any(|n| n == id) { out.push_str(&format!("float({id})")); } else { out.push_str(id); }
            } else if c.is_ascii_digit() {
                let st = i;
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'.') { i += 1; }
                if i < b.len() && (b[i] == b'-' || b[i] == b'+') && line[st..i].ends_with(|c| c == 'e' || c == 'E') && line[st..i].starts_with(|c: char| c.is_ascii_digit()) {
                    i += 1;
                    while i < b.len() && b[i].is_ascii_digit() { i += 1; }
                }
                let tok = &line[st..i];
                let prev = if st > 0 { b[st - 1] } else { b' ' };
                if tok.bytes().all(|d| d.is_ascii_digit()) && bracket == 0 && prev != b'.' { out.push_str(tok); out.push_str(".0"); } else { out.push_str(tok); }
            } else { out.push(c as char); i += 1; }
        }
        out.push('\n');
    }
    out
}

pub fn translate(read: &dyn Fn(&str) -> Option<String>, shader: &str, combo_over: &Map<String, Value>, present: &[usize], floatify: bool) -> Result<Translated, String> {
    let mut uniforms = vec![];
    let mut combos = BTreeMap::new();
    // two passes so the vertex stage also sees combos declared in the fragment stage and vice versa
    let _ = stage(read, &format!("shaders/{shader}.vert"), false, combo_over, &mut uniforms, &mut combos, present, floatify)?;
    let _ = stage(read, &format!("shaders/{shader}.frag"), true, combo_over, &mut uniforms, &mut combos, present, floatify)?;
    let vert = stage(read, &format!("shaders/{shader}.vert"), false, combo_over, &mut uniforms, &mut combos, present, floatify)?;
    let frag = stage(read, &format!("shaders/{shader}.frag"), true, combo_over, &mut uniforms, &mut combos, present, floatify)?;
    let uses_time = vert.contains("g_Time") || frag.contains("g_Time");
    Ok(Translated { vert, frag, uniforms, uses_time, combos })
}

fn ty_dim(t: &str) -> Option<usize> { Some(match t { "float" | "int" | "uint" | "bool" => 1, "vec2" | "ivec2" | "uvec2" => 2, "vec3" | "ivec3" | "uvec3" => 3, "vec4" | "ivec4" | "uvec4" => 4, _ => return None }) }
fn is_id(c: u8) -> bool { c.is_ascii_alphanumeric() || c == b'_' }

/// Index just past the bracket group opening at `i` (`(` or `[`), or None when unbalanced.
fn match_close(b: &[u8], i: usize) -> Option<usize> {
    let mut d = 0i32;
    for j in i..b.len() { match b[j] { b'(' | b'[' => d += 1, b')' | b']' => { d -= 1; if d == 0 { return Some(j + 1); } } _ => {} } }
    None
}
/// Start index of the operand ending just before `end` (identifier/number chain with calls, indexing and swizzles).
fn operand_start(b: &[u8], end: usize) -> usize {
    let mut i = end;
    while i > 0 && b[i - 1] == b' ' { i -= 1; }
    while i > 0 {
        let c = b[i - 1];
        if c == b')' || c == b']' {
            let mut d = 0i32; let mut j = i;
            while j > 0 { j -= 1; match b[j] { b')' | b']' => d += 1, b'(' | b'[' => { d -= 1; if d == 0 { break; } } _ => {} } }
            i = j;
        } else if is_id(c) || c == b'.' { while i > 0 && (is_id(b[i - 1]) || b[i - 1] == b'.') { i -= 1; } } else { break; }
        if i > 0 && (b[i - 1] == b')' || b[i - 1] == b']' || is_id(b[i - 1]) || b[i - 1] == b'.') { continue; }
        break;
    }
    i
}
/// HLSL `%` is fmod on floats; GLSL only allows ints. `a * b % c` -> `wf_mod(a * b, c)` (the multiplicative chain is the left operand).
fn fix_mod(line: &str) -> String {
    let mut s = line.to_string();
    loop {
        let b = s.as_bytes();
        let code = s.find("//").unwrap_or(s.len());
        let Some(p) = (0..code).rev().find(|&p| b[p] == b'%' && b.get(p + 1) != Some(&b'=')) else { return s };
        let mut st = operand_start(b, p);
        loop {
            let mut k = st; while k > 0 && b[k - 1] == b' ' { k -= 1; }
            if k > 0 && (b[k - 1] == b'*' || b[k - 1] == b'/') { st = operand_start(b, k - 1); } else { break; }
        }
        let mut r = p + 1; while r < b.len() && b[r] == b' ' { r += 1; }
        let r0 = r; if r < b.len() && (b[r] == b'-' || b[r] == b'+') { r += 1; }
        while r < b.len() && (is_id(b[r]) || b[r] == b'.') { r += 1; }
        while r < b.len() && (b[r] == b'(' || b[r] == b'[') { match match_close(b, r) { Some(e) => r = e, None => return s } while r < b.len() && (is_id(b[r]) || b[r] == b'.') { r += 1; } }
        if r == r0 { return s; }
        s = format!("{}wf_mod({}, {}){}", &s[..st], s[st..p].trim(), s[r0..r].trim(), &s[r..]);
        // the rewritten call hides its own `%`-free text; continue with any earlier `%`
        if !s[..st].contains('%') { return s; }
    }
}
/// `a[f]` -> `a[int(f)]` (HLSL allows float indices).
fn fix_index(line: &str) -> String {
    let b = line.as_bytes();
    let (mut out, mut i) = (String::new(), 0);
    let code = line.find("//").unwrap_or(line.len());
    while i < b.len() {
        if b[i] == b'[' && i < code && i > 0 && (is_id(b[i - 1]) || b[i - 1] == b']') {
            if let Some(e) = match_close(b, i) {
                let inner = &line[i + 1..e - 1];
                if inner.bytes().all(|c| c.is_ascii_digit()) || inner.is_empty() { out.push_str(&line[i..e]); } else { out.push_str(&format!("[int({})]", inner)); }
                i = e; continue;
            }
        }
        out.push(b[i] as char); i += 1;
    }
    out
}
/// Wraps the right-hand side of the first assignment/initializer on the line in `to(..)` (HLSL converts and truncates implicitly).
fn fix_assign(line: &str, from: &str, to: &str) -> String {
    let b = line.as_bytes();
    let code = line.find("//").unwrap_or(line.len());
    let Some(p) = (0..code).find(|&p| b[p] == b'=' && b.get(p + 1) != Some(&b'=') && (p == 0 || !matches!(b[p - 1], b'=' | b'<' | b'>' | b'!'))) else { return line.to_string() };
    let s = p + 1;
    let (mut d, mut e) = (0i32, code);
    for j in s..code { match b[j] { b'(' | b'[' => d += 1, b')' | b']' => { if d == 0 { e = j; break; } d -= 1; } b';' | b',' if d == 0 => { e = j; break; } _ => {} } }
    let rhs = line[s..e].trim();
    let (fd, td) = (ty_dim(from).unwrap_or(1), ty_dim(to).unwrap_or(1));
    let w = if td > fd && fd > 1 { format!("{to}({rhs}{})", ", 0.0".repeat(td - fd)) } else { format!("{to}({rhs})") };
    format!("{} {}{}", &line[..s], w, &line[e..])
}
/// Wraps integer-typed symbols (`int` variables, `const int`, integer `#define`s) on the line in `float(..)` (HLSL mixes int and float freely).
fn fix_float(line: &str, ints: &[String]) -> String {
    // `intVar *= floatExpr;` -> HLSL truncates the right-hand side
    let t = line.trim_start();
    let id_end = t.bytes().position(|c| !is_id(c)).unwrap_or(0);
    if id_end > 0 && ints.iter().any(|n| n == &t[..id_end]) && ["+=", "-=", "*=", "/="].iter().any(|o| t[id_end..].trim_start().starts_with(o)) { return fix_assign(line, "float", "int"); }
    let b = line.as_bytes();
    let code = line.find("//").unwrap_or(line.len());
    let (mut out, mut i, mut br) = (String::new(), 0, 0i32);
    while i < b.len() {
        let c = b[i];
        if i >= code { out.push(c as char); i += 1; continue; }
        if c == b'[' { br += 1; } else if c == b']' { br -= 1; }
        if c.is_ascii_alphabetic() || c == b'_' {
            let st = i; while i < b.len() && is_id(b[i]) { i += 1; }
            let id = &line[st..i];
            let after = line[i..].trim_start();
            let before = line[..st].trim_end();
            let decl = ["int", "uint", "const", "float"].iter().any(|t| before.ends_with(t) && before[..before.len() - t.len()].bytes().last().map_or(true, |c| !is_id(c)));
            let assigned = after.starts_with("++") || after.starts_with("--") || (after.starts_with('=') && !after.starts_with("==")) || ["+=", "-=", "*=", "/="].iter().any(|o| after.starts_with(o));
            if br == 0 && !(st > 0 && b[st - 1] == b'.') && !decl && !assigned && !after.starts_with('(') && (after.starts_with(|c| "+-*/<>".contains(c)) || before.ends_with(|c| "+-*/<>".contains(c))) && !(before.ends_with("float(") && after.starts_with(')')) && ints.iter().any(|n| n == id) { out.push_str(&format!("float({id})")); } else { out.push_str(id); }
        } else if c.is_ascii_digit() {
            let st = i; while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'.') { i += 1; }
            let tok = &line[st..i];
            if br == 0 && tok.bytes().all(|d| d.is_ascii_digit()) && !(st > 0 && b[st - 1] == b'.') { out.push_str(tok); out.push_str(".0"); } else { out.push_str(tok); }
        } else { out.push(c as char); i += 1; }
    }
    out
}
fn int_symbols(src: &str) -> Vec<String> {
    let mut v: Vec<String> = vec![];
    for l in src.lines() {
        let t = l.trim_start();
        if let Some(r) = t.strip_prefix("#define ") {
            let mut it = r.split_whitespace();
            if let (Some(n), Some(val)) = (it.next(), it.next()) { if !val.is_empty() && val.bytes().all(|c| c.is_ascii_digit()) { v.push(n.to_string()); } }
            continue;
        }
        let t = t.strip_prefix("const ").unwrap_or(t).trim_start().strip_prefix("for (").or_else(|| t.strip_prefix("for(")).unwrap_or(t.strip_prefix("const ").unwrap_or(t));
        if let Some(r) = t.strip_prefix("int ").or_else(|| t.strip_prefix("uint ")) { if let Some(n) = r.split(|c: char| !is_id(c as u8)).next() { if !n.is_empty() { v.push(n.to_string()); } } }
    }
    v
}
fn set_varying(src: &str, name: &str, ty: &str) -> String {
    let mut o = String::new();
    for l in src.lines() {
        let t = l.trim_start();
        if t.starts_with("varying ") && has_word(t, name) && !t.contains(',') {
            let mut it = t.split_whitespace(); it.next();
            if let Some(old) = it.next() { o.push_str(&l.replacen(old, ty, 1)); o.push('\n'); continue; }
        }
        o.push_str(l); o.push('\n');
    }
    o
}
fn between<'a>(s: &'a str, a: &str, b: &str) -> Option<&'a str> { let i = s.find(a)? + a.len(); let j = s[i..].find(b)?; Some(&s[i..i + j]) }

/// Patches a shader that failed to compile/link, driven by the GL info log: HLSL-style implicit conversions (int/float mixing,
/// vecN truncation on init/assign, float array indices, float `%`) and mismatched varying widths between stages.
/// Returns false when the log held nothing it knows how to fix.
pub fn repair(tr: &mut Translated, err: &str) -> bool {
    let mut changed = false;
    if let Some(l) = err.lines().find(|l| l.contains("shader output `") && l.contains("declared as type")) {
        if let (Some(n), Some(t1), Some(t2)) = (between(l, "output `", "'"), between(l, "declared as type `", "'"), l.rsplit("declared as type `").next().and_then(|r| r.split('\'').next())) {
            let ty = if ty_dim(t1) >= ty_dim(t2) { t1 } else { t2 };
            let (v, f) = (set_varying(&tr.vert, n, ty), set_varying(&tr.frag, n, ty));
            if v != tr.vert || f != tr.frag { tr.vert = v; tr.frag = f; changed = true; }
        }
    }
    let is_frag = err.starts_with("fragment:");
    if !(err.starts_with("fragment:") || err.starts_with("vertex:")) { return changed; }
    let src = if is_frag { &tr.frag } else { &tr.vert };
    let ints = int_symbols(src);
    let mut lines: Vec<String> = src.lines().map(String::from).collect();
    let mut done: Vec<(usize, &str)> = vec![];
    for l in err.lines() {
        let l = l.strip_prefix("fragment:").or_else(|| l.strip_prefix("vertex:")).unwrap_or(l).trim_start();
        let Some(r) = l.strip_prefix("0:") else { continue };
        let Some((n, rest)) = r.split_once('(') else { continue };
        let Ok(n) = n.parse::<usize>() else { continue };
        let Some(msg) = rest.split_once("error: ").map(|x| x.1) else { continue };
        if n == 0 || n > lines.len() { continue; }
        let kind = if msg.contains("cannot be assigned to variable of type") { "assign" } else if msg.contains("array index must be integer") { "index" }
            else if msg.contains("must be an integer") && lines[n - 1].contains('%') { "mod" }
            else if msg.contains("could not implicitly convert operands") || msg.contains("must be numeric") { "float" } else { continue };
        if done.contains(&(n, kind)) { continue; }
        done.push((n, kind));
        let cur = lines[n - 1].clone();
        let new = match kind {
            "assign" => match (between(msg, "type ", " cannot"), msg.rsplit("variable of type ").next()) { (Some(f), Some(t)) if ty_dim(f).is_some() && ty_dim(t.trim()).is_some() => fix_assign(&cur, f, t.trim()), _ => cur.clone() },
            "index" => fix_index(&cur), "mod" => fix_mod(&cur), _ => fix_float(&cur, &ints),
        };
        if new != cur { lines[n - 1] = new; changed = true; }
    }
    let mut out = lines.join("\n"); out.push('\n');
    if is_frag { tr.frag = out; } else { tr.vert = out; }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fs(name: &str) -> Option<String> {
        Some(match name {
            "shaders/common.h" => "#define PI 3.14159\nvec3 gray(vec3 c) { return vec3(dot(c, vec3(0.3, 0.59, 0.11))); }\n",
            "shaders/effects/test.vert" => "#include \"common.h\"\nuniform mat4 g_ModelViewProjectionMatrix;\nattribute vec3 a_Position;\nattribute vec2 a_TexCoord;\nvarying vec2 v_TexCoord;\nvoid main() { gl_Position = mul(vec4(a_Position, 1.0), g_ModelViewProjectionMatrix); v_TexCoord = a_TexCoord; }\n",
            "shaders/effects/test.frag" => "// [COMBO] {\"material\":\"ui_editor_properties_mode\",\"combo\":\"MODE\",\"type\":\"options\",\"default\":1}\n#include \"common.h\"\nvarying vec2 v_TexCoord;\nuniform sampler2D g_Texture0; // {\"material\":\"framebuffer\",\"label\":\"ui_editor_properties_framebuffer\",\"hidden\":true}\nuniform float g_Time;\nuniform float u_amount; // {\"material\":\"amount\",\"default\":0.5,\"range\":[0,1]}\nvoid main() { vec4 c = texSample2D(g_Texture0, v_TexCoord);\n#if MODE == 1\n c.rgb = mix(c.rgb, gray(c.rgb), saturate(u_amount));\n#endif\n gl_FragColor = c; }\n",
            _ => return None,
        }.to_string())
    }
    fn tr(vert: &str, frag: &str) -> Translated { Translated { vert: vert.into(), frag: frag.into(), uniforms: vec![], uses_time: false, combos: BTreeMap::new() } }
    #[test]
    fn repairs_hlsl_conversions() {
        // float %, scalar chain on the left
        assert_eq!(fix_mod("float d = abs((x.x * n % 1.0) * 2.0);"), "float d = abs((wf_mod(x.x * n, 1.0)) * 2.0);");
        // float array index
        assert_eq!(fix_index("a = s[g_C] + t[2];"), "a = s[int(g_C)] + t[2];");
        // truncation / narrowing / widening on init
        assert_eq!(fix_assign("\tvec2 c = v_TexCoord;", "vec4", "vec2"), "\tvec2 c = vec2(v_TexCoord);");
        assert_eq!(fix_assign("float p = a * b;", "vec2", "float"), "float p = float(a * b);");
        assert_eq!(fix_assign("for (int i = u_Min; i < 3; i++) {", "float", "int"), "for (int i = int(u_Min); i < 3; i++) {");
        assert_eq!(fix_assign("vec4 c = v;", "vec2", "vec4"), "vec4 c = vec4(v, 0.0, 0.0);");
        // int symbols in float arithmetic, not as call args
        assert_eq!(fix_float("x = f / N * g(N);", &["N".to_string()]), "x = f / float(N) * g(N);");
        // driven by the info log
        let mut t = tr("", "void main() {\n\tvec3 c = vec4(1.0);\n\tfloat f = v[g];\n}\n");
        assert!(repair(&mut t, "fragment: 0:2(2): error: initializer of type vec4 cannot be assigned to variable of type vec3\n0:3(14): error: array index must be integer type"));
        assert!(t.frag.contains("vec3 c = vec3(vec4(1.0));") && t.frag.contains("v[int(g)]"));
        // varying width mismatch widens both stages
        let mut t = tr("varying vec4 v_T;\n", "varying vec2 v_T;\n");
        assert!(repair(&mut t, "link: error: vertex shader output `v_T' declared as type `vec4', but fragment shader input declared as type `vec2'"));
        assert_eq!((t.vert.as_str(), t.frag.as_str()), ("varying vec4 v_T;\n", "varying vec4 v_T;\n"));
        assert!(!repair(&mut t, "fragment: 0:1(1): error: something else"));
        // prelude carries the mismatched mix / float mod overloads
        assert!(PRE.contains("vec3 wf_mix(vec3 a, vec3 b, vec4 t)") && PRE.contains("float wf_mod(float a, float b)"));
    }
    #[test]
    fn translates_sample() {
        let t = translate(&fs, "effects/test", &Map::new(), &[], false).unwrap();
        assert!(t.vert.starts_with("#version 300 es"));
        assert!(t.frag.contains("#define MODE 1"));
        assert!(t.vert.contains("vec3 gray") && !t.vert.contains("#include"));
        assert_eq!(t.combos["MODE"], "1");
        let names: Vec<_> = t.uniforms.iter().map(|u| u.name.as_str()).collect();
        assert!(names.contains(&"g_Texture0") && names.contains(&"u_amount") && names.contains(&"g_ModelViewProjectionMatrix"));
        let u = t.uniforms.iter().find(|u| u.name == "u_amount").unwrap();
        assert_eq!((u.material.as_deref(), u.default.clone()), (Some("amount"), Some(serde_json::json!(0.5))));
        assert!(t.uses_time);
        let mut over = Map::new(); over.insert("MODE".into(), serde_json::json!(0));
        assert!(translate(&fs, "effects/test", &over, &[], false).unwrap().frag.contains("#define MODE 0"));
        assert_eq!(undefined_in_conditions("#define A 1\n#if A == 1 && defined(B) && MASK\n#endif\n#elif SPEC > 2\n"), vec!["MASK", "SPEC"]);
        assert_eq!(floatify_src("const int N = 3;\n\tconst float d = N - 1.0;\n"), "const int N = 3;\n\tconst float d = float(N) - 1.0;\n");
        assert_eq!(floatify_src("float b = 2 * x + v[1] - smoothstep(0, 1.5, y);\nint k = 3;\n"), "float b = 2.0 * x + v[1] - smoothstep(0.0, 1.5, y);\nint k = 3;\n");
        assert!(translate(&fs, "effects/none", &Map::new(), &[], false).is_err());
    }
}

fn has_word(line: &str, w: &str) -> bool {
    let b = line.as_bytes();
    line.match_indices(w).any(|(i, _)| {
        let before = i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_');
        let j = i + w.len();
        let after = j >= b.len() || !(b[j].is_ascii_alphanumeric() || b[j] == b'_');
        before && after
    })
}
