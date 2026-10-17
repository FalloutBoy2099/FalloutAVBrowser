//! Mémorisation de la taille et de la position de la fenêtre (fichier texte `window.txt`).

use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct WinPrefs {
    pub remember_size: bool,
    pub remember_pos: bool,
    pub size: Option<[f32; 2]>,
    pub pos: Option<[f32; 2]>,
}

impl Default for WinPrefs {
    fn default() -> Self {
        WinPrefs { remember_size: true, remember_pos: true, size: None, pos: None }
    }
}

fn file() -> Option<PathBuf> {
    eframe::storage_dir("FalloutAVBrowser").map(|d| d.join("window.txt"))
}

fn pair(v: &str) -> Option<[f32; 2]> {
    let (a, b) = v.split_once(',')?;
    let (a, b): (f32, f32) = (a.trim().parse().ok()?, b.trim().parse().ok()?);
    Some([a, b])
}

impl WinPrefs {
    pub fn load() -> WinPrefs {
        let mut w = WinPrefs::default();
        let Some(text) = file().and_then(|f| std::fs::read_to_string(f).ok()) else { return w };
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            match k.trim() {
                "remember_size" => w.remember_size = v.trim() == "1",
                "remember_pos" => w.remember_pos = v.trim() == "1",
                "size" => w.size = pair(v).filter(|s| s[0] >= 200.0 && s[1] >= 150.0 && s[0] < 20000.0 && s[1] < 20000.0),
                "pos" => w.pos = pair(v).filter(|p| p[0] > -16000.0 && p[1] > -16000.0 && p[0] < 16000.0 && p[1] < 16000.0),
                _ => {}
            }
        }
        w
    }

    pub fn save(&self) {
        let Some(f) = file() else { return };
        if let Some(d) = f.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let mut s = format!("remember_size={}\nremember_pos={}\n", self.remember_size as u8, self.remember_pos as u8);
        if let Some(v) = self.size {
            s += &format!("size={},{}\n", v[0], v[1]);
        }
        if let Some(v) = self.pos {
            s += &format!("pos={},{}\n", v[0], v[1]);
        }
        let _ = std::fs::write(f, s);
    }
}
