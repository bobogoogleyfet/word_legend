//! Precomputing rounds for the server.
//!
//! The Worker has to know a round's board and which words are valid on it, but
//! shipping the lexicon into a Worker is not practical. So rounds are vetted here,
//! where the dictionary lives -- full quality bar, every tile covered -- and the
//! results are uploaded as packs the Worker can serve and check against.

use crate::dictionary::Dictionary;
use crate::game::{self, SIZE};
use crate::themes::Themes;
use std::fmt::Write as _;

/// Boards per pack. One KV entry holds a pack, so this trades entry size against
/// how many entries an upload costs.
pub const PACK_SIZE: u64 = 100;

/// Build one round as JSON.
fn round_json(dict: &Dictionary, themes: &Themes, round: u64) -> String {
    // The seed is the round number here; in play the server keeps the mapping to
    // itself so nobody can work out a board before its round starts.
    let (grid, words, theme) = game::generate_board_seeded(dict, themes, round);

    let tiles: Vec<String> = (0..SIZE)
        .flat_map(|r| (0..SIZE).map(move |c| (r, c)))
        .map(|(r, c)| format!("\"{}\"", grid[r][c].letters))
        .collect();

    // Everything that scores: both tiers, since both are playable.
    let mut valid: Vec<&String> = words.common.iter().chain(words.obscure.iter()).collect();
    valid.sort();
    let words_json: Vec<String> = valid.iter().map(|w| format!("\"{w}\"")).collect();

    let theme_json = match &theme {
        Some(name) => format!("\"{name}\""),
        None => "null".to_string(),
    };

    let mut out = String::new();
    let _ = write!(
        out,
        "{{\"round\":{round},\"grid\":[{}],\"theme\":{theme_json},\"words\":[{}]}}",
        tiles.join(","),
        words_json.join(",")
    );
    out
}

/// Build `count` rounds starting at `first`, as one JSON array per pack.
/// Returns `(pack_index, json)` pairs.
pub fn build_packs(first: u64, count: u64) -> Vec<(u64, String)> {
    let dict = Dictionary::new();
    let themes = Themes::load();

    let mut packs = Vec::new();
    let mut round = first;
    let end = first + count;

    while round < end {
        let pack_index = round / PACK_SIZE;
        let pack_end = ((pack_index + 1) * PACK_SIZE).min(end);

        let rounds: Vec<String> = (round..pack_end)
            .map(|r| round_json(&dict, &themes, r))
            .collect();

        packs.push((pack_index, format!("[{}]", rounds.join(","))));
        round = pack_end;
    }
    packs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pack_holds_playable_rounds() {
        let packs = build_packs(0, 3);
        assert_eq!(packs.len(), 1);
        let (index, json) = &packs[0];
        assert_eq!(*index, 0);

        assert!(json.starts_with('['), "a pack is a JSON array");
        assert!(json.contains("\"round\":0"));
        assert!(json.contains("\"round\":2"));
        assert!(json.contains("\"grid\":["));
        assert!(json.contains("\"words\":["));
        // Sixteen tiles, quoted and comma separated.
        let grid_start = json.find("\"grid\":[").unwrap() + 8;
        let grid_end = json[grid_start..].find(']').unwrap() + grid_start;
        assert_eq!(json[grid_start..grid_end].split(',').count(), 16);
    }

    #[test]
    fn packs_split_on_the_pack_size() {
        let packs = build_packs(PACK_SIZE - 1, 2);
        assert_eq!(packs.len(), 2, "a run straddling a boundary makes two packs");
        assert_eq!(packs[0].0, 0);
        assert_eq!(packs[1].0, 1);
    }

    #[test]
    fn a_rounds_words_match_its_board() {
        // What the server validates against has to be what the client can find.
        let dict = Dictionary::new();
        let themes = Themes::load();
        let (grid, words, _) = game::generate_board_seeded(&dict, &themes, 5);
        let json = round_json(&dict, &themes, 5);

        for word in words.common.iter().take(20) {
            assert!(json.contains(&format!("\"{word}\"")), "{word} missing from the pack");
        }
        let letters: String = (0..SIZE)
            .flat_map(|r| (0..SIZE).map(move |c| (r, c)))
            .map(|(r, c)| format!("\"{}\"", grid[r][c].letters))
            .collect::<Vec<_>>()
            .join(",");
        assert!(json.contains(&letters), "grid in the pack does not match the board");
    }
}
