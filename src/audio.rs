//! Sortie audio (cpal) : un flux permanent qui lit un tampon PCM partagé. La position de lecture sert d'horloge maîtresse
//! (la vidéo s'y cale).

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::{Arc, Mutex};

/// Nombre maximal d'échantillons (toutes pistes confondues) qu'un fichier peut produire : environ 25 minutes en stéréo 44,1 kHz.
/// Un fichier piégé ne doit pas pouvoir exiger des dizaines de gigaoctets de mémoire.
pub const MAX_SAMPLES: usize = 1 << 27;

/// Nombre maximal de pixels d'une image vidéo (4096 x 4096).
pub const MAX_PIXELS: usize = 4096 * 4096;

/// Audio PCM 16 bits entrelacé.
#[derive(Clone)]
pub struct Pcm {
    pub samples: Arc<Vec<i16>>,
    pub channels: u16,
    pub rate: u32,
}

impl Pcm {
    pub fn frames(&self) -> usize {
        self.samples.len() / self.channels.max(1) as usize
    }

    pub fn duration(&self) -> f64 {
        self.frames() as f64 / self.rate.max(1) as f64
    }
}

struct State {
    pcm: Option<Pcm>,
    /// Position en trames source (fractionnaire : rééchantillonnage linéaire).
    pos: f64,
    playing: bool,
    volume: f32,
    looping: bool,
    /// Vitesse de lecture (1.0 = normale) ; la hauteur du son change avec elle.
    speed: f64,
    /// Passe à vrai quand la lecture s'arrête d'elle-même en fin de piste.
    finished: bool,
}

pub struct Output {
    st: Arc<Mutex<State>>,
    _stream: Option<cpal::Stream>,
    pub device_error: Option<String>,
}

impl Output {
    pub fn new() -> Output {
        let st = Arc::new(Mutex::new(State { pcm: None, pos: 0.0, playing: false, volume: 0.8, looping: false, speed: 1.0, finished: false }));
        match Self::open(st.clone()) {
            Ok(s) => Output { st, _stream: Some(s), device_error: None },
            Err(e) => Output { st, _stream: None, device_error: Some(e) },
        }
    }

    fn open(st: Arc<Mutex<State>>) -> Result<cpal::Stream, String> {
        let host = cpal::default_host();
        let dev = host.default_output_device().ok_or("aucune sortie audio")?;
        let cfg = dev.default_output_config().map_err(|e| e.to_string())?;
        let fmt = cfg.sample_format();
        let sc: cpal::StreamConfig = cfg.into();
        let (rate, ch) = (sc.sample_rate.0 as f64, sc.channels as usize);
        let err = |e| eprintln!("audio : {e}");
        macro_rules! build {
            ($t:ty) => {{
                let st = st.clone();
                dev.build_output_stream(
                    &sc,
                    move |out: &mut [$t], _| {
                        let mut tmp = vec![0f32; out.len()];
                        fill(&st, &mut tmp, ch, rate);
                        for (o, v) in out.iter_mut().zip(tmp) {
                            *o = <$t as cpal::FromSample<f32>>::from_sample_(v);
                        }
                    },
                    err,
                    None,
                )
            }};
        }
        let stream = match fmt {
            cpal::SampleFormat::F32 => build!(f32),
            cpal::SampleFormat::I16 => build!(i16),
            cpal::SampleFormat::U16 => build!(u16),
            cpal::SampleFormat::I32 => build!(i32),
            other => return Err(format!("format audio non géré : {other:?}")),
        }
        .map_err(|e| e.to_string())?;
        stream.play().map_err(|e| e.to_string())?;
        Ok(stream)
    }

    /// Charge un tampon (sans démarrer). `None` : silence.
    pub fn set(&self, pcm: Option<Pcm>) {
        let mut s = self.st.lock().unwrap();
        s.pcm = pcm;
        s.pos = 0.0;
        s.playing = false;
        s.finished = false;
    }

    pub fn play(&self) {
        let mut s = self.st.lock().unwrap();
        if let Some(p) = &s.pcm
            && s.pos >= p.frames() as f64
        {
            s.pos = 0.0;
        }
        s.finished = false;
        s.playing = true;
    }

    pub fn pause(&self) {
        self.st.lock().unwrap().playing = false;
    }

    pub fn stop(&self) {
        let mut s = self.st.lock().unwrap();
        s.playing = false;
        s.pos = 0.0;
    }

    pub fn is_playing(&self) -> bool {
        self.st.lock().unwrap().playing
    }

    /// Vrai (une seule fois) quand la piste vient de se terminer.
    pub fn take_finished(&self) -> bool {
        std::mem::take(&mut self.st.lock().unwrap().finished)
    }

    /// Position en secondes.
    pub fn position(&self) -> f64 {
        let s = self.st.lock().unwrap();
        s.pcm.as_ref().map_or(0.0, |p| s.pos / p.rate.max(1) as f64)
    }

    pub fn seek(&self, secs: f64) {
        let mut s = self.st.lock().unwrap();
        if let Some(p) = &s.pcm {
            let max = p.frames() as f64;
            s.pos = (secs * p.rate as f64).clamp(0.0, max);
        }
    }

    pub fn set_volume(&self, v: f32) {
        self.st.lock().unwrap().volume = v.clamp(0.0, 1.0);
    }

    pub fn set_speed(&self, v: f32) {
        self.st.lock().unwrap().speed = v.clamp(0.2, 5.0) as f64;
    }

    pub fn set_looping(&self, l: bool) {
        self.st.lock().unwrap().looping = l;
    }
}

impl Default for Output {
    fn default() -> Self {
        Self::new()
    }
}

fn fill(st: &Mutex<State>, out: &mut [f32], ch: usize, dev_rate: f64) {
    let Ok(mut s) = st.lock() else { return };
    let (playing, vol, looping) = (s.playing, s.volume, s.looping);
    let Some(pcm) = s.pcm.clone() else { return };
    if !playing || ch == 0 {
        return;
    }
    let step = pcm.rate.max(1) as f64 / dev_rate * s.speed;
    let sc = pcm.channels.max(1) as usize;
    let frames = pcm.frames();
    let at = |f: usize, c: usize| -> f32 { pcm.samples.get(f * sc + c.min(sc - 1)).copied().unwrap_or(0) as f32 / 32768.0 };
    let mut pos = s.pos;
    for frame in out.chunks_mut(ch) {
        if pos >= frames as f64 {
            if looping && frames > 0 {
                pos -= frames as f64;
            } else {
                s.playing = false;
                s.finished = true;
                pos = frames as f64;
                break;
            }
        }
        let i = pos as usize;
        let t = (pos - i as f64) as f32;
        let j = (i + 1).min(frames.saturating_sub(1));
        for (c, o) in frame.iter_mut().enumerate() {
            // Mono : même signal partout ; stéréo : gauche / droite ; autres canaux : repli sur le dernier.
            let a = at(i, c);
            let b = at(j, c);
            *o = (a + (b - a) * t) * vol;
        }
        pos += step;
    }
    s.pos = pos;
}
