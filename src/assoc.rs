//! Association des fichiers Fallout / Fallout Tactics / Arcanum à l'application (menu Aide).
//!
//! * Windows : clés `HKEY_CURRENT_USER\Software\Classes` (aucun droit administrateur). Si l'utilisateur a déjà choisi un
//!   autre programme par défaut pour une extension, Windows conserve ce choix : il faut alors le changer dans
//!   Paramètres → Applications par défaut.
//! * Linux : types MIME et fichier `.desktop` dans `~/.local/share`, puis `xdg-mime default`.
//! * macOS : une association demande un bundle d'application (`Info.plist`), pas disponible pour l'exécutable seul.

use std::path::Path;

/// Une extension de fichier et sa description (libellé du type de fichier).
#[derive(Clone, Copy, Debug)]
pub struct FileType {
    pub ext: &'static str,
    pub description: &'static str,
}

pub const AUDIO: [FileType; 3] = [
    FileType { ext: "acm", description: "Interplay ACM audio" },
    FileType { ext: "wav", description: "WAV audio" },
    FileType { ext: "mp3", description: "MP3 audio" },
];
pub const VIDEO: [FileType; 2] = [FileType { ext: "mve", description: "Interplay MVE video" }, FileType { ext: "bik", description: "Bink video" }];

/// Toutes les extensions gérées (utile pour déclarer les types MIME d'un seul coup).
pub fn all() -> Vec<FileType> {
    AUDIO.iter().chain(VIDEO.iter()).copied().collect()
}

/// Texte « .FRM, .RIX » pour les messages.
pub fn list(types: &[FileType]) -> String {
    types.iter().map(|t| format!(".{}", t.ext.to_uppercase())).collect::<Vec<_>>().join(", ")
}

/// L'association est-elle disponible sur ce système ?
pub fn is_supported() -> bool {
    cfg!(any(windows, all(unix, not(target_os = "macos"))))
}

/// Associe les extensions données à l'exécutable en cours.
pub fn associate(types: &[FileType]) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    imp::associate(&exe, types)
}

// ----------------------------------------------------------------------------- types MIME (Linux, testable partout)

/// Type MIME propre à l'application pour une extension (`application/x-fallout-frm`).
pub fn mime_type(ext: &str) -> String {
    format!("application/x-fallout-{}", ext.to_lowercase())
}

/// Fichier de définition des types MIME (`~/.local/share/mime/packages/`) pour toutes les extensions.
pub fn mime_xml() -> String {
    let mut s = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<mime-info xmlns=\"http://www.freedesktop.org/standards/shared-mime-info\">\n");
    for t in all() {
        s += &format!("  <mime-type type=\"{}\">\n    <comment>{}</comment>\n    <glob pattern=\"*.{}\"/>\n  </mime-type>\n", mime_type(t.ext), t.description, t.ext);
    }
    s + "</mime-info>\n"
}

/// Fichier `.desktop` de l'application, déclarant tous les types MIME ci-dessus.
pub fn desktop_entry(exe: &Path) -> String {
    // Dans Exec, le chemin est entre guillemets ; `\`, `"`, `` ` `` et `$` y sont protégés, et `%` doublé.
    let mut path = String::new();
    for c in exe.to_string_lossy().chars() {
        match c {
            // Protégé une première fois pour Exec, puis une seconde pour la syntaxe des valeurs d'un fichier .desktop.
            '\\' => path.push_str(r"\\\\"),
            '"' | '`' | '$' => {
                path.push_str(r"\\");
                path.push(c);
            }
            '%' => path.push_str("%%"),
            c => path.push(c),
        }
    }
    let mimes: String = all().iter().map(|t| format!("{};", mime_type(t.ext))).collect();
    format!(
        "[Desktop Entry]\nType=Application\nName=Fallout Image Browser\nComment=Image viewer and converter for Fallout, Fallout Tactics and Arcanum files\nExec=\"{path}\" %f\nTerminal=false\nCategories=AudioVideo;Player;\nMimeType={mimes}\n"
    )
}

// ----------------------------------------------------------------------------- Windows

#[cfg(windows)]
mod imp {
    use super::*;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::System::Registry::{HKEY, HKEY_CURRENT_USER, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey, RegCreateKeyExW, RegSetValueExW};
    use windows_sys::Win32::UI::Shell::{SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify};

    fn wide(s: &str) -> Vec<u16> {
        std::ffi::OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
    }

    /// Écrit une valeur texte (`name` = None : valeur par défaut de la clé) sous `HKEY_CURRENT_USER\key`.
    fn set(key: &str, name: Option<&str>, value: &str) -> Result<(), String> {
        let k = wide(key);
        let mut h: HKEY = std::ptr::null_mut();
        // SAFETY : pointeurs valides vers des tampons terminés par zéro qui vivent pendant l'appel.
        unsafe {
            let r = RegCreateKeyExW(HKEY_CURRENT_USER, k.as_ptr(), 0, std::ptr::null(), REG_OPTION_NON_VOLATILE, KEY_WRITE, std::ptr::null(), &mut h, std::ptr::null_mut());
            if r != 0 {
                return Err(format!("registre : {key} (code {r})"));
            }
            let n = name.map(wide);
            let v = wide(value);
            let r = RegSetValueExW(h, n.as_ref().map_or(std::ptr::null(), |n| n.as_ptr()), 0, REG_SZ, v.as_ptr() as *const u8, (v.len() * 2) as u32);
            RegCloseKey(h);
            if r != 0 {
                return Err(format!("registre : {key} (code {r})"));
            }
        }
        Ok(())
    }

    /// Enregistre les types sous `HKEY_CURRENT_USER\<root>` (`Software\Classes` en production ; un autre dossier pour les tests).
    pub fn register_in(root: &str, exe: &Path, types: &[FileType]) -> Result<(), String> {
        let exe = exe.display().to_string();
        for t in types {
            let prog = format!("FalloutAVBrowser.{}", t.ext.to_uppercase());
            set(&format!(r"{root}\.{}", t.ext), None, &prog)?;
            set(&format!(r"{root}\.{}\OpenWithProgids", t.ext), Some(&prog), "")?;
            set(&format!(r"{root}\{prog}"), None, t.description)?;
            set(&format!(r"{root}\{prog}\DefaultIcon"), None, &format!("\"{exe}\",0"))?;
            set(&format!(r"{root}\{prog}\shell\open\command"), None, &format!("\"{exe}\" \"%1\""))?;
        }
        Ok(())
    }

    pub fn associate(exe: &Path, types: &[FileType]) -> Result<(), String> {
        register_in(r"Software\Classes", exe, types)?;
        // Prévient l'Explorateur : icônes et programmes associés sont rafraîchis.
        unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED as i32, SHCNF_IDLIST, std::ptr::null(), std::ptr::null()) };
        Ok(())
    }
}

#[cfg(windows)]
pub use imp::register_in;

// ----------------------------------------------------------------------------- Linux

#[cfg(all(unix, not(target_os = "macos")))]
mod imp {
    use super::*;
    use std::path::PathBuf;
    use std::process::Command;

    /// `$XDG_DATA_HOME`, à défaut `~/.local/share`.
    fn data_home() -> Result<PathBuf, String> {
        if let Some(d) = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).filter(|p| p.is_absolute()) {
            return Ok(d);
        }
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")).ok_or_else(|| "HOME n'est pas défini".to_string())
    }

    /// Lance un utilitaire facultatif : son absence n'est pas une erreur.
    fn optional(cmd: &str, args: &[&Path]) {
        let _ = Command::new(cmd).args(args).status();
    }

    pub fn associate(exe: &Path, types: &[FileType]) -> Result<(), String> {
        let data = data_home()?;
        let (mime_dir, apps_dir) = (data.join("mime"), data.join("applications"));
        for d in [mime_dir.join("packages"), apps_dir.clone()] {
            std::fs::create_dir_all(&d).map_err(|e| format!("{} : {e}", d.display()))?;
        }
        let xml = mime_dir.join("packages").join("fallout-av-browser.xml");
        let desktop = apps_dir.join("fallout-av-browser.desktop");
        std::fs::write(&xml, mime_xml()).map_err(|e| format!("{} : {e}", xml.display()))?;
        std::fs::write(&desktop, desktop_entry(exe)).map_err(|e| format!("{} : {e}", desktop.display()))?;
        optional("update-mime-database", &[&mime_dir]);
        optional("update-desktop-database", &[&apps_dir]);
        let mut cmd = Command::new("xdg-mime");
        cmd.arg("default").arg("fallout-av-browser.desktop");
        for t in types {
            cmd.arg(mime_type(t.ext));
        }
        match cmd.status() {
            Ok(s) if s.success() => Ok(()),
            Ok(s) => Err(format!("xdg-mime : {s}")),
            Err(e) => Err(format!("xdg-mime : {e}")),
        }
    }
}

// ----------------------------------------------------------------------------- macOS et autres

#[cfg(not(any(windows, all(unix, not(target_os = "macos")))))]
mod imp {
    use super::*;

    pub fn associate(_exe: &Path, _types: &[FileType]) -> Result<(), String> {
        Err("non disponible sur ce système".to_string())
    }
}
