//! What kind of board each round gets, and building it.
//!
//! The mix is curated rather than rolled. Every fifth round is a superword board,
//! and the four between are themed. Themes rotate evenly and never back to back.
//! Left to chance, a one-in-five board can go twenty rounds without showing up
//! and then come twice in a row; scheduled, it is always the fifth.
//!
//! Each month favours its season: Halloween in October, Christmas in December,
//! and so on. When a month is given, the second round of every five is that
//! month's theme -- a fifth of all rounds, several times any other theme, and
//! always five rounds apart -- and the regular themes rotate through the rest.
//!
//! - A **superword board** is built around one word that fills all sixteen tiles.
//!   The word is laid along a path through every tile, so the whole board is that
//!   word, and it is the best-playing of many such layouts.
//! - A **themed board** holds at least [`THEME_WORDS_REQUIRED`] words from its
//!   category (plurals included) and still clears the usual quality bar. That many
//!   cannot be planted; the board is searched for, starting from a few planted
//!   words and changing letters until it gets there.

use crate::dictionary::Dictionary;
use crate::game::{
    board_is_good, collect_theme_words, fill_grid, generate_board_local, plant_word, solve_board,
    worst_tile, BoardWords, Cell, LONG_WORD_LEN, MIN_LONG_SOLUTIONS, MIN_SOLUTIONS,
    MIN_TILE_COVERAGE, SIZE,
};
use crate::themes::{Theme, Themes};
use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::{Rng, SeedableRng};

pub type Grid = [[Cell; SIZE]; SIZE];

/// A themed board must hold at least this many words from its category.
pub const THEME_WORDS_REQUIRED: usize = 20;

/// Every this-many-th round is a superword board.
pub const SUPERWORD_EVERY: u64 = 5;

/// Themes that can reliably reach [`THEME_WORDS_REQUIRED`] on one board, in the
/// order they rotate. Measured by searching each theme's best board: these reach
/// 22 to 49 theme words; the rest top out below 20 and stay out of the rotation
/// until their word lists grow (see `rotation_themes_can_reach_the_bar`).
pub const ROTATION_THEMES: [&str; 10] = [
    "Animals", "Food", "Nature", "Body", "Music", "Science", "Math", "History", "Verbs", "Adjectives",
];

/// The theme each month favours, January first. Each is in `themes.txt` and, like
/// the rotation themes, measured to reach [`THEME_WORDS_REQUIRED`] on a board.
pub const SEASONAL_THEMES: [&str; 12] = [
    "Winter",
    "Valentine's Day",
    "St. Patrick's Day",
    "Easter",
    "Spring",
    "Summer",
    "Fourth of July",
    "School",
    "Fall",
    "Halloween",
    "Thanksgiving",
    "Christmas",
];

/// The theme a month (1 = January) favours.
pub fn season_for_month(month: u32) -> Option<&'static str> {
    SEASONAL_THEMES.get((month as usize).checked_sub(1)?).copied()
}

/// Where in each block of five the month's seasonal theme falls: the second round,
/// as far from the superword round at the block's end as it can be.
const SEASONAL_POSITION: u64 = 1;

/// Search steps a themed board gets before the search restarts from a new plant.
const THEME_SEARCH_STEPS: usize = 30_000;
/// Restarts before giving up on a theme for this round.
const THEME_SEARCH_RESTARTS: u64 = 4;
/// Layouts tried for a superword board; the best-playing one is kept.
const SUPERWORD_LAYOUTS: usize = 150;

const EMBEDDED_SUPERWORDS: &str = include_str!("../superwords.txt");

/// The curated superwords, in file order.
pub fn superwords() -> Vec<String> {
    EMBEDDED_SUPERWORDS
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_ascii_lowercase)
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RoundKind {
    Superword(String),
    Themed(String),
}

// --- the schedule ------------------------------------------------------------

/// A stable 64-bit mix, so the schedule is the same on every machine and build.
fn mix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}


/// The board kind for a round slot, favouring `season`'s theme when one is given.
pub fn kind_for_slot(slot: u64, superwords: &[String], season: Option<&str>) -> RoundKind {
    let block = slot / SUPERWORD_EVERY;
    let within = slot % SUPERWORD_EVERY;

    // The last round of each block of five: rounds 5, 10, 15, counting from one.
    if within == SUPERWORD_EVERY - 1 {
        return RoundKind::Superword(superword_for_block(block, superwords));
    }

    let Some(season) = season else {
        // How many themed rounds came before this one: four per whole block, plus
        // the themed rounds earlier in this block.
        let themed_before = block * (SUPERWORD_EVERY - 1) + within;
        return RoundKind::Themed(theme_at(themed_before).to_string());
    };

    if within == SEASONAL_POSITION {
        return RoundKind::Themed(season.to_string());
    }
    // The regular rotation carries on through the other three.
    let themed_before = block * (SUPERWORD_EVERY - 2) + within - u64::from(within > SEASONAL_POSITION);
    RoundKind::Themed(theme_at(themed_before).to_string())
}

/// Superwords are dealt through a fixed shuffle of the list, so every word is used
/// once before any repeats.
fn superword_for_block(block: u64, superwords: &[String]) -> String {
    let n = superwords.len() as u64;
    let cycle = block / n;
    let mut order: Vec<usize> = (0..superwords.len()).collect();
    order.shuffle(&mut StdRng::seed_from_u64(mix(cycle ^ 0x574F_5244)));
    superwords[order[(block % n) as usize]].clone()
}

/// The themes are dealt in shuffled passes through the rotation. Each pass is its
/// own shuffle, and one that would open with the theme the last pass ended on
/// starts with its second theme instead.
fn theme_at(index: u64) -> &'static str {
    let n = ROTATION_THEMES.len() as u64;
    let pass = |p: u64| {
        let mut order: Vec<&'static str> = ROTATION_THEMES.to_vec();
        order.shuffle(&mut StdRng::seed_from_u64(mix(p ^ 0x5448_454D_45)));
        order
    };
    let p = index / n;
    let mut order = pass(p);
    if p > 0 && order[0] == pass(p - 1)[ROTATION_THEMES.len() - 1] {
        order.swap(0, 1);
    }
    order[(index % n) as usize]
}

// --- building boards ---------------------------------------------------------

/// Build the board for a round kind from a seed. A themed board that cannot be
/// found in the search budget falls back to the next theme in the rotation, and
/// failing every theme, to a plain board -- a round always gets a board.
pub fn build_round(
    dict: &Dictionary,
    themes: &Themes,
    kind: &RoundKind,
    seed: u64,
) -> (Grid, BoardWords, Option<String>) {
    let mut rng = StdRng::seed_from_u64(mix(seed));
    match kind {
        RoundKind::Superword(word) => {
            if let Some((grid, words)) = build_superword_board(dict, word, &mut rng) {
                return (grid, words, None);
            }
        }
        RoundKind::Themed(name) => {
            // A seasonal theme is tried first; if its board cannot be found, the
            // round falls back through the regular rotation like any other.
            if !ROTATION_THEMES.contains(&name.as_str()) {
                if let Some(theme) = themes.list.iter().find(|t| &t.name == name) {
                    if let Some((grid, words)) = build_theme_board(dict, theme, &mut rng) {
                        return (grid, words, Some(theme.name.clone()));
                    }
                }
            }
            let start = ROTATION_THEMES.iter().position(|t| t == name).unwrap_or(0);
            for offset in 0..ROTATION_THEMES.len() {
                let name = ROTATION_THEMES[(start + offset) % ROTATION_THEMES.len()];
                let Some(theme) = themes.list.iter().find(|t| t.name == name) else { continue };
                if let Some((grid, words)) = build_theme_board(dict, theme, &mut rng) {
                    return (grid, words, Some(theme.name.clone()));
                }
            }
        }
    }
    let (grid, words, _) = generate_board_local(dict, &mut rng);
    (grid, words, None)
}

/// A word split into board tiles: one letter each, with "qu" on a single tile.
pub fn word_tiles(word: &str) -> Option<Vec<&'static str>> {
    const LETTERS: [&str; 26] = [
        "a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "m", "n", "o", "p", "q", "r",
        "s", "t", "u", "v", "w", "x", "y", "z",
    ];
    let bytes = word.as_bytes();
    let mut tiles = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if !b.is_ascii_lowercase() {
            return None;
        }
        if b == b'q' && bytes.get(i + 1) == Some(&b'u') {
            tiles.push("qu");
            i += 2;
        } else {
            tiles.push(LETTERS[(b - b'a') as usize]);
            i += 1;
        }
    }
    Some(tiles)
}

/// Lay a sixteen-tile word along random paths through every tile, and keep the
/// layout that plays best: one clearing the quality bar, with the most words.
pub fn build_superword_board(dict: &Dictionary, word: &str, rng: &mut StdRng) -> Option<(Grid, BoardWords)> {
    let tiles = word_tiles(word)?;
    if tiles.len() != SIZE * SIZE {
        return None;
    }

    let mut best: Option<(Grid, BoardWords, (bool, usize))> = None;
    for _ in 0..SUPERWORD_LAYOUTS {
        let path = tour(rng);
        let mut grid: Grid = [[Cell { letters: "e" }; SIZE]; SIZE];
        for (i, (row, col)) in path.iter().enumerate() {
            grid[*row][*col].letters = tiles[i];
        }
        let words = solve_board(dict, &grid);
        let rank = (board_is_good(&words), words.common.len());
        if best.as_ref().is_none_or(|(_, _, r)| rank > *r) {
            best = Some((grid, words, rank));
        }
    }
    best.map(|(grid, words, _)| (grid, words))
}

/// A random path through all sixteen tiles, stepping to a touching tile each time.
fn tour(rng: &mut StdRng) -> Vec<(usize, usize)> {
    fn extend(path: &mut Vec<(usize, usize)>, used: &mut u16, rng: &mut StdRng) -> bool {
        if path.len() == SIZE * SIZE {
            return true;
        }
        let (row, col) = *path.last().expect("path is never empty");
        let mut next: Vec<(usize, usize)> = Vec::with_capacity(8);
        for dr in -1i32..=1 {
            for dc in -1i32..=1 {
                let (r, c) = (row as i32 + dr, col as i32 + dc);
                if (dr, dc) != (0, 0) && (0..SIZE as i32).contains(&r) && (0..SIZE as i32).contains(&c) {
                    let bit = 1u16 << (r as usize * SIZE + c as usize);
                    if *used & bit == 0 {
                        next.push((r as usize, c as usize));
                    }
                }
            }
        }
        next.shuffle(rng);
        for step in next {
            let bit = 1u16 << (step.0 * SIZE + step.1);
            *used |= bit;
            path.push(step);
            if extend(path, used, rng) {
                return true;
            }
            path.pop();
            *used &= !bit;
        }
        false
    }

    loop {
        let start = (rng.gen_range(0..SIZE), rng.gen_range(0..SIZE));
        let mut path = vec![start];
        let mut used = 1u16 << (start.0 * SIZE + start.1);
        if extend(&mut path, &mut used, rng) {
            return path;
        }
    }
}

/// How good a candidate themed board is, and whether it is good enough.
struct Scored {
    score: f64,
    words: BoardWords,
    done: bool,
}

fn score_theme_board(dict: &Dictionary, theme: &Theme, grid: &Grid) -> Scored {
    let mut words = solve_board(dict, grid);
    words.theme = collect_theme_words(&words, theme);
    let themed = words.theme.len();
    let long = words.common.iter().filter(|w| w.len() >= LONG_WORD_LEN).count();

    // Theme words dominate until the bar is met; the quality bar's three parts
    // pull the board toward playable the whole way.
    let score = 100.0 * themed.min(THEME_WORDS_REQUIRED) as f64
        + 5.0 * themed.saturating_sub(THEME_WORDS_REQUIRED) as f64
        + words.common.len().min(MIN_SOLUTIONS) as f64
        + 15.0 * long.min(MIN_LONG_SOLUTIONS) as f64
        + 30.0 * worst_tile(&words).min(MIN_TILE_COVERAGE) as f64;
    let done = themed >= THEME_WORDS_REQUIRED && board_is_good(&words);
    Scored { score, words, done }
}

/// Letters weighted roughly by English frequency, for mutations.
const WEIGHTED: &[(&str, u32)] = &[
    ("a", 82), ("b", 15), ("c", 28), ("d", 43), ("e", 127), ("f", 22), ("g", 20), ("h", 61),
    ("i", 70), ("j", 2), ("k", 8), ("l", 40), ("m", 24), ("n", 67), ("o", 75), ("p", 19),
    ("r", 60), ("s", 63), ("t", 91), ("u", 28), ("v", 10), ("w", 24), ("x", 2), ("y", 20), ("z", 1),
];

fn weighted_letter(rng: &mut StdRng) -> &'static str {
    let total: u32 = WEIGHTED.iter().map(|(_, w)| w).sum();
    let mut pick = rng.gen_range(0..total);
    for (letter, weight) in WEIGHTED {
        if pick < *weight {
            return letter;
        }
        pick -= weight;
    }
    "e"
}

/// Search for a board holding enough of a theme's words, by simulated annealing:
/// change a letter or swap two, keep changes that help, and early on sometimes
/// keep ones that do not, so the search does not stall on a merely decent board.
pub fn build_theme_board(dict: &Dictionary, theme: &Theme, rng: &mut StdRng) -> Option<(Grid, BoardWords)> {
    // Letters that appear in the theme's words, for mutations that lean on-theme.
    let theme_letters: Vec<&'static str> = theme
        .plantable
        .iter()
        .filter_map(|w| word_tiles(w))
        .flatten()
        .collect();

    for _ in 0..THEME_SEARCH_RESTARTS {
        // Start from a handful of planted theme words rather than noise.
        let mut cells: [[Option<&'static str>; SIZE]; SIZE] = Default::default();
        let mut candidates = theme.plantable.clone();
        candidates.shuffle(rng);
        let mut planted = 0;
        for word in &candidates {
            if planted >= 6 {
                break;
            }
            if plant_word(&mut cells, word, rng) {
                planted += 1;
            }
        }
        let mut grid = fill_grid(cells, rng);
        let mut current = score_theme_board(dict, theme, &grid);
        if current.done {
            return Some((grid, current.words));
        }

        for step in 0..THEME_SEARCH_STEPS {
            let progress = step as f64 / THEME_SEARCH_STEPS as f64;
            let temperature = 120.0 * (1.0 - progress) + 2.0;

            let mut next = grid;
            let at = (rng.gen_range(0..SIZE), rng.gen_range(0..SIZE));
            match rng.gen_range(0..10) {
                0..=3 => next[at.0][at.1].letters = weighted_letter(rng),
                4..=6 => {
                    if let Some(letter) = theme_letters.choose(rng) {
                        next[at.0][at.1].letters = letter;
                    }
                }
                _ => {
                    let other = (rng.gen_range(0..SIZE), rng.gen_range(0..SIZE));
                    let tile = next[at.0][at.1];
                    next[at.0][at.1] = next[other.0][other.1];
                    next[other.0][other.1] = tile;
                }
            }

            let candidate = score_theme_board(dict, theme, &next);
            if candidate.done {
                return Some((next, candidate.words));
            }
            let delta = candidate.score - current.score;
            if delta >= 0.0 || rng.gen_bool((delta / temperature).exp().clamp(0.0, 1.0)) {
                grid = next;
                current = candidate;
            }
        }
    }
    None
}

// --- solo play ---------------------------------------------------------------

/// Board kinds for solo play, when there is no server to serve rounds.
///
/// Solo keeps a superword board every fifth round. Its other rounds are plain:
/// a themed board takes seconds of searching, which on a phone would freeze the
/// game at the start of every round.
pub struct SoloRotation {
    played: u64,
    superwords: Vec<String>,
}

impl SoloRotation {
    pub fn new(_rng: &mut impl Rng) -> Self {
        SoloRotation { played: 0, superwords: superwords() }
    }

    /// The next solo board.
    pub fn next_board(&mut self, dict: &Dictionary, rng: &mut StdRng) -> (Grid, BoardWords, Option<String>) {
        self.played += 1;

        if self.played % SUPERWORD_EVERY == 0 {
            if let Some(word) = self.superwords.choose(rng).cloned() {
                if let Some((grid, words)) = build_superword_board(dict, &word, rng) {
                    return (grid, words, None);
                }
            }
        }
        generate_board_local(dict, rng)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_superword_fills_the_board_and_is_a_common_word() {
        let dict = Dictionary::new();
        let words = superwords();
        assert!(words.len() >= 100, "only {} superwords: fewer than one per 1000 rounds' blocks", words.len());
        let mut seen = std::collections::HashSet::new();
        for word in &words {
            assert_eq!(word_tiles(word).map(|t| t.len()), Some(SIZE * SIZE), "{word} does not fill 16 tiles");
            assert!(dict.contains(word), "{word} is not accepted");
            assert!(seen.insert(word.clone()), "{word} is listed twice");
        }
    }

    #[test]
    fn every_fifth_round_is_a_superword_board() {
        let words = superwords();
        for slot in 0..3_000u64 {
            let superword = matches!(kind_for_slot(slot, &words, None), RoundKind::Superword(_));
            assert_eq!(superword, slot % 5 == 4, "slot {slot}: superword board {superword}");
        }
        // The packs repeat every thousand rounds; five divides it, so the pattern
        // carries across the wrap.
        assert_eq!(1_000 % SUPERWORD_EVERY, 0);
    }

    #[test]
    fn solo_play_serves_a_superword_board_every_fifth_round() {
        let dict = Dictionary::new();
        let mut rng = StdRng::seed_from_u64(1);
        let mut solo = SoloRotation::new(&mut rng);
        let supers: Vec<bool> = (0..10)
            .map(|_| {
                let (_, words, _) = solo.next_board(&dict, &mut rng);
                words.common.iter().any(|w| word_tiles(w).is_some_and(|t| t.len() == SIZE * SIZE))
            })
            .collect();
        assert_eq!(supers, [false, false, false, false, true, false, false, false, false, true]);
    }

    #[test]
    fn superwords_do_not_repeat_until_the_list_runs_out() {
        let words = superwords();
        let dealt: Vec<String> = (0..words.len() as u64).map(|b| superword_for_block(b, &words)).collect();
        let distinct: std::collections::HashSet<&String> = dealt.iter().collect();
        assert_eq!(distinct.len(), words.len());
    }

    #[test]
    fn themes_rotate_evenly_and_never_back_to_back() {
        let words = superwords();
        let themed: Vec<String> = (0..2_000u64)
            .filter_map(|slot| match kind_for_slot(slot, &words, None) {
                RoundKind::Themed(t) => Some(t),
                RoundKind::Superword(_) => None,
            })
            .collect();
        assert_eq!(themed.len(), 1_600);
        for pair in themed.windows(2) {
            assert_ne!(pair[0], pair[1], "the same theme twice in a row");
        }
        for name in ROTATION_THEMES {
            let count = themed.iter().filter(|t| *t == name).count();
            assert_eq!(count, 160, "{name} came up {count} times in 1600 themed rounds");
        }
    }

    #[test]
    fn a_superword_board_spells_its_word_across_every_tile() {
        let dict = Dictionary::new();
        let mut rng = StdRng::seed_from_u64(3);
        let (grid, words) = build_superword_board(&dict, "characterization", &mut rng).expect("a board");
        assert!(words.common.iter().any(|w| w == "characterization"), "the superword is not findable");
        assert!(board_is_good(&words), "the superword board does not clear the quality bar");
        let letters: usize = grid.iter().flatten().map(|c| c.letters.len()).sum();
        assert_eq!(letters, 16);
    }

    #[test]
    fn a_themed_board_holds_twenty_theme_words() {
        let dict = Dictionary::new();
        let themes = Themes::load();
        let theme = themes.list.iter().find(|t| t.name == "Verbs").expect("Verbs");
        let mut rng = StdRng::seed_from_u64(11);
        let (_, words) = build_theme_board(&dict, theme, &mut rng).expect("no Verbs board found");
        assert!(words.theme.len() >= THEME_WORDS_REQUIRED, "only {} theme words", words.theme.len());
        assert!(board_is_good(&words));
        for word in &words.theme {
            assert!(theme.all.contains(word));
            assert!(words.common.contains(word) || words.obscure.contains(word), "{word} is not on the board");
        }
    }

    #[test]
    fn a_round_is_the_same_board_every_time_it_is_built() {
        let dict = Dictionary::new();
        let themes = Themes::load();
        let kind = RoundKind::Superword("misunderstanding".into());
        let (a, _, _) = build_round(&dict, &themes, &kind, 77);
        let (b, _, _) = build_round(&dict, &themes, &kind, 77);
        let letters = |g: &Grid| g.iter().flatten().map(|c| c.letters).collect::<String>();
        assert_eq!(letters(&a), letters(&b));
    }

    #[test]
    fn every_rotation_and_seasonal_theme_exists() {
        let themes = Themes::load();
        for name in ROTATION_THEMES.iter().chain(SEASONAL_THEMES.iter()) {
            assert!(themes.list.iter().any(|t| t.name == *name), "{name} is scheduled but not in themes.txt");
        }
        for name in SEASONAL_THEMES {
            assert!(!ROTATION_THEMES.contains(&name), "{name} is seasonal; it must not also rotate year-round");
        }
    }

    #[test]
    fn each_month_favours_its_season() {
        assert_eq!(season_for_month(9), Some("Fall"));
        assert_eq!(season_for_month(10), Some("Halloween"));
        assert_eq!(season_for_month(11), Some("Thanksgiving"));
        assert_eq!(season_for_month(12), Some("Christmas"));
        assert_eq!(season_for_month(1), Some("Winter"));
        assert_eq!(season_for_month(2), Some("Valentine's Day"));
        assert_eq!(season_for_month(3), Some("St. Patrick's Day"));
        assert_eq!(season_for_month(4), Some("Easter"));
        assert_eq!(season_for_month(5), Some("Spring"));
        assert_eq!(season_for_month(6), Some("Summer"));
        assert_eq!(season_for_month(7), Some("Fourth of July"));
        assert_eq!(season_for_month(8), Some("School"));
        assert_eq!(season_for_month(0), None);
        assert_eq!(season_for_month(13), None);

        let words = superwords();
        let kinds: Vec<RoundKind> = (0..2_000u64).map(|slot| kind_for_slot(slot, &words, Some("Halloween"))).collect();
        let themed: Vec<&str> = kinds
            .iter()
            .filter_map(|k| match k {
                RoundKind::Themed(t) => Some(t.as_str()),
                RoundKind::Superword(_) => None,
            })
            .collect();
        let halloween = themed.iter().filter(|t| **t == "Halloween").count();
        assert_eq!(halloween, 400, "the season should be one round in five");
        // Several times as often as any regular theme, which share the other 1,200.
        for name in ROTATION_THEMES {
            let count = themed.iter().filter(|t| **t == name).count();
            assert_eq!(count, 120, "{name} came up {count} times");
            assert!(halloween >= 3 * count);
        }
        // Never back to back, and superwords still every fifth round.
        for (slot, pair) in kinds.windows(2).enumerate() {
            assert_ne!(pair[0], pair[1], "rounds {slot} and {} repeat", slot + 1);
        }
        for (slot, kind) in kinds.iter().enumerate() {
            assert_eq!(matches!(kind, RoundKind::Superword(_)), slot % 5 == 4);
        }
    }

    /// Slow: searches a board for every rotation theme. Run with --ignored after
    /// changing the theme lists or the search.
    #[test]
    #[ignore]
    fn rotation_themes_can_reach_the_bar() {
        let dict = Dictionary::new();
        let themes = Themes::load();
        for name in ROTATION_THEMES.iter().chain(SEASONAL_THEMES.iter()) {
            let theme = themes.list.iter().find(|t| t.name == *name).unwrap();
            let found = (0..3u64).filter(|s| build_theme_board(&dict, theme, &mut StdRng::seed_from_u64(*s)).is_some()).count();
            assert!(found >= 2, "{name}: only {found} of 3 searches reached {THEME_WORDS_REQUIRED}");
        }
    }
}
