//! Robustesse : les lecteurs reçoivent des fichiers tronqués ou corrompus et ne doivent ni paniquer, ni boucler, ni exploser en mémoire.
//!
//! Les fichiers d'origine viennent des jeux (non redistribuables) : indiquez-les dans la variable `FAV_SAMPLES`
//! (chemins séparés par `;`). Sans elle, le test ne fait rien.
//!
//!     FAV_SAMPLES="C:\a.acm;C:\b.mve;C:\c.bik" cargo test --release --test robust -- --nocapture

use fallout_av_browser::{acm, bink, dat, mp3, mve, wav};
use std::path::PathBuf;
use std::sync::mpsc::channel;
use std::time::Duration;

/// Générateur pseudo-aléatoire déterministe (xorshift).
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

fn mutate(src: &[u8], rng: &mut Rng, case: usize) -> Vec<u8> {
    let mut d = src.to_vec();
    match case % 4 {
        // Tronqué.
        0 => d.truncate(rng.below(d.len())),
        // Quelques octets modifiés n'importe où.
        1 => {
            for _ in 0..1 + rng.below(8) {
                let i = rng.below(d.len());
                d[i] = rng.next() as u8;
            }
        }
        // Octets modifiés dans l'en-tête (tailles, nombres d'éléments).
        2 => {
            let head = d.len().min(256);
            for _ in 0..1 + rng.below(4) {
                let i = rng.below(head);
                d[i] = rng.next() as u8;
            }
        }
        // Un mot de 32 bits de l'en-tête remplacé par une valeur extrême.
        _ => {
            let head = d.len().min(256);
            if head >= 8 {
                let i = rng.below(head - 4);
                let v = [0u32, 1, 0x7FFF_FFFF, 0xFFFF_FFFF, 0x8000_0000, 0xFFFF][rng.below(6)];
                d[i..i + 4].copy_from_slice(&v.to_le_bytes());
            }
        }
    }
    d
}

/// Exécute `f` dans un thread : renvoie `Err` en cas de panique, de blocage (> `secs`) ou d'abandon.
fn guarded(secs: u64, f: impl FnOnce() + Send + 'static) -> Result<(), String> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
        let _ = tx.send(r.map_err(|e| e.downcast_ref::<String>().cloned().or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_else(|| "panique".into())));
    });
    match rx.recv_timeout(Duration::from_secs(secs)) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(msg)) => Err(format!("panique : {msg}")),
        Err(_) => Err(format!("aucune réponse après {secs} s")),
    }
}

fn decode_mve(d: Vec<u8>) {
    if let Ok(m) = mve::Mve::parse(d) {
        // Une taille d'image absurde ne doit pas provoquer d'allocation géante.
        if m.width * m.height > 64 << 20 {
            return;
        }
        let mut dec = mve::MveDecoder::new(&m);
        let mut n = 0;
        while dec.step(&m) && n < 400 {
            let _ = dec.rgba();
            n += 1;
        }
    }
}

fn decode_bink(d: Vec<u8>) {
    if let Ok(b) = bink::Bink::parse(d) {
        if b.width * b.height > 64 << 20 {
            return;
        }
        let mut dec = bink::BinkDecoder::new(&b);
        let mut n = 0;
        while dec.step(&b) && n < 150 {
            let _ = dec.rgba();
            n += 1;
        }
    }
}

fn open_dat(d: Vec<u8>, tag: String) {
    let p = std::env::temp_dir().join(format!("fav_robust_{tag}.dat"));
    std::fs::write(&p, &d).unwrap();
    if let Ok(a) = dat::Dat::open(&p) {
        for i in 0..a.entries.len().min(40) {
            let _ = a.read(i);
        }
    }
    let _ = std::fs::remove_file(&p);
}

#[test]
fn robust() {
    let Ok(list) = std::env::var("FAV_SAMPLES") else {
        eprintln!("FAV_SAMPLES non défini : test ignoré");
        return;
    };
    let cases: usize = std::env::var("FAV_CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(200);
    let mut failures: Vec<String> = Vec::new();
    for sample in list.split(';').filter(|s| !s.is_empty()) {
        let path = PathBuf::from(sample);
        let Ok(src) = std::fs::read(&path) else {
            eprintln!("illisible : {sample}");
            continue;
        };
        let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15 ^ src.len() as u64);
        let mut bad = 0;
        for case in 0..cases {
            let d = mutate(&src, &mut rng, case);
            let tag = format!("{}_{case}", std::process::id());
            let r = match ext.as_str() {
                "acm" => guarded(20, move || drop(acm::decode(&d, false))),
                "mve" => guarded(20, move || decode_mve(d)),
                "bik" => guarded(30, move || decode_bink(d)),
                "wav" => guarded(20, move || drop(wav::decode(&d))),
                "mp3" => guarded(20, move || drop(mp3::decode(&d))),
                _ => guarded(30, move || open_dat(d, tag)),
            };
            if let Err(e) = r {
                bad += 1;
                if bad <= 3 {
                    failures.push(format!("{} (cas {case}) : {e}", path.file_name().unwrap().to_string_lossy()));
                }
            }
        }
        eprintln!("{} : {cases} cas, {bad} échec(s)", path.file_name().unwrap().to_string_lossy());
    }
    for f in &failures {
        eprintln!("ÉCHEC {f}");
    }
    assert!(failures.is_empty(), "{} échec(s)", failures.len());
}
