//! Le thread de décodage vidéo rend les images demandées, dans le bon ordre, y compris après un saut en arrière.
//!
//! Il lui faut une vidéo des jeux (non redistribuable) : indiquez-la dans `FAV_VIDEO`. Sans elle, le test ne fait rien.
//!
//!     FAV_VIDEO="C:\...\50000.bik" cargo test --test worker -- --nocapture

use fallout_av_browser::media::{self, ChannelMode};
use fallout_av_browser::video_worker::{Frame, VideoWorker};
use std::path::Path;
use std::time::{Duration, Instant};

fn wait_for(w: &VideoWorker, index: usize) -> Frame {
    let start = Instant::now();
    loop {
        if let Some(f) = w.latest()
            && f.index == index
        {
            return f;
        }
        assert!(start.elapsed() < Duration::from_secs(60), "image {index} non rendue après 60 s");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn worker() {
    let Ok(path) = std::env::var("FAV_VIDEO") else {
        eprintln!("FAV_VIDEO non défini : test ignoré");
        return;
    };
    let loaded = media::load(Path::new(&path), ChannelMode::Auto).expect("chargement");
    let video = loaded.video.expect("pas de vidéo");
    let n = video.frame_count();
    assert!(n > 20, "vidéo trop courte pour le test");
    let w = VideoWorker::new(video.clone(), eframe::egui::Context::default());

    // L'appelant ne doit jamais attendre : la demande revient tout de suite, même vers une image lointaine.
    let t = Instant::now();
    w.request(n - 1);
    assert!(t.elapsed() < Duration::from_millis(50), "la demande a bloqué");
    let last = wait_for(&w, n - 1);
    assert_eq!(last.rgba.len(), last.width * last.height * 4);

    // Retour en arrière lointain, puis demandes rapprochées : seule la dernière compte.
    w.request(3);
    let first = wait_for(&w, 3);
    assert_eq!((first.width, first.height), (video.width(), video.height()));
    w.request(10);
    w.request(15);
    w.request(17);
    wait_for(&w, 17);
    eprintln!("ok : {n} images, saut vers l'arrière et vers l'avant");
}
