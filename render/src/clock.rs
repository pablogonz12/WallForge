//! Recognises the common clock/date text scripts (SceneScript is not run) and renders their string for the current local time.
use serde_json::{Map, Value};

pub struct Tm { pub h: i32, pub m: i32, pub s: i32, pub day: i32, pub mon: i32, pub year: i32, pub wday: i32 }

pub fn now() -> Tm {
    unsafe {
        let t = libc::time(std::ptr::null_mut()); let mut tm: libc::tm = std::mem::zeroed(); libc::localtime_r(&t, &mut tm);
        Tm { h: tm.tm_hour, m: tm.tm_min, s: tm.tm_sec, day: tm.tm_mday, mon: tm.tm_mon, year: tm.tm_year + 1900, wday: tm.tm_wday }
    }
}

fn flag(p: &Map<String, Value>, k: &str, d: bool) -> bool { match p.get(k) { Some(Value::Bool(b)) => *b, Some(Value::Number(n)) => n.as_f64() != Some(0.0), Some(Value::String(s)) => s == "true" || s == "1", _ => d } }

pub fn eval(script: &str, p: &Map<String, Value>) -> Option<String> { eval_at(script, p, &now()) }

pub fn eval_at(script: &str, p: &Map<String, Value>, t: &Tm) -> Option<String> {
    if script.contains("getHours") {
        let (mut h, mut ampm) = (t.h, "");
        if !flag(p, "use24hFormat", true) { ampm = if h >= 12 { " PM" } else { " AM" }; h %= 12; if h == 0 { h = 12; } }
        let delim = p.get("delimiter").and_then(|v| v.as_str()).unwrap_or(":");
        let mut s = format!("{:02}{delim}{:02}", h, t.m);
        if flag(p, "showSeconds", false) { s += &format!("{delim}{:02}", t.s); }
        return Some(s + ampm);
    }
    None
}
