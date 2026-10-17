// Application console sur tous les systèmes : les options en ligne de commande (--info, --wav, --frame) fonctionnent dans
// un terminal. En mode fenêtre, la console est détachée sous Windows.

use fallout_av_browser::app::App;
use fallout_av_browser::{cli, winprefs::WinPrefs};
use std::path::PathBuf;
use std::sync::Arc;

/// Windows : la fenêtre ne doit pas garder (ni bloquer) le terminal qui l'a lancée.
#[cfg(all(windows, not(debug_assertions)))]
fn detach_console() {
    use std::os::windows::process::CommandExt;
    if std::env::var_os("FAV_DETACHED").is_none() {
        if let Ok(exe) = std::env::current_exe() {
            let spawned = std::process::Command::new(exe)
                .args(std::env::args_os().skip(1))
                .env("FAV_DETACHED", "1")
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .creation_flags(0x0000_0008)
                .spawn();
            if spawned.is_ok() {
                std::process::exit(0);
            }
        }
    }
    unsafe {
        windows_sys::Win32::System::Console::FreeConsole();
    }
}
#[cfg(not(all(windows, not(debug_assertions))))]
fn detach_console() {}

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if cli::is_cli(&args) {
        std::process::exit(cli::run(&args));
    }
    detach_console();
    let initial: Option<PathBuf> = args.first().map(PathBuf::from);
    let prefs = WinPrefs::load();
    let (rgba, width, height) = fallout_av_browser::icon::window_icon();
    let icon = eframe::egui::IconData { rgba, width, height };
    let mut vp = eframe::egui::ViewportBuilder::default().with_min_inner_size([620.0, 400.0]).with_title("FalloutAVBrowser").with_icon(Arc::new(icon)).with_drag_and_drop(true);
    vp = match prefs.size.filter(|_| prefs.remember_size) {
        Some(s) => vp.with_inner_size(s),
        None => vp.with_inner_size([1100.0, 700.0]),
    };
    if let (true, Some(p)) = (prefs.remember_pos, prefs.pos) {
        vp = vp.with_position(p);
    }
    let options = eframe::NativeOptions { viewport: vp, renderer: eframe::Renderer::Wgpu, persist_window: false, ..Default::default() };
    eframe::run_native("FalloutAVBrowser", options, Box::new(move |cc| Ok(Box::new(App::new(cc, initial.clone())))))
}
