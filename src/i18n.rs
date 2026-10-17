//! Internationalisation : 16 langues, fichiers `assets/lang/xx.txt` (clé=valeur) embarqués.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Lang {
    Fr,
    En,
    Es,
    It,
    Pt,
    Pl,
    Da,
    Fi,
    El,
    Hu,
    Nl,
    Ro,
    Sv,
    Cs,
    Zh,
    Ru,
}

impl Lang {
    pub const ALL: [Lang; 16] =
        [Lang::Fr, Lang::En, Lang::Es, Lang::It, Lang::Pt, Lang::Pl, Lang::Da, Lang::Fi, Lang::El, Lang::Hu, Lang::Nl, Lang::Ro, Lang::Sv, Lang::Cs, Lang::Zh, Lang::Ru];

    pub fn code(self) -> &'static str {
        match self {
            Lang::Fr => "fr",
            Lang::En => "en",
            Lang::Es => "es",
            Lang::It => "it",
            Lang::Pt => "pt",
            Lang::Pl => "pl",
            Lang::Da => "da",
            Lang::Fi => "fi",
            Lang::El => "el",
            Lang::Hu => "hu",
            Lang::Nl => "nl",
            Lang::Ro => "ro",
            Lang::Sv => "sv",
            Lang::Cs => "cs",
            Lang::Zh => "zh",
            Lang::Ru => "ru",
        }
    }

    pub fn native_name(self) -> &'static str {
        match self {
            Lang::Fr => "Français",
            Lang::En => "English",
            Lang::Es => "Español",
            Lang::It => "Italiano",
            Lang::Pt => "Português",
            Lang::Pl => "Polski",
            Lang::Da => "Dansk",
            Lang::Fi => "Suomi",
            Lang::El => "Ελληνικά",
            Lang::Hu => "Magyar",
            Lang::Nl => "Nederlands",
            Lang::Ro => "Română",
            Lang::Sv => "Svenska",
            Lang::Cs => "Čeština",
            Lang::Zh => "中文",
            Lang::Ru => "Русский",
        }
    }

    fn source(self) -> &'static str {
        match self {
            Lang::Fr => include_str!("../assets/lang/fr.txt"),
            Lang::En => include_str!("../assets/lang/en.txt"),
            Lang::Es => include_str!("../assets/lang/es.txt"),
            Lang::It => include_str!("../assets/lang/it.txt"),
            Lang::Pt => include_str!("../assets/lang/pt.txt"),
            Lang::Pl => include_str!("../assets/lang/pl.txt"),
            Lang::Da => include_str!("../assets/lang/da.txt"),
            Lang::Fi => include_str!("../assets/lang/fi.txt"),
            Lang::El => include_str!("../assets/lang/el.txt"),
            Lang::Hu => include_str!("../assets/lang/hu.txt"),
            Lang::Nl => include_str!("../assets/lang/nl.txt"),
            Lang::Ro => include_str!("../assets/lang/ro.txt"),
            Lang::Sv => include_str!("../assets/lang/sv.txt"),
            Lang::Cs => include_str!("../assets/lang/cs.txt"),
            Lang::Zh => include_str!("../assets/lang/zh.txt"),
            Lang::Ru => include_str!("../assets/lang/ru.txt"),
        }
    }

    /// Langue du système, anglais par défaut.
    pub fn detect() -> Lang {
        let loc = sys_locale::get_locale().unwrap_or_default().to_ascii_lowercase();
        let code = loc.split(['-', '_']).next().unwrap_or("");
        let code = if code == "nb" || code == "no" { "da" } else { code };
        Lang::ALL.iter().copied().find(|l| l.code() == code).unwrap_or(Lang::En)
    }

    fn index(self) -> usize {
        Lang::ALL.iter().position(|l| *l == self).unwrap()
    }
}

type Table = HashMap<&'static str, String>;

fn parse(src: &'static str) -> Table {
    let mut m = HashMap::new();
    for line in src.lines() {
        let line = line.trim_start_matches('\u{feff}');
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            m.insert(k.trim(), v.trim().replace("\\n", "\n"));
        }
    }
    m
}

fn table(l: Lang) -> &'static Table {
    static T: [OnceLock<Table>; 16] = [const { OnceLock::new() }; 16];
    T[l.index()].get_or_init(|| parse(l.source()))
}

static CURRENT: AtomicU8 = AtomicU8::new(1);

pub fn set_lang(l: Lang) {
    CURRENT.store(l.index() as u8, Ordering::Relaxed);
}

pub fn lang() -> Lang {
    Lang::ALL[CURRENT.load(Ordering::Relaxed) as usize % 16]
}

/// Traduit une clé ; repli sur l'anglais puis sur la clé elle-même. Une clé commençant par '@'
/// est un texte brut (retourné sans le '@').
pub fn tr(key: &str) -> String {
    if let Some(raw) = key.strip_prefix('@') {
        return raw.to_string();
    }
    if let Some(v) = table(lang()).get(key) {
        return v.clone();
    }
    if let Some(v) = table(Lang::En).get(key) {
        return v.clone();
    }
    key.to_string()
}

/// Traduction avec substitution de `{nom}`.
pub fn trf(key: &str, args: &[(&str, String)]) -> String {
    let mut s = tr(key);
    for (k, v) in args {
        s = s.replace(&format!("{{{k}}}"), v);
    }
    s
}

/// Clés présentes dans le fichier anglais (référence pour les tests de couverture).
pub fn reference_keys() -> Vec<&'static str> {
    table(Lang::En).keys().copied().collect()
}

pub fn has_key(l: Lang, key: &str) -> bool {
    table(l).contains_key(key)
}
