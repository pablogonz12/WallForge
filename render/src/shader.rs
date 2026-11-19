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
#define texture2D texture
#define texSample2D texture
#define texSample2DLod textureLod
#define texture2DLod textureLod
";

fn expand(read: &dyn Fn(&str) -> Option<String>, src: &str, depth: u32, out: &mut String) -> Result<(), String> {
    if depth > 8 { return Err("#include nesting too deep".into()); }
    for line in src.lines() {
        let t = line.trim_start();
        if let Some(rest) = t.strip_prefix("#include") {
            let name = rest.trim().trim_matches(|c| c == '"' || c == '<' || c == '>');
            let inc = read(&format!("shaders/{name}")).ok_or_else(|| format!("missing include '{name}'"))?;
            expand(read, &inc, depth + 1, out)?;
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
    o.push_str(&if floatify { floatify_src(&src) } else { src });
    Ok(o)
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
        let intish = t.starts_with('#') || line.contains("int ") || line.contains("ivec") || line.contains("uint") || line.contains("switch") || line.contains("case ");
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
