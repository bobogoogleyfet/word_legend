mod dictionary;
mod game;
mod identity;
mod league;
mod storage;
mod themes;
mod ui;

pub use ui::WordLegendApp;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

/// Browser entry point. `index.html` imports this and hands it the canvas.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub async fn start(canvas: web_sys::HtmlCanvasElement) -> Result<(), JsValue> {
    // Panics otherwise land in the console as "unreachable executed", which says
    // nothing about what actually went wrong.
    console_error_panic_hook::set_once();

    eframe::WebRunner::new()
        .start(
            canvas,
            eframe::WebOptions::default(),
            Box::new(|_cc| Ok(Box::new(WordLegendApp::new()))),
        )
        .await
}
