//! Precomputing rounds for the server.
//!
//! The Worker has to know a round's board and which words are valid on it, but
//! shipping the lexicon into a Worker is not practical. So rounds are vetted here,
//! where the dictionary lives -- full quality bar, every tile covered -- and the
//! results are uploaded as packs the Worker can serve and check against.

use crate::dictionary::Dictionary;
use crate::game::SIZE;
use crate::rotation::{self, RoundKind};
use crate::themes::Themes;
use std::fmt::Write as _;

/// Boards per pack. One KV entry holds a pack, so this trades entry size against
/// how many entries an upload costs.
pub const PACK_SIZE: u64 = 100;

/// Build one round as JSON: its kind comes from the curated rotation, and the
/// board is built from the round number, so rebuilding a round gives the same one.
fn round_json(dict: &Dictionary, themes: &Themes, superwords: &[String], round: u64) -> String {
    let kind = rotation::kind_for_slot(round, superwords);
    let (grid, words, theme) = rotation::build_round(dict, themes, &kind, round);

    // A themed round that fell back to another theme, or to a plain board, says so:
    // the rotation is meant to be exact, so a fallback is worth knowing about.
    if let RoundKind::Themed(wanted) = &kind {
        if theme.as_deref() != Some(wanted.as_str()) {
            eprintln!("  round {round}: no {wanted} board found, used {}", theme.as_deref().unwrap_or("a plain board"));
        }
    }

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

/// Build `count` rounds starting at `first`, as one JSON array per pack, using
/// every core: themed boards are searched for, and a thousand of them one at a
/// time would take the better part of an hour.
/// Returns `(pack_index, json)` pairs.
pub fn build_packs(first: u64, count: u64) -> Vec<(u64, String)> {
    let end = first + count;
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4) as u64;
    let next = std::sync::atomic::AtomicU64::new(first);
    let built = std::sync::Mutex::new(std::collections::BTreeMap::new());

    std::thread::scope(|scope| {
        for _ in 0..threads.min(count.max(1)) {
            scope.spawn(|| {
                let dict = Dictionary::new();
                let themes = Themes::load();
                let superwords = rotation::superwords();
                loop {
                    let round = next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    if round >= end {
                        break;
                    }
                    let json = round_json(&dict, &themes, &superwords, round);
                    let mut built = built.lock().expect("no builder panicked");
                    built.insert(round, json);
                    let done = built.len() as u64;
                    if done % 50 == 0 || done == count {
                        eprintln!("  {done}/{count} rounds built");
                    }
                }
            });
        }
    });

    let built = built.into_inner().expect("no builder panicked");
    let mut packs = Vec::new();
    let mut round = first;
    while round < end {
        let pack_index = round / PACK_SIZE;
        let pack_end = ((pack_index + 1) * PACK_SIZE).min(end);
        let rounds: Vec<&str> = (round..pack_end).map(|r| built[&r].as_str()).collect();
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
        let superwords = rotation::superwords();
        let kind = rotation::kind_for_slot(5, &superwords);
        let (grid, words, _) = rotation::build_round(&dict, &themes, &kind, 5);
        let json = round_json(&dict, &themes, &superwords, 5);

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
