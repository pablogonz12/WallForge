//! Cheap evaluation of the common `applyUserProperties` visibility script:
//! `if (property.NAME === true) { thisLayer.visible = false; }` (SceneScript itself is not run).
use serde_json::Value;

/// `v` = the object's `visible` field `{script, user, value}`; `user_val` = current value of the bound user property.
pub fn script_visible(v: &Value, user_val: Option<bool>) -> Option<bool> {
    let o = v.as_object()?; let src = o.get("script")?.as_str()?; let user = o.get("user")?.as_str()?; let pv = user_val?;
    let key = format!("property.{user}");
    let mut from = 0;
    while let Some(i) = src[from..].find("thisLayer.visible") {
        let at = from + i; from = at + 17;
        let rest = src[from..].trim_start();
        let Some(rest) = rest.strip_prefix('=') else { continue };
        if rest.starts_with('=') { continue; }
        let set = if rest.trim_start().starts_with("true") { true } else if rest.trim_start().starts_with("false") { false } else { continue };
        let Some(k) = src[..at].rfind(&key) else { continue };
        let cond = src[k + key.len()..at].trim_start();
        let cond = cond.trim_start_matches(|c| c == '=' || c == '!').trim_start();
        let lit = if cond.starts_with("true") { true } else if cond.starts_with("false") { false } else { continue };
        let neg = src[k + key.len()..].trim_start().starts_with('!');
        if (lit == pv) != neg { return Some(set); }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inverted() {
        let v: Value = serde_json::json!({"user": "h", "value": false, "script": "export function a(property){ if(property.h === true) { thisLayer.visible = false; }\n if(property.h === false) { thisLayer.visible = true; } }"});
        assert_eq!(script_visible(&v, Some(false)), Some(true)); assert_eq!(script_visible(&v, Some(true)), Some(false));
    }
}
