//! Ouvre un fichier avec le programme par défaut du système (Windows, macOS, Linux).

use std::path::{Path, PathBuf};

/// Pour un fichier situé dans une archive .DAT, on l'extrait d'abord dans le dossier temporaire.
fn real_path(path: &Path) -> Result<PathBuf, String> {
    if crate::dat::is_virtual(path) {
        let data = crate::dat::read_any(path).map_err(|e| e.to_string())?;
        let name = path.file_name().ok_or("nom de fichier invalide")?;
        let dir = std::env::temp_dir().join("FalloutAVBrowser");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let out = dir.join(name);
        std::fs::write(&out, data).map_err(|e| e.to_string())?;
        Ok(out)
    } else {
        Ok(path.to_path_buf())
    }
}

#[cfg(windows)]
pub fn open_default(path: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    let p = real_path(path)?;
    let wide: Vec<u16> = p.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    let verb: Vec<u16> = "open\0".encode_utf16().collect();
    // SW_SHOWNORMAL = 1 ; une valeur > 32 signale le succès.
    let r = unsafe { ShellExecuteW(std::ptr::null_mut(), verb.as_ptr(), wide.as_ptr(), std::ptr::null(), std::ptr::null(), 1) };
    if (r as isize) > 32 { Ok(()) } else { Err(format!("{} (code {})", p.display(), r as isize)) }
}

#[cfg(target_os = "macos")]
pub fn open_default(path: &Path) -> Result<(), String> {
    let p = real_path(path)?;
    std::process::Command::new("open").arg(&p).spawn().map(|_| ()).map_err(|e| e.to_string())
}

#[cfg(all(not(windows), not(target_os = "macos")))]
pub fn open_default(path: &Path) -> Result<(), String> {
    let p = real_path(path)?;
    std::process::Command::new("xdg-open").arg(&p).spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// Ouvre l'explorateur du système sur un dossier, ou sur le dossier contenant le fichier en le sélectionnant.
/// Pour un élément situé dans une archive .DAT, c'est l'archive qui est sélectionnée.
pub fn reveal(path: &Path) -> Result<(), String> {
    let real = match crate::dat::split(path) {
        Some((arch, _)) => arch,
        None => path.to_path_buf(),
    };
    if !real.exists() {
        return Err(format!("{} : introuvable", real.display()));
    }
    reveal_real(&real)
}

#[cfg(windows)]
fn reveal_real(p: &Path) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    if p.is_dir() {
        return open_default(p);
    }
    std::process::Command::new("explorer").raw_arg(format!("/select,\"{}\"", p.display())).spawn().map(|_| ()).map_err(|e| e.to_string())
}

#[cfg(target_os = "macos")]
fn reveal_real(p: &Path) -> Result<(), String> {
    let mut c = std::process::Command::new("open");
    if !p.is_dir() {
        c.arg("-R");
    }
    c.arg(p).spawn().map(|_| ()).map_err(|e| e.to_string())
}

#[cfg(all(not(windows), not(target_os = "macos")))]
fn reveal_real(p: &Path) -> Result<(), String> {
    let dir = if p.is_dir() { p.to_path_buf() } else { p.parent().map(|d| d.to_path_buf()).unwrap_or_default() };
    std::process::Command::new("xdg-open").arg(dir).spawn().map(|_| ()).map_err(|e| e.to_string())
}
