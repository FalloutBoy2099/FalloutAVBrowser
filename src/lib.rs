//! FalloutAVBrowser : lecteur audio / vidéo pour les formats Fallout 1/2 et Arcanum (ACM, MVE, WAV).

#![allow(clippy::needless_range_loop, clippy::too_many_arguments)]

pub mod acm;
pub mod app;
pub mod assoc;
pub mod audio;
pub mod bink;
pub mod bink_data;
pub mod cli;
pub mod convert;
pub mod dat;
pub mod dialog;
pub mod fileops;
pub mod i18n;
pub mod icon;
pub mod hash;
pub mod icons;
pub mod media;
pub mod model;
pub mod png;
pub mod props;
pub mod mp3;
pub mod mve;
pub mod opener;
pub mod tree;
pub mod video;
pub mod wav;
pub mod winprefs;
