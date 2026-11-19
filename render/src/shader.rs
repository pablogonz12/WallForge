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
         combos: &mut BTreeMap<String, String>) -> Result<String, String> {
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
    for (k, v) in &all { o.push_str(&format!("#define {k} {v}\n")); }
    o.push_str(&src);
    Ok(o)
}

pub fn translate(read: &dyn Fn(&str) -> Option<String>, shader: &str, combo_over: &Map<String, Value>) -> Result<Translated, String> {
    let mut uniforms = vec![];
    let mut combos = BTreeMap::new();
    // two passes so the vertex stage also sees combos declared in the fragment stage and vice versa
    let _ = stage(read, &format!("shaders/{shader}.vert"), false, combo_over, &mut uniforms, &mut combos)?;
    let _ = stage(read, &format!("shaders/{shader}.frag"), true, combo_over, &mut uniforms, &mut combos)?;
    let vert = stage(read, &format!("shaders/{shader}.vert"), false, combo_over, &mut uniforms, &mut combos)?;
    let frag = stage(read, &format!("shaders/{shader}.frag"), true, combo_over, &mut uniforms, &mut combos)?;
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
        let t = translate(&fs, "effects/test", &Map::new()).unwrap();
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
        assert!(translate(&fs, "effects/test", &over).unwrap().frag.contains("#define MODE 0"));
        assert!(translate(&fs, "effects/none", &Map::new()).is_err());
    }
}
