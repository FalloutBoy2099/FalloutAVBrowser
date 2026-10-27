//! Options en ligne de commande (sans fenêtre) :
//!
//! ```text
//! --info  <fichier>                   propriétés
//! --ls    <archive.dat> [filtre]      contenu d'une archive
//! --wav   <fichier> <sortie.wav>      audio converti en WAV
//! --frame <fichier.mve> <n> <sortie.png>   image n° n
//! ```

use crate::media::{self, ChannelMode};
use crate::video::VideoDecoder;
use std::path::Path;

pub fn run(args: &[String]) -> i32 {
    match run_inner(args) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

fn run_inner(args: &[String]) -> anyhow::Result<()> {
    let a = |i: usize| args.get(i).map(String::as_str).ok_or_else(|| anyhow::anyhow!("arguments manquants (voir cli.rs)"));
    match args[0].as_str() {
        "--info" => {
            let l = media::load(Path::new(a(1)?), ChannelMode::Auto)?;
            println!("{:?}, {} octets", l.kind, l.size);
            if let Some(p) = &l.pcm {
                println!("audio : {} Hz, {} canaux, {:.2} s", p.rate, p.channels, p.duration());
            }
            if let Some(m) = &l.video {
                println!("vidéo : {}x{}, {:.2} ips, {} images", m.width(), m.height(), m.fps(), m.frame_count());
            }
            if let Some(w) = &l.warning {
                println!("avertissement : {w}");
            }
        }
        "--ls" => {
            let d = crate::dat::Dat::open(Path::new(a(1)?))?;
            let filter = args.get(2).map(|s| s.to_lowercase()).unwrap_or_default();
            for e in d.entries.iter().filter(|e| e.path.to_lowercase().contains(&filter)) {
                println!("{:>10}  {}", e.size, e.path);
            }
            println!("{} entrées ({:?})", d.entries.len(), d.kind);
        }
        "--convert" => {
            // --convert <fichier> <wav16|wav8|f32|raw|mp3|ogg|flac|video|frame|seq|orig> <dossier> [fréquence] [canaux] [normaliser 0/1]
            use crate::convert::{ConvSettings, Job, Source, Target};
            use std::sync::atomic::{AtomicBool, AtomicUsize};
            let path = Path::new(a(1)?);
            let l = media::load(path, ChannelMode::Auto)?;
            let target = match a(2)? {
                "wav16" => Target::Wav16,
                "wav8" => Target::Wav8,
                "f32" => Target::WavF32,
                "raw" => Target::Raw16,
                "mp3" => Target::Mp3,
                "ogg" => Target::Ogg,
                "flac" => Target::Flac,
                "video" => Target::Video,
                "frame" => Target::FramePng,
                "seq" => Target::Sequence,
                "orig" => Target::Original,
                t => anyhow::bail!("cible inconnue : {t}"),
            };
            let num = |i: usize| args.get(i).and_then(|v| v.parse::<u32>().ok()).unwrap_or(0);
            let s = ConvSettings { target, rate: num(4), channels: num(5) as u8, normalize: num(6) == 1, ..Default::default() };
            let source = Source { path: path.to_path_buf(), pcm: l.pcm, video: l.video, frame: 0 };
            let name = path.file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let job = Job { source, s, out_dir: a(3)?.into(), name, start: 0.0, end: 0.0, frame_first: 1, frame_last: 5, frame_step: 2 };
            let files = crate::convert::run(job, &AtomicUsize::new(0), &AtomicUsize::new(0), &AtomicBool::new(false))?;
            for f in files {
                println!("{}", f.display());
            }
        }
        "--wav" => {
            let l = media::load(Path::new(a(1)?), ChannelMode::Auto)?;
            let p = l.pcm.ok_or_else(|| anyhow::anyhow!("pas d'audio"))?;
            std::fs::write(a(2)?, crate::wav::encode(&p.samples, p.channels, p.rate))?;
        }
        "--frame" => {
            let l = media::load(Path::new(a(1)?), ChannelMode::Auto)?;
            let m = l.video.ok_or_else(|| anyhow::anyhow!("pas de vidéo"))?;
            let n: usize = a(2)?.parse()?;
            let mut d = VideoDecoder::new(&m);
            d.seek_to(m.key_before(n));
            while d.position() <= n && d.step(&m) {}
            std::fs::write(a(3)?, crate::png::encode_rgba(m.width() as u32, m.height() as u32, &d.rgba()))?;
        }
        _ => anyhow::bail!("option inconnue : {}", args[0]),
    }
    Ok(())
}

/// Vrai si les arguments demandent une opération en ligne de commande (et non l'ouverture d'un fichier).
pub fn is_cli(args: &[String]) -> bool {
    args.first().is_some_and(|a| a.starts_with("--") || a == "/?" || a == "-h")
}
