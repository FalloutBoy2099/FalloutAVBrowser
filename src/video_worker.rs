//! Décodage vidéo hors du thread de l'interface.
//!
//! Les vidéos Bink d'Arcanum n'ont qu'une image clé (la première) : atteindre une image éloignée oblige à décoder tout ce qui
//! précède, ce qui prendrait plusieurs secondes et gèlerait la fenêtre. Un thread dédié décode vers l'image demandée ; une
//! nouvelle demande interrompt la précédente, et seule l'image la plus récente est rendue à l'interface.

use crate::video::{Video, VideoDecoder};
use eframe::egui;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};

/// Une image décodée, prête à devenir une texture.
pub struct Frame {
    /// Numéro de l'image réellement décodée (peut précéder la demande si la vidéo se termine).
    pub index: usize,
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

pub struct VideoWorker {
    tx: Sender<usize>,
    rx: Receiver<Frame>,
}

impl VideoWorker {
    pub fn new(video: Arc<Video>, ctx: egui::Context) -> VideoWorker {
        let (req_tx, req_rx) = channel::<usize>();
        let (frame_tx, frame_rx) = channel::<Frame>();
        std::thread::spawn(move || {
            // Un décodeur qui panique (flux corrompu) arrête le thread : la dernière image reste affichée.
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(&video, &ctx, &req_rx, &frame_tx)));
        });
        VideoWorker { tx: req_tx, rx: frame_rx }
    }

    /// Demande l'image `target` (la précédente demande, si elle n'est pas finie, est abandonnée).
    pub fn request(&self, target: usize) {
        let _ = self.tx.send(target);
    }

    /// Image la plus récente rendue depuis le dernier appel.
    pub fn latest(&self) -> Option<Frame> {
        let mut last = None;
        while let Ok(f) = self.rx.try_recv() {
            last = Some(f);
        }
        last
    }
}

fn work(video: &Video, ctx: &egui::Context, rx: &Receiver<usize>, tx: &Sender<Frame>) {
    let mut dec = VideoDecoder::new(video);
    let mut last_sent: Option<usize> = None;
    let Ok(mut target) = rx.recv() else { return };
    loop {
        // Ne garde que la demande la plus récente.
        loop {
            match rx.try_recv() {
                Ok(t) => target = t,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }
        // Retour en arrière (ou saut au-delà de la prochaine image clé) : on repart d'une image clé.
        let key = video.key_before(target);
        if dec.position() > target + 1 || key > dec.position() {
            dec.seek_to(key);
        }
        let mut changed = false;
        let mut restart = false;
        while dec.position() <= target {
            if !dec.step(video) {
                break;
            }
            changed = true;
            // Toutes les 8 images, on regarde si une demande plus récente est arrivée.
            if dec.position().is_multiple_of(8) {
                match rx.try_recv() {
                    Ok(t) => {
                        target = t;
                        restart = true;
                        break;
                    }
                    Err(TryRecvError::Empty) => {}
                    Err(TryRecvError::Disconnected) => return,
                }
            }
        }
        if restart {
            continue;
        }
        let index = dec.position().saturating_sub(1);
        if changed || last_sent != Some(index) {
            let frame = Frame { index, width: video.width(), height: video.height(), rgba: dec.rgba() };
            if tx.send(frame).is_err() {
                return;
            }
            last_sent = Some(index);
            ctx.request_repaint();
        }
        // Attend la demande suivante.
        match rx.recv() {
            Ok(t) => target = t,
            Err(_) => return,
        }
    }
}
