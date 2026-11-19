//! Synthetic scene generator (`--make-test-scene DIR`): project.json + scene.pkg with no real Wallpaper Engine content.
use crate::pkg;
use crate::tex::synth;
use std::path::Path;

const VERT: &str = "uniform mat4 g_ModelViewProjectionMatrix;
attribute vec3 a_Position;
attribute vec2 a_TexCoord;
varying vec2 v_TexCoord;
void main() { gl_Position = mul(vec4(a_Position, 1.0), g_ModelViewProjectionMatrix); v_TexCoord = a_TexCoord; }
";
const FRAG_TINT: &str = "// [COMBO] {\"material\":\"Mode\",\"combo\":\"MODE\",\"type\":\"options\",\"default\":1}
varying vec2 v_TexCoord;
uniform sampler2D g_Texture0; // {\"material\":\"framebuffer\",\"hidden\":true}
uniform float g_Time;
uniform float u_amount; // {\"material\":\"amount\",\"default\":0.5}
void main() {
  vec4 c = texSample2D(g_Texture0, v_TexCoord);
#if MODE == 1
  float g = dot(c.rgb, vec3(0.3, 0.59, 0.11));
  c.rgb = mix(c.rgb, vec3(g), saturate(u_amount));
#endif
  c.rgb *= 0.85 + 0.15 * sin(g_Time * 3.0);
  gl_FragColor = c;
}
";
const FRAG_BLUR: &str = "// [COMBO] {\"material\":\"Direction\",\"combo\":\"DIR\",\"default\":0}
varying vec2 v_TexCoord;
uniform sampler2D g_Texture0;
uniform vec4 g_Texture0Resolution;
void main() {
  vec2 d = (DIR == 0 ? vec2(1.0, 0.0) : vec2(0.0, 1.0)) / g_Texture0Resolution.xy;
  vec4 s = vec4(0.0);
  for (int i = -4; i <= 4; i++) { float w = 1.0 - abs(float(i)) / 5.0; s += texSample2D(g_Texture0, v_TexCoord + d * float(i) * 1.5) * w; }
  gl_FragColor = s / 5.0;
}
";

fn j(v: serde_json::Value) -> Vec<u8> { v.to_string().into_bytes() }

pub fn files() -> Vec<(&'static str, Vec<u8>)> {
    use serde_json::json;
    let bg = synth::rgba_gradient(256, 144, |x, y| [(40 + x / 3) as u8, (60 + y) as u8, 160, 255]);
    let sq = synth::png(64, 64, &synth::rgba_gradient(64, 64, |x, y| if (x / 8 + y / 8) % 2 == 0 { [255, 120, 30, 255] } else { [30, 200, 90, 255] }));
    // DXT1, 8x8 = 4 blocks: red/blue, green/white, yellow/black, cyan/magenta endpoints
    let dxt: Vec<u8> = [(0xf800u16, 0x001fu16), (0x07e0, 0xffff), (0xffe0, 0x0000), (0x07ff, 0xf81f)].iter()
        .flat_map(|(a, b)| { let mut v = a.to_le_bytes().to_vec(); v.extend(b.to_le_bytes()); v.extend([0x44, 0x44, 0xee, 0xee]); v }).collect();
    let mut ctx = vec![];
    let mut add = |n: &'static str, d: Vec<u8>| ctx.push((n, d));
    add("scene.json", j(json!({"general": {"clearcolor": "0.05 0.05 0.1", "orthogonalprojection": {"width": 1920, "height": 1080}},
        "objects": [
          {"id": 1, "name": "background", "image": "models/bg.json", "origin": "960 540 0", "size": "1920 1080", "alpha": 1.0},
          {"id": 2, "name": "square", "image": "models/sq.json", "origin": "600 540 0", "size": "320 320", "angles": "0 0 30", "alpha": 0.9,
           "effects": [{"file": "effects/wftest/effect.json", "passes": [{"constantshadervalues": {"amount": {"user": "gray", "value": 0.0}}}]}]},
          {"id": 3, "name": "child", "image": "models/sq.json", "parent": 2, "origin": "260 0 0", "size": "120 120", "color": "1 0.6 0.6"},
          {"id": 4, "name": "glow", "image": "models/dxt.json", "origin": "1350 600 0", "size": "420 420", "scale": "1 1 1", "alpha": 0.8, "color": "1 1 1",
           "effects": [{"file": "effects/wfblur/effect.json"}]},
          {"id": 5, "name": "hidden", "image": "models/sq.json", "visible": {"user": "showhidden", "value": false}},
          {"id": 6, "name": "snd", "sound": ["sounds/x.mp3"]}]})));
    add("models/bg.json", j(json!({"material": "materials/bg.json", "autosize": true})));
    add("models/sq.json", j(json!({"material": "materials/sq.json"})));
    add("models/dxt.json", j(json!({"material": "materials/dxt.json"})));
    add("materials/bg.json", j(json!({"passes": [{"blending": "translucent", "shader": "genericimagev2", "textures": ["bg"]}]})));
    add("materials/sq.json", j(json!({"passes": [{"blending": "translucent", "shader": "genericimagev2", "textures": ["sq"]}]})));
    add("materials/dxt.json", j(json!({"passes": [{"blending": "additive", "shader": "genericimagev2", "textures": ["dxt"]}]})));
    add("materials/bg.tex", synth::tex(0, 2, 256, 144, 256, 144, 3, -1, &[(256, 144, bg, true)]));
    add("materials/sq.tex", synth::tex(0, 2, 64, 64, 64, 64, 3, 13, &[(64, 64, sq, false)]));
    add("materials/dxt.tex", synth::tex(7, 2, 8, 8, 8, 8, 2, -1, &[(8, 8, dxt, false)]));
    add("effects/wftest/effect.json", j(json!({"name": "wftest", "passes": [{"material": "materials/effects/wftest.json"}]})));
    add("materials/effects/wftest.json", j(json!({"passes": [{"shader": "effects/wftest", "textures": [null], "blending": "translucent"}]})));
    add("effects/wfblur/effect.json", j(json!({"name": "wfblur", "fbos": [{"name": "_rt_half", "scale": 2}],
        "passes": [{"material": "materials/effects/wfblur_h.json", "target": "_rt_half", "bind": [{"name": "previous", "index": 0}]},
                   {"material": "materials/effects/wfblur_v.json", "bind": [{"name": "_rt_half", "index": 0}]}]})));
    add("materials/effects/wfblur_h.json", j(json!({"passes": [{"shader": "effects/wfblur", "combos": {"DIR": 0}, "textures": [null]}]})));
    add("materials/effects/wfblur_v.json", j(json!({"passes": [{"shader": "effects/wfblur", "combos": {"DIR": 1}, "textures": [null]}]})));
    add("shaders/effects/wftest.vert", VERT.as_bytes().to_vec());
    add("shaders/effects/wftest.frag", FRAG_TINT.as_bytes().to_vec());
    add("shaders/effects/wfblur.vert", VERT.as_bytes().to_vec());
    add("shaders/effects/wfblur.frag", FRAG_BLUR.as_bytes().to_vec());
    ctx
}

pub fn make(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("scene.pkg"), pkg::build(&files())).map_err(|e| e.to_string())?;
    let proj = serde_json::json!({"title": "wallforge synthetic test scene", "type": "scene", "file": "scene.json",
        "general": {"properties": {"gray": {"type": "slider", "value": 0.7}, "showhidden": {"type": "bool", "value": false}}}});
    std::fs::write(dir.join("project.json"), proj.to_string()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{scene, shader};
    #[test]
    fn synthetic_scene_parses_end_to_end() {
        let d = std::env::temp_dir().join(format!("wf-synth-{}", std::process::id()));
        make(&d).unwrap();
        let vfs = pkg::Vfs::open(&d, Some(d.join("no-assets"))).unwrap();
        assert!(vfs.pkg.is_some());
        let proj: serde_json::Value = serde_json::from_slice(&std::fs::read(d.join("project.json")).unwrap()).unwrap();
        let props = scene::Props::load(&proj, &[]);
        let s = scene::parse(&vfs.read_json("scene.json").unwrap(), &props).unwrap();
        assert_eq!(s.objects.len(), 5); // hidden one dropped
        let r = scene::report(&vfs, &s);
        assert_eq!(r["supported"], true);
        assert_eq!(r["images"], 4);
        assert!(r["missing_files"].as_array().unwrap().is_empty(), "{r}");
        let img = scene::load_image(&vfs, "models/dxt.json").unwrap();
        let t = crate::tex::parse(&vfs.read(img.tex_path.as_deref().unwrap()).unwrap(), false).unwrap();
        assert_eq!((t.img_w, t.img_h), (8, 8));
        let tr = shader::translate(&|n| vfs.read_str(n), "effects/wfblur", &serde_json::Map::new(), &[], false).unwrap();
        assert!(tr.frag.contains("#define DIR 0"));
        let _ = std::fs::remove_dir_all(&d);
    }
}
