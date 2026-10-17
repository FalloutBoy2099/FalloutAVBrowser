//! Opérations sur le fichier affiché : copie, déplacement et renommage. Un `.PAL` du même nom (la palette d'un FRM) suit
//! le fichier, pour que l'image garde ses couleurs.

use crate::dat;
use anyhow::{Result, anyhow, bail};
use std::path::{Path, PathBuf};

/// Caractères interdits dans un nom de fichier (ceux de Windows ; `/` l'est partout).
const FORBIDDEN: &[char] = &['\\', '/', ':', '*', '?', '"', '<', '>', '|'];

/// Un nom de fichier est utilisable : non vide, sans séparateur ni caractère interdit ou de contrôle, sans point ni espace final.
pub fn validate_name(name: &str) -> Result<()> {
    let n = name.trim();
    if n.is_empty() || n == "." || n == ".." {
        bail!("nom de fichier vide");
    }
    if n.chars().any(|c| FORBIDDEN.contains(&c) || c.is_control()) {
        bail!("le nom contient un caractère interdit ({})", FORBIDDEN.iter().collect::<String>());
    }
    if n.ends_with('.') || name.ends_with(' ') {
        bail!("le nom ne peut pas finir par un point ou un espace");
    }
    Ok(())
}

/// Chemin libre dans `dir` pour `name` : `name` lui-même, sinon `nom (2).ext`, `nom (3).ext`…
pub fn free_path(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    (2..).map(|n| dir.join(format!("{stem} ({n}){ext}"))).find(|p| !p.exists()).unwrap()
}

/// Le `.PAL` du même nom que `path`, s'il existe sur le disque.
fn sibling_pal(path: &Path) -> Option<PathBuf> {
    ["pal", "PAL", "Pal"].iter().map(|e| path.with_extension(e)).find(|p| p.is_file())
}

fn file_name(path: &Path) -> Result<String> {
    path.file_name().map(|n| n.to_string_lossy().to_string()).ok_or_else(|| anyhow!("{} : nom de fichier invalide", path.display()))
}

/// Copie un fichier du disque ou d'une archive `.DAT` dans `dir` sans rien écraser (un nom pris reçoit « (2) »).
/// Renvoie le chemin créé.
pub fn copy_to(src: &Path, dir: &Path) -> Result<PathBuf> {
    if !dir.is_dir() {
        bail!("{} : dossier introuvable", dir.display());
    }
    let dest = free_path(dir, &file_name(src)?);
    if dat::is_virtual(src) {
        std::fs::write(&dest, dat::read_any(src)?).map_err(|e| anyhow!("{} : {e}", dest.display()))?;
        return Ok(dest);
    }
    std::fs::copy(src, &dest).map_err(|e| anyhow!("{} : {e}", dest.display()))?;
    if let Some(pal) = sibling_pal(src) {
        // La palette suit l'image sous le même nom, même si l'image a dû être renommée.
        let _ = std::fs::copy(&pal, dest.with_extension(pal.extension().unwrap_or_default()));
    }
    Ok(dest)
}

/// Déplace un fichier du disque dans `dir` (copie puis suppression quand le simple renommage est impossible, par exemple
/// d'un disque à un autre). Les fichiers d'une archive ne peuvent pas être déplacés.
pub fn move_to(src: &Path, dir: &Path) -> Result<PathBuf> {
    if dat::is_virtual(src) {
        bail!("un fichier d'une archive ne peut pas être déplacé");
    }
    if dir.canonicalize().ok() == src.parent().and_then(|p| p.canonicalize().ok()) {
        bail!("le fichier est déjà dans ce dossier");
    }
    let dest = copy_to(src, dir)?;
    std::fs::remove_file(src).map_err(|e| anyhow!("{} : {e}", src.display()))?;
    if let Some(pal) = sibling_pal(src) {
        let _ = std::fs::remove_file(pal);
    }
    Ok(dest)
}

/// Renomme un fichier du disque dans son dossier (`new_name` avec son extension). Refuse un nom déjà pris, sauf s'il ne
/// change que la casse du même fichier.
pub fn rename(src: &Path, new_name: &str) -> Result<PathBuf> {
    if dat::is_virtual(src) {
        bail!("un fichier d'une archive ne peut pas être renommé");
    }
    let new_name = new_name.trim();
    validate_name(new_name)?;
    let dest = src.with_file_name(new_name);
    if dest != src && dest.exists() && dest.canonicalize().ok() != src.canonicalize().ok() {
        bail!("un fichier nommé « {new_name} » existe déjà");
    }
    std::fs::rename(src, &dest).map_err(|e| anyhow!("{} : {e}", dest.display()))?;
    if let Some(pal) = sibling_pal(src) {
        let target = dest.with_extension(pal.extension().unwrap_or_default());
        if !target.exists() {
            let _ = std::fs::rename(pal, target);
        }
    }
    Ok(dest)
}
