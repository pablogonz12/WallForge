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

fn strip_comments(s: &str) -> String { s.lines().map(|l| match l.find("//") { Some(i) if !l[..i].contains(['\'', '"']) => &l[..i], _ => l }).collect::<Vec<_>>().join("\n") }
fn pstr(p: &Map<String, Value>, k: &str) -> Option<String> { p.get(k).map(|v| match v { Value::String(s) => s.clone(), Value::Bool(b) => b.to_string(), x => x.to_string() }) }
fn quoted(s: &str) -> Vec<String> {
    let (mut v, mut c, mut q) = (vec![], String::new(), None::<char>);
    let mut it = s.chars();
    while let Some(ch) = it.next() {
        match q { None => if ch == '\'' || ch == '"' { q = Some(ch); c.clear(); },
            Some(e) => if ch == '\\' { match it.next() { Some('n') => c.push('\n'), Some(x) => c.push(x), None => {} } } else if ch == e { v.push(c.clone()); q = None; } else { c.push(ch); } }
    }
    v
}
/// `name = [ 'a', 'b', ... ]` chosen by `if (scriptProperties.<prop> == N)` matching the current property value.
fn pick_array(src: &str, prop: &str, p: &Map<String, Value>) -> Option<Vec<String>> {
    let want = pstr(p, prop)?; let key = format!("scriptProperties.{prop} ==");
    let mut from = 0;
    while let Some(i) = src[from..].find(&key) {
        let a = from + i + key.len(); from = a;
        let n: String = src[a..].trim_start().trim_start_matches(['=', ' ']).chars().take_while(|c| c.is_alphanumeric() || *c == '\'' || *c == '"').collect::<String>().trim_matches(['\'', '"']).to_string();
        let lb = src[a..].find('[')? + a; let rb = src[lb..].find(']')? + lb;
        if n == want || n.parse::<f64>().ok() == want.parse::<f64>().ok() { return Some(quoted(&src[lb..rb])); }
    }
    None
}
fn split_plus(e: &str) -> Vec<String> {
    let (mut v, mut d, mut q, mut cur) = (vec![], 0i32, None::<char>, String::new());
    for c in e.chars() {
        match q { Some(x) => { cur.push(c); if c == x { q = None; } } None => match c {
            '\'' | '"' => { q = Some(c); cur.push(c); } '(' | '[' => { d += 1; cur.push(c); } ')' | ']' => { d -= 1; cur.push(c); }
            '+' if d == 0 => { v.push(cur.trim().to_string()); cur.clear(); } _ => cur.push(c) } }
    }
    v.push(cur.trim().to_string()); v
}
struct Env<'a> { t: &'a Tm, p: &'a Map<String, Value>, months: Vec<String>, day: Vec<String> }
impl Env<'_> {
    fn term(&self, k: &str) -> Option<String> {
        let k = k.trim();
        if let Some(l) = quoted(k).into_iter().next() { if k.starts_with(['\'', '"']) { return Some(l); } }
        let t = self.t; let two = |x: i32| format!("{:02}", x);
        Some(match k {
            "date.getFullYear()" => t.year.to_string(), "date.getDate()" => t.day.to_string(), "currentDate" => two(t.day),
            "delimiterValue" => if pstr(self.p, "useDelimiter").as_deref() == Some("true") { pstr(self.p, "addDelimiter").unwrap_or_default() } else { " ".into() },
            "newLine" => if pstr(self.p, "alignVertical").as_deref() == Some("true") { "\n".into() } else { String::new() },
            "scriptProperties.delimiter" => pstr(self.p, "delimiter").unwrap_or(":".into()),
            "hours" => two(if flag(self.p, "use24hFormat", true) { t.h } else { (t.h + 11) % 12 + 1 }), "minutes" => two(t.m), "seconds" => two(t.s),
            "date.getMonth()" => (t.mon + 1).to_string(),
            _ if k.starts_with("months[") => self.months.get(t.mon as usize)?.clone(),
            _ if k.starts_with("day[") => self.day.get(t.wday as usize)?.clone(),
            _ => return None })
    }
    fn expr(&self, e: &str) -> Option<String> { let mut o = String::new(); for tk in split_plus(e.trim().trim_end_matches(';')) { o += &self.term(&tk)?; } Some(o) }
}

pub fn eval_at(script: &str, p: &Map<String, Value>, t: &Tm) -> Option<String> {
    let src = strip_comments(script);
    let env = Env { t, p, months: pick_array(&src, "monthFormat", p).unwrap_or_default(), day: pick_array(&src, "dayFormat", p).unwrap_or_default() };
    if src.contains("getHours") {
        let ampm = if !flag(p, "use24hFormat", true) { if t.h >= 12 { " PM" } else { " AM" } } else { "" };
        // `value = <expr>;` right after the minutes are computed (literal prefix/suffix and delimiter live there)
        let base = src.find("let minutes").and_then(|i| src[i..].find("value =").map(|j| i + j + 7)).and_then(|i| src[i..].find(';').map(|e| &src[i..i + e])).and_then(|e| env.expr(e));
        let mut s = base.unwrap_or_else(|| format!("{}{}{}", env.term("hours").unwrap(), env.term("scriptProperties.delimiter").unwrap(), env.term("minutes").unwrap()));
        if flag(p, "showSeconds", false) { s += &format!("{}{}", env.term("scriptProperties.delimiter").unwrap(), env.term("seconds").unwrap()); }
        if !ampm.is_empty() && src.contains("ampm") { s += ampm; }
        return Some(s);
    }
    if src.contains("date.getMonth()") || src.contains("date.getDay()") {
        let i = src.find("new Date()")?; let body = &src[i..];
        let show = pstr(p, "showDay").as_deref() == Some("true");
        let mut best: Option<String> = None;
        for part in body.split("scriptProperties.showDay ==").skip(1) {
            let want = part.trim_start().starts_with("true");
            let r = part.find("return ")? + 7; let e = part[r..].find('\n').unwrap_or(part.len() - r);
            if want == show { best = env.expr(&part[r..r + e]); break; }
        }
        if best.is_none() { // no showDay switch: use the first return
            let r = body.find("return ")? + 7; let e = body[r..].find('\n').unwrap_or(body.len() - r); best = env.expr(&body[r..r + e]);
        }
        return best;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clock_with_literals_and_date() {
        let t = Tm { h: 9, m: 5, s: 7, day: 8, mon: 9, year: 2023, wday: 0 };
        let mut p = Map::new(); p.insert("use24hFormat".into(), Value::Bool(true)); p.insert("delimiter".into(), ":".into());
        let sc = "export function update(value) { let time = new Date(); var hours = time.getHours(); hours = (\"00\" + hours).slice(-2); let minutes = (\"00\" + time.getMinutes()).slice(-2);\n value = \"-\" + hours + scriptProperties.delimiter + minutes+\"-\";\n return value; }";
        assert_eq!(eval_at(sc, &p, &t).as_deref(), Some("-09:05-"));
        let sc = "if (scriptProperties.monthFormat == 2){ months = ['JAN','FEB','MAR','APR','MAY','JUN','JUL','AUG','SEP','OCT','NOV','DEC'] }\n let date = new Date();{\n if (scriptProperties.showDay == false ){\n return date.getDate() + delimiterValue + months[date.getMonth()] + delimiterValue + date.getFullYear();\n}}";
        let mut p = Map::new(); p.insert("monthFormat".into(), "2".into()); p.insert("showDay".into(), Value::Bool(false));
        assert_eq!(eval_at(sc, &p, &t).as_deref(), Some("8 OCT 2023"));
    }
}
