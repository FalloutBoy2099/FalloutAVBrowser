//! Vidéos prises en charge (MVE, Bink) derrière une interface commune.

use crate::bink::{Bink, BinkDecoder};
use crate::mve::{Mve, MveDecoder};

pub enum Video {
    Mve(Mve),
    Bink(Bink),
}

impl Video {
    pub fn width(&self) -> usize {
        match self {
            Video::Mve(m) => m.width,
            Video::Bink(b) => b.width,
        }
    }

    pub fn height(&self) -> usize {
        match self {
            Video::Mve(m) => m.height,
            Video::Bink(b) => b.height,
        }
    }

    pub fn frame_count(&self) -> usize {
        match self {
            Video::Mve(m) => m.frame_count(),
            Video::Bink(b) => b.frame_count(),
        }
    }

    pub fn fps(&self) -> f64 {
        match self {
            Video::Mve(m) => m.fps(),
            Video::Bink(b) => b.fps(),
        }
    }

    pub fn duration(&self) -> f64 {
        self.frame_count() as f64 / self.fps()
    }

    /// Dernière image décodable sans rien avant elle (image clé), à ou avant `target`.
    pub fn key_before(&self, target: usize) -> usize {
        match self {
            Video::Mve(_) => 0,
            Video::Bink(b) => b.key_before(target),
        }
    }
}

pub enum VideoDecoder {
    Mve(MveDecoder),
    Bink(BinkDecoder),
}

impl VideoDecoder {
    pub fn new(v: &Video) -> VideoDecoder {
        match v {
            Video::Mve(m) => VideoDecoder::Mve(MveDecoder::new(m)),
            Video::Bink(b) => VideoDecoder::Bink(BinkDecoder::new(b)),
        }
    }

    /// Index de la prochaine image à décoder.
    pub fn position(&self) -> usize {
        match self {
            VideoDecoder::Mve(d) => d.position(),
            VideoDecoder::Bink(d) => d.position(),
        }
    }

    /// Repart de l'image clé `frame` (0 pour MVE).
    pub fn seek_to(&mut self, frame: usize) {
        match self {
            VideoDecoder::Mve(d) => d.reset(),
            VideoDecoder::Bink(d) => d.seek_to(frame),
        }
    }

    /// Décode l'image suivante ; `false` à la fin.
    pub fn step(&mut self, v: &Video) -> bool {
        match (self, v) {
            (VideoDecoder::Mve(d), Video::Mve(m)) => d.step(m),
            (VideoDecoder::Bink(d), Video::Bink(b)) => d.step(b),
            _ => false,
        }
    }

    pub fn rgba(&self) -> Vec<u8> {
        match self {
            VideoDecoder::Mve(d) => d.rgba(),
            VideoDecoder::Bink(d) => d.rgba(),
        }
    }
}
