// Windows : intègre l'icône (assets/icon.ico) et les informations de version dans l'exécutable.
//
// Le fichier .rc désigne l'icône par un chemin relatif. Selon l'outil (rc.exe sous Windows, llvm-rc en compilation
// croisée depuis Linux avec cargo-xwin), ce chemin n'est pas résolu au même endroit : on écrit donc dans le dossier de
// sortie une copie du .rc où il est remplacé par un chemin absolu.
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=assets/app.rc");
    println!("cargo:rerun-if-changed=assets/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
        let icon = manifest.join("assets").join("icon.ico").to_string_lossy().replace('\\', "/");
        let rc = std::fs::read_to_string(manifest.join("assets").join("app.rc")).expect("assets/app.rc").replace("assets/icon.ico", &icon);
        let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR")).join("app.rc");
        std::fs::write(&out, rc).expect("écriture de app.rc");
        embed_resource::compile(&out, embed_resource::NONE).manifest_optional().unwrap();
    }
}
