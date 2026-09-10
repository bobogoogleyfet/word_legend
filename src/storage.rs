//! Save slots, backed by whatever the platform actually has: a dotfile in `$HOME`
//! on desktop, `localStorage` in the browser.
//!
//! The league ladder is only meaningful if it survives a restart, so this is not
//! optional on the web -- without it every reload would drop you back into a fresh
//! Bronze season.

/// Logical slot names, mapped to a real location by the backend below.
pub const SAVE: &str = "save";
/// Best score from before leagues existed, under the game's pre-rename name.
pub const LEGACY_BEST: &str = "legacy_best";
/// The account: a random id and a display name.
pub const IDENTITY: &str = "identity";

#[cfg(all(not(target_arch = "wasm32"), not(test)))]
mod backend {
    use std::fs;
    use std::path::PathBuf;

    fn path(slot: &str) -> PathBuf {
        let file = match slot {
            // The name the game shipped under before it became Word Legend.
            super::LEGACY_BEST => ".wordherd_best",
            super::IDENTITY => ".word_legend_account",
            _ => ".word_legend_save",
        };
        match std::env::var("HOME") {
            Ok(home) => PathBuf::from(home).join(file),
            Err(_) => PathBuf::from(file),
        }
    }

    pub fn read(slot: &str) -> Option<String> {
        fs::read_to_string(path(slot)).ok()
    }

    pub fn write(slot: &str, value: &str) {
        let _ = fs::write(path(slot), value);
    }
}

/// Tests run in this process against the real `$HOME`. Several of them play a
/// round to completion, which persists -- so without this stub `cargo test` would
/// overwrite the player's league ladder with fixture data. Reads are stubbed too,
/// or tests would inherit whatever season the player happens to be in.
#[cfg(all(not(target_arch = "wasm32"), test))]
mod backend {
    pub fn read(_slot: &str) -> Option<String> {
        None
    }

    pub fn write(_slot: &str, _value: &str) {}
}

#[cfg(target_arch = "wasm32")]
mod backend {
    fn local_storage() -> Option<web_sys::Storage> {
        // Private-browsing modes can refuse storage entirely; the game still plays,
        // it just forgets.
        web_sys::window()?.local_storage().ok().flatten()
    }

    fn key(slot: &str) -> String {
        format!("word_legend.{slot}")
    }

    pub fn read(slot: &str) -> Option<String> {
        local_storage()?.get_item(&key(slot)).ok().flatten()
    }

    pub fn write(slot: &str, value: &str) {
        if let Some(store) = local_storage() {
            let _ = store.set_item(&key(slot), value);
        }
    }
}

pub use backend::{read, write};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tests_never_touch_the_real_save() {
        write(SAVE, "league=5\nround=3\n");
        assert!(
            read(SAVE).is_none(),
            "test storage is not stubbed; cargo test would clobber the player's save"
        );
    }
}
