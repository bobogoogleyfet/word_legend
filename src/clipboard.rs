//! Reading the clipboard, for the Paste button on the recovery screen.
//!
//! Writing needs nothing from here: `ctx.copy_text` works on the desktop and in
//! the browser alike. Reading is another matter. egui only sees clipboard text as
//! a Ctrl+V into a focused field, which a phone has no key for, so a button has to
//! ask the platform itself: arboard on the desktop, `navigator.clipboard` in the
//! browser. The browser's answer is asynchronous and may need the player's
//! permission, so the result lands in a slot the UI checks each frame.

use std::sync::{Arc, Mutex};

/// Where a paste result arrives: `Ok(text)`, or `Err(reason)` if the platform
/// would not hand the clipboard over.
#[derive(Clone, Default)]
pub struct Paste(Arc<Mutex<Option<Result<String, String>>>>);

impl Paste {
    /// Take whatever has arrived since the last look.
    pub fn take(&self) -> Option<Result<String, String>> {
        self.0.lock().ok()?.take()
    }

    pub(crate) fn put(&self, result: Result<String, String>) {
        if let Ok(mut slot) = self.0.lock() {
            *slot = Some(result);
        }
    }

    /// Ask for the clipboard's text.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn request(&self) {
        let result = arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.get_text())
            .map_err(|e| e.to_string());
        self.put(result);
    }

    /// Ask for the clipboard's text.
    #[cfg(target_arch = "wasm32")]
    pub fn request(&self) {
        let Some(window) = web_sys::window() else {
            self.put(Err("no window".to_string()));
            return;
        };
        // The async clipboard API only exists on https (or localhost).
        if !window.is_secure_context() {
            self.put(Err("clipboard needs a secure page".to_string()));
            return;
        }
        let promise = window.navigator().clipboard().read_text();
        let slot = self.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let result = match wasm_bindgen_futures::JsFuture::from(promise).await {
                Ok(value) => value.as_string().ok_or_else(|| "clipboard held no text".to_string()),
                Err(_) => Err("clipboard access was refused".to_string()),
            };
            slot.put(result);
        });
    }
}
