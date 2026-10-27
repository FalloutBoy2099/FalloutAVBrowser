//! ffmpeg externe : recherche de l'exécutable, liste de ses encodeurs et lancement d'un processus nourri par son entrée standard.
//! Il sert aux conversions MP3 / OGG / FLAC et vidéo ; il n'est pas embarqué.

use crate::i18n::{tr, trf};
use anyhow::{Result, anyhow, bail};
use std::collections::HashSet;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::thread::JoinHandle;

fn exe_name() -> &'static str {
    if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" }
}

/// ffmpeg à utiliser : celui choisi par l'utilisateur (s'il existe), sinon celui à côté de l'exécutable, sinon celui du PATH.
pub fn find(custom: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = custom.filter(|p| p.is_file()) {
        return Some(p.to_path_buf());
    }
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        let p = dir.join(exe_name());
        if p.is_file() {
            return Some(p);
        }
    }
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths).map(|d| d.join(exe_name())).find(|p| p.is_file())
}

fn command(exe: &Path) -> Command {
    let mut c = Command::new(exe);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    c
}

/// Noms des encodeurs (audio et vidéo) que cet ffmpeg sait utiliser.
pub fn encoders(exe: &Path) -> HashSet<String> {
    let mut set = HashSet::new();
    let Ok(out) = command(exe).args(["-hide_banner", "-encoders"]).stdin(Stdio::null()).stderr(Stdio::null()).output() else { return set };
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let mut it = line.split_whitespace();
        if let (Some(flags), Some(name)) = (it.next(), it.next())
            && flags.len() == 6
            && flags.starts_with(['V', 'A'])
        {
            set.insert(name.to_string());
        }
    }
    set
}

/// Premier des encodeurs `names` que ffmpeg possède.
pub fn pick(enc: &HashSet<String>, names: &[&'static str]) -> Result<&'static str> {
    names.iter().copied().find(|n| enc.contains(*n)).ok_or_else(|| anyhow!(trf("c.no_encoder", &[("name", names[0].to_string())])))
}

/// Processus ffmpeg en cours : on lui écrit les données sur son entrée standard, ses messages d'erreur sont recueillis.
pub struct Proc {
    child: Child,
    stdin: Option<ChildStdin>,
    err: Option<JoinHandle<String>>,
}

pub fn spawn(exe: &Path, args: &[String]) -> Result<Proc> {
    let mut child = command(exe).args(args).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::piped()).spawn().map_err(|e| anyhow!("{} : {e}", exe.display()))?;
    let stdin = child.stdin.take();
    let mut stderr = child.stderr.take();
    let err = std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(e) = stderr.as_mut() {
            let _ = e.read_to_string(&mut s);
        }
        s
    });
    Ok(Proc { child, stdin, err: Some(err) })
}

impl Proc {
    pub fn write(&mut self, data: &[u8]) -> std::io::Result<()> {
        match self.stdin.as_mut() {
            Some(s) => s.write_all(data),
            None => Err(std::io::ErrorKind::BrokenPipe.into()),
        }
    }

    /// Ferme l'entrée, attend la fin de ffmpeg et en rapporte l'échec éventuel (avec son message).
    pub fn finish(mut self) -> Result<()> {
        drop(self.stdin.take());
        let status = self.child.wait()?;
        let err = self.err.take().and_then(|h| h.join().ok()).unwrap_or_default();
        if !status.success() {
            let msg = err.trim();
            bail!("{}", trf("c.ffmpeg_failed", &[("err", if msg.is_empty() { status.to_string() } else { msg.to_string() })]));
        }
        Ok(())
    }

    /// Interrompt ffmpeg.
    pub fn abort(mut self) {
        drop(self.stdin.take());
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Message quand ffmpeg est introuvable.
pub fn missing() -> anyhow::Error {
    anyhow!(tr("c.ffmpeg_missing"))
}
