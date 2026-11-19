//! scene.pkg parser + virtual file system (pkg -> wallpaper dir -> Wallpaper Engine assets dir).
//! Format (public docs: RePKG): u32 len + "PKGV000x", u32 count, count * (u32 len + name, u32 offset, u32 size),
//! file data follows the header; offsets are relative to the end of the header.
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub struct Pkg {
    data: Vec<u8>,
    entries: HashMap<String, (usize, usize)>, // lowercase name -> (abs offset, size)
    pub names: Vec<String>,
}

struct Rd<'a> { b: &'a [u8], p: usize }
impl<'a> Rd<'a> {
    fn u32(&mut self) -> Result<u32, String> {
        let s = self.b.get(self.p..self.p + 4).ok_or("pkg: truncated header")?;
        self.p += 4;
        Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }
    fn str(&mut self) -> Result<String, String> {
        let n = self.u32()? as usize;
        if n > 4096 { return Err("pkg: implausible string length".into()); }
        let s = self.b.get(self.p..self.p + n).ok_or("pkg: truncated string")?;
        self.p += n;
        Ok(String::from_utf8_lossy(s).into_owned())
    }
}

impl Pkg {
    pub fn parse(data: Vec<u8>) -> Result<Pkg, String> {
        let mut r = Rd { b: &data, p: 0 };
        let magic = r.str()?;
        if !magic.starts_with("PKGV") { return Err(format!("not a scene.pkg (magic '{magic}')")); }
        let count = r.u32()? as usize;
        if count > 1_000_000 { return Err("pkg: implausible entry count".into()); }
        let mut raw = Vec::with_capacity(count);
        for _ in 0..count {
            let name = r.str()?;
            let off = r.u32()? as usize;
            let size = r.u32()? as usize;
            raw.push((name, off, size));
        }
        let base = r.p;
        let mut entries = HashMap::new();
        let mut names = Vec::new();
        for (name, off, size) in raw {
            let abs = base.checked_add(off).ok_or("pkg: bad offset")?;
            if abs.checked_add(size).map_or(true, |e| e > data.len()) { return Err(format!("pkg: entry '{name}' outside file")); }
            names.push(name.clone());
            entries.insert(name.to_lowercase(), (abs, size));
        }
        Ok(Pkg { data, entries, names })
    }
    pub fn get(&self, name: &str) -> Option<&[u8]> {
        let (o, s) = *self.entries.get(&norm(name))?;
        Some(&self.data[o..o + s])
    }
}

fn norm(n: &str) -> String { n.replace('\\', "/").trim_start_matches('/').to_lowercase() }

/// Layered lookup. Names are case-insensitive inside the pkg (WE content is Windows-authored).
pub struct Vfs {
    pub pkg: Option<Pkg>,
    pub dir: PathBuf,
    pub assets: Option<PathBuf>,
}

pub fn detect_assets_dir() -> Option<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    [".local/share/Steam/steamapps/common/wallpaper_engine/assets", ".steam/steam/steamapps/common/wallpaper_engine/assets",
     ".var/app/com.valvesoftware.Steam/.local/share/Steam/steamapps/common/wallpaper_engine/assets", ".local/share/wallforge/assets"]
        .iter().map(|p| home.join(p)).find(|p| p.is_dir())
}

impl Vfs {
    pub fn open(dir: &Path, assets: Option<PathBuf>) -> Result<Vfs, String> {
        let pkg = match std::fs::read(dir.join("scene.pkg")) {
            Ok(d) => Some(Pkg::parse(d)?),
            Err(_) => None,
        };
        Ok(Vfs { pkg, dir: dir.to_path_buf(), assets: assets.or_else(detect_assets_dir) })
    }
    fn from_disk(root: &Path, name: &str) -> Option<Vec<u8>> {
        let rel = name.replace('\\', "/");
        let rel = rel.trim_start_matches('/');
        if rel.split('/').any(|c| c == "..") { return None; }
        if let Ok(d) = std::fs::read(root.join(rel)) { return Some(d); }
        // case-insensitive walk (pkg names are lowercase-insensitive, Linux disks are not)
        let mut cur = root.to_path_buf();
        for comp in rel.split('/') {
            let want = comp.to_lowercase();
            let ent = std::fs::read_dir(&cur).ok()?.flatten().find(|e| e.file_name().to_string_lossy().to_lowercase() == want)?;
            cur = ent.path();
        }
        std::fs::read(cur).ok()
    }
    pub fn read(&self, name: &str) -> Option<Vec<u8>> {
        if let Some(p) = &self.pkg { if let Some(d) = p.get(name) { return Some(d.to_vec()); } }
        if let Some(d) = Self::from_disk(&self.dir, name) { return Some(d); }
        if let Some(a) = &self.assets { if let Some(d) = Self::from_disk(a, name) { return Some(d); } }
        None
    }
    pub fn read_str(&self, name: &str) -> Option<String> { self.read(name).map(|d| String::from_utf8_lossy(&d).into_owned()) }
    pub fn read_json(&self, name: &str) -> Option<serde_json::Value> {
        let s = self.read_str(name)?;
        serde_json::from_str(s.trim_start_matches('\u{feff}')).ok()
    }
    pub fn exists(&self, name: &str) -> bool {
        self.pkg.as_ref().map_or(false, |p| p.get(name).is_some()) || self.read(name).is_some()
    }
}

/// Build a PKGV0001 archive (used by tests and `--make-test-scene`).
pub fn build(files: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut hdr = Vec::new();
    let put_s = |v: &mut Vec<u8>, s: &str| { v.extend((s.len() as u32).to_le_bytes()); v.extend(s.as_bytes()); };
    put_s(&mut hdr, "PKGV0001");
    hdr.extend((files.len() as u32).to_le_bytes());
    let mut off = 0u32;
    for (n, d) in files { put_s(&mut hdr, n); hdr.extend(off.to_le_bytes()); hdr.extend((d.len() as u32).to_le_bytes()); off += d.len() as u32; }
    for (_, d) in files { hdr.extend(d); }
    hdr
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roundtrip() {
        let p = Pkg::parse(build(&[("scene.json", b"{}".to_vec()), ("Materials/A.tex", vec![1, 2, 3])])).unwrap();
        assert_eq!(p.get("scene.json").unwrap(), b"{}");
        assert_eq!(p.get("materials\\a.tex").unwrap(), &[1, 2, 3]);
        assert!(p.get("nope").is_none());
        assert_eq!(p.names.len(), 2);
    }
    #[test]
    fn rejects_garbage() {
        assert!(Pkg::parse(vec![1, 2, 3]).is_err());
        let mut d = build(&[("a", vec![0; 8])]);
        d.truncate(d.len() - 4);
        assert!(Pkg::parse(d).is_err());
        assert!(Pkg::parse(vec![4, 0, 0, 0, b'X', b'X', b'X', b'X', 0, 0, 0, 0]).is_err());
    }
}
