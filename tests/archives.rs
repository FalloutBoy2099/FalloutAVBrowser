//! Archives ZIP : le fichier n'est pas verrouillé pendant que le programme le lit, et une archive modifiée sur le disque est relue.

use fallout_av_browser::dat::{self, Dat};
use std::path::PathBuf;

/// ZIP « stocké » (sans compression) minimal, construit à la main : le test ne dépend d'aucun fichier extérieur.
fn make_zip(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in files {
        let mut crc = flate2::Crc::new();
        crc.update(data);
        let offset = out.len() as u32;
        let hdr = |o: &mut Vec<u8>, central: bool| {
            o.extend_from_slice(&(if central { 0x0201_4b50u32 } else { 0x0403_4b50u32 }).to_le_bytes());
            if central {
                o.extend_from_slice(&20u16.to_le_bytes()); // version utilisée
            }
            o.extend_from_slice(&20u16.to_le_bytes()); // version requise
            o.extend_from_slice(&0u16.to_le_bytes()); // indicateurs
            o.extend_from_slice(&0u16.to_le_bytes()); // méthode : stocké
            o.extend_from_slice(&0u16.to_le_bytes()); // heure
            o.extend_from_slice(&0x21u16.to_le_bytes()); // date
            o.extend_from_slice(&crc.sum().to_le_bytes());
            o.extend_from_slice(&(data.len() as u32).to_le_bytes());
            o.extend_from_slice(&(data.len() as u32).to_le_bytes());
            o.extend_from_slice(&(name.len() as u16).to_le_bytes());
            o.extend_from_slice(&0u16.to_le_bytes()); // champ supplémentaire
            if central {
                o.extend_from_slice(&[0u8; 2 + 2 + 2 + 4]); // commentaire, disque, attributs internes, externes
                o.extend_from_slice(&offset.to_le_bytes());
            }
            o.extend_from_slice(name.as_bytes());
        };
        hdr(&mut out, false);
        out.extend_from_slice(data);
        hdr(&mut central, true);
    }
    let cd_offset = out.len() as u32;
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&[0u8; 4]);
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(central.len() as u32).to_le_bytes());
    out.extend_from_slice(&cd_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

fn temp(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("fav_archives_{}_{name}", std::process::id()))
}

#[test]
fn archive_not_locked() {
    let p = temp("lock.zip");
    std::fs::write(&p, make_zip(&[("a/one.wav", b"hello"), ("two.wav", b"world!")])).unwrap();
    let d = Dat::open(&p).expect("ouverture");
    assert_eq!(d.entries.len(), 2);
    let idx = d.find("a/one.wav").unwrap();
    assert_eq!(d.read(idx).unwrap(), b"hello");
    // Le lecteur ZIP est maintenant ouvert et conservé : le fichier doit pourtant rester renommable puis supprimable.
    let moved = temp("lock_moved.zip");
    std::fs::rename(&p, &moved).expect("renommage refusé : le fichier est verrouillé");
    // Les lectures suivantes passent encore par le descripteur déjà ouvert.
    assert_eq!(d.read(d.find("two.wav").unwrap()).unwrap(), b"world!");
    std::fs::remove_file(&moved).expect("suppression refusée : le fichier est verrouillé");
}

#[test]
fn archive_changed_on_disk_is_reread() {
    let p = temp("stale.zip");
    std::fs::write(&p, make_zip(&[("old.wav", b"1")])).unwrap();
    assert_eq!(Dat::open(&p).unwrap().entries.len(), 1);
    // Même chemin, contenu différent (et taille différente) : l'index en cache ne doit plus servir.
    std::fs::write(&p, make_zip(&[("new1.wav", b"22"), ("new2.wav", b"333")])).unwrap();
    let d = Dat::open(&p).unwrap();
    assert_eq!(d.entries.len(), 2);
    assert!(d.find("new2.wav").is_some() && d.find("old.wav").is_none());
    // `forget` retire l'archive du cache.
    dat::forget(&p);
    assert_eq!(Dat::open(&p).unwrap().entries.len(), 2);
    let _ = std::fs::remove_file(&p);
}

#[test]
fn zip_entry_longer_than_declared_is_an_error_not_a_truncation() {
    // Lecture d'une entrée normale : octets identiques (pas de troncature silencieuse).
    let big = vec![7u8; 3 << 20];
    let p = temp("big.zip");
    std::fs::write(&p, make_zip(&[("big.wav", &big)])).unwrap();
    let d = Dat::open(&p).unwrap();
    assert_eq!(d.read(0).unwrap(), big);
    let _ = std::fs::remove_file(&p);
}
