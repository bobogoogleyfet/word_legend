// The web build is driven by `lib.rs`; this binary is the desktop one.

#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), eframe::Error> {
    use eframe::egui;
    use word_legend_app::WordLegendApp;

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1000.0, 780.0])
            .with_min_inner_size([820.0, 700.0])
            .with_title("Word Legend"),
        ..Default::default()
    };

    eframe::run_native(
        "Word Legend",
        options,
        Box::new(|_cc| Ok(Box::new(WordLegendApp::new()))),
    )
}

/// `cargo build --target wasm32-...` still builds this target; the real web entry
/// point is `start()` in the library.
#[cfg(target_arch = "wasm32")]
fn main() {}
