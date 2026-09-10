//! Who you are, without knowing anything about you.
//!
//! There is no email and no password, so there is nothing personal to store and
//! nothing to leak. An account is a random 80-bit id generated on this device; the
//! display name beside it on a leaderboard is whatever you choose. The id is the
//! only way back into an account, which is why the game shows it as a code and
//! tells you to keep it.

use crate::storage;
use rand::RngCore;

/// Crockford base32: no I, L, O or U, so a handwritten code cannot be misread and
/// no four-letter word can appear in one.
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
const ID_BYTES: usize = 10; // 80 bits -> 16 characters

pub const MIN_NAME: usize = 3;
pub const MAX_NAME: usize = 16;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    /// The account itself: 16 characters, never shown to other players.
    pub id: String,
    /// What other players see. Uniqueness is the server's to enforce.
    pub name: String,
}

impl Identity {
    /// A brand new account.
    pub fn generate() -> Self {
        let mut bytes = [0u8; ID_BYTES];
        rand::thread_rng().fill_bytes(&mut bytes);
        Identity { id: encode(&bytes), name: String::new() }
    }

    pub fn load() -> Option<Self> {
        let text = storage::read(storage::IDENTITY)?;
        let mut id = String::new();
        let mut name = String::new();
        for line in text.lines() {
            if let Some((key, value)) = line.split_once('=') {
                match key {
                    "id" => id = value.trim().to_string(),
                    "name" => name = value.trim().to_string(),
                    _ => {}
                }
            }
        }
        (!id.is_empty()).then_some(Identity { id, name })
    }

    pub fn store(&self) {
        storage::write(storage::IDENTITY, &format!("id={}\nname={}\n", self.id, self.name));
    }

    /// The id in groups of four, which is how it is shown and typed.
    pub fn recovery_code(&self) -> String {
        self.id
            .as_bytes()
            .chunks(4)
            .map(|c| String::from_utf8_lossy(c).to_string())
            .collect::<Vec<_>>()
            .join("-")
    }

    /// Read a code back, however the player typed it: spacing, dashes and case are
    /// all forgiven, and the digits people confuse are folded together.
    pub fn from_recovery(code: &str) -> Option<Self> {
        let cleaned: String = code
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .map(|c| match c.to_ascii_uppercase() {
                'I' | 'L' => '1',
                'O' => '0',
                'U' => 'V',
                other => other,
            })
            .collect();

        if cleaned.len() != ID_BYTES * 8 / 5 {
            return None;
        }
        if !cleaned.bytes().all(|b| ALPHABET.contains(&b)) {
            return None;
        }
        Some(Identity { id: cleaned, name: String::new() })
    }
}

/// Why a name was refused, so the screen can say so rather than just going red.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NameError {
    TooShort,
    TooLong,
    BadCharacter,
}

impl NameError {
    pub fn message(self) -> String {
        match self {
            NameError::TooShort => format!("At least {MIN_NAME} characters"),
            NameError::TooLong => format!("At most {MAX_NAME} characters"),
            NameError::BadCharacter => "Letters, numbers, - and _ only".to_string(),
        }
    }
}

/// Local checks only. Whether the name is *taken* is a question only the server
/// can answer, and it answers it when the name is claimed.
pub fn check_name(name: &str) -> Result<String, NameError> {
    let trimmed = name.trim();
    if trimmed.chars().count() < MIN_NAME {
        return Err(NameError::TooShort);
    }
    if trimmed.chars().count() > MAX_NAME {
        return Err(NameError::TooLong);
    }
    if !trimmed.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err(NameError::BadCharacter);
    }
    Ok(trimmed.to_string())
}

fn encode(bytes: &[u8]) -> String {
    let mut out = String::new();
    let (mut acc, mut bits) = (0u16, 0u32);
    for byte in bytes {
        acc = (acc << 8) | *byte as u16;
        bits += 8;
        while bits >= 5 {
            let idx = ((acc >> (bits - 5)) & 0x1f) as usize;
            out.push(ALPHABET[idx] as char);
            bits -= 5;
        }
    }
    if bits > 0 {
        let idx = ((acc << (5 - bits)) & 0x1f) as usize;
        out.push(ALPHABET[idx] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn generated_ids_are_unique_and_well_formed() {
        let ids: HashSet<String> = (0..2_000).map(|_| Identity::generate().id).collect();
        assert_eq!(ids.len(), 2_000, "ids collided");
        for id in &ids {
            assert_eq!(id.len(), 16);
            assert!(id.bytes().all(|b| ALPHABET.contains(&b)), "{id} has a stray character");
        }
    }

    #[test]
    fn a_code_survives_being_written_down_and_typed_back() {
        let me = Identity::generate();
        let code = me.recovery_code();
        assert_eq!(code.len(), 19, "expected four groups of four: {code}");

        for variant in [
            code.clone(),
            code.to_lowercase(),
            code.replace('-', " "),
            code.replace('-', ""),
            format!("  {code}  "),
        ] {
            let back = Identity::from_recovery(&variant).expect("should parse: {variant}");
            assert_eq!(back.id, me.id, "{variant} did not round-trip");
        }
    }

    #[test]
    fn confusable_characters_are_folded() {
        // Someone reading a code aloud will say "oh" for zero and "eye" for one.
        let me = Identity { id: "0123456789ABCDEF".to_string(), name: String::new() };
        let typed = "O123-456789-ABCDEF".replace('-', "");
        assert_eq!(Identity::from_recovery(&typed).unwrap().id, me.id);
    }

    #[test]
    fn a_wrong_length_code_is_refused() {
        assert!(Identity::from_recovery("").is_none());
        assert!(Identity::from_recovery("ABCD-EFGH").is_none());
        assert!(Identity::from_recovery("0123456789ABCDEF0").is_none());
    }

    #[test]
    fn names_are_checked_before_they_are_claimed() {
        assert_eq!(check_name("  tanner  ").unwrap(), "tanner");
        assert_eq!(check_name("word_legend-1").unwrap(), "word_legend-1");
        assert_eq!(check_name("ab"), Err(NameError::TooShort));
        assert_eq!(check_name(&"x".repeat(17)), Err(NameError::TooLong));
        assert_eq!(check_name("hey there"), Err(NameError::BadCharacter));
        assert_eq!(check_name("drop table"), Err(NameError::BadCharacter));
    }
}
