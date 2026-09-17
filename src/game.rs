//! WordHero-style round logic: trace adjacent letters, score by word length,
//! beat the clock.

use crate::dictionary::{Dictionary, MIN_WORD_LEN};
use crate::league::{RankChange, Ranking};
use crate::net;
use crate::storage;
use crate::rotation::SoloRotation;
use crate::themes::{Theme, Themes};
use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::{Rng, SeedableRng};
use std::collections::{HashMap, HashSet};

pub const SIZE: usize = 4;
/// A live round. Everyone playing is on the same one at the same time.
pub const ROUND_SECONDS: f32 = 180.0;
/// How long the scorecard holds before the next round begins. Short, so the next
/// round is never long in coming.
pub const RESULTS_SECONDS: f32 = 30.0;
/// Everyone is on the same cycle, so the scorecard is a fixed window rather than
/// something you dismiss: when it runs out the next round begins.

/// The 16 standard Boggle dice. Rolling real dice gives a far better letter mix
/// than sampling letter frequencies independently, which tends to strand vowels.
const DICE: [[&str; 6]; 16] = [
    ["a", "a", "e", "e", "g", "n"],
    ["a", "b", "b", "j", "o", "o"],
    ["a", "c", "h", "o", "p", "s"],
    ["a", "f", "f", "k", "p", "s"],
    ["a", "o", "o", "t", "t", "w"],
    ["c", "i", "m", "o", "t", "u"],
    ["d", "e", "i", "l", "r", "x"],
    ["d", "e", "l", "r", "v", "y"],
    ["d", "i", "s", "t", "t", "y"],
    ["e", "e", "g", "h", "n", "w"],
    ["e", "e", "i", "n", "s", "u"],
    ["e", "h", "r", "t", "v", "w"],
    ["e", "i", "o", "s", "s", "t"],
    ["e", "l", "r", "t", "t", "y"],
    ["h", "i", "m", "n", "qu", "u"],
    ["h", "l", "n", "n", "r", "z"],
];

/// A board is only fun if there is plenty to find. Reroll until it clears these.
pub const MIN_SOLUTIONS: usize = 60;
pub const MIN_LONG_SOLUTIONS: usize = 4;
pub const LONG_WORD_LEN: usize = 6;
/// A superword fills the whole board: one word across all sixteen tiles.
pub const SUPERWORD_TILES: usize = SIZE * SIZE;
/// What finding a superword is worth, whatever its length: nearly four times its
/// length's points and more than two very good rounds, so the board built around
/// it is worth playing for.
pub const SUPERWORD_POINTS: u32 = 20_000;
/// Obscure words score this many tenths of a common word's points: 10% more, for
/// knowing them.
const OBSCURE_TENTHS: u32 = 11;

/// The letter bonus, for using the whole board. Each letter used in a found word
/// (a yellow star) earns this much...
pub const LETTER_USED_BONUS: u32 = 25;
/// ...each letter used in two or more (a green star) this much again: reusing a
/// letter is the harder find...
pub const LETTER_REUSED_BONUS: u32 = 100;
/// ...and using every letter on the board this much more on top.
pub const FULL_BOARD_BONUS: u32 = 500;
/// A longest word at least this long makes an unthemed board a "Long" one.
const LONG_BOARD_LEN: usize = 10;
/// Attempts the dictionary-free derivation makes at a board that looks playable.
const DERIVE_ATTEMPTS: usize = 24;
/// Every tile has to appear in at least this many words, so no letter is dead and
/// every one of them can be reused for the bonus.
pub const MIN_TILE_COVERAGE: u32 = 2;
/// Seeds tried before settling for the best board found.
const SEED_ATTEMPTS: u32 = 48;

/// Single letters as `&'static str`, so planted tiles match the type the dice use.
const LETTERS: [&str; 26] = [
    "a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "m", "n", "o", "p", "q", "r", "s",
    "t", "u", "v", "w", "x", "y", "z",
];
const MAX_BOARD_ATTEMPTS: usize = 400;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Position {
    pub row: usize,
    pub col: usize,
}

#[derive(Clone, Copy)]
pub struct Cell {
    /// Usually one letter, but the `Qu` die puts two on a single tile.
    pub letters: &'static str,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    Ready,
    Playing,
    Over,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Feedback {
    None,
    Accepted,
    NotAWord,
    Duplicate,
    TooShort,
}

#[derive(Clone)]
pub struct FoundWord {
    pub word: String,
    pub points: u32,
    /// The tiles traced to spell it. The letter bonus follows the tiles actually
    /// used, and a word can often be traced more than one way.
    pub path: Vec<Position>,
}

pub struct Game {
    pub dictionary: Dictionary,
    pub grid: [[Cell; SIZE]; SIZE],
    pub path: Vec<Position>,
    pub found: Vec<FoundWord>,
    found_set: HashSet<String>,
    /// Every word hiding on the current board, split by tier.
    pub words: BoardWords,
    /// How many times each tile was used across the words you found.
    pub tile_uses: [[u32; SIZE]; SIZE],
    /// The category this board was built around, if any.
    pub theme: Option<String>,
    /// The server's round number when this board is the one everyone is playing;
    /// `None` for a board generated here for solo play.
    pub round: Option<u64>,
    themes: Themes,
    pub score: u32,
    pub time_left: f32,
    /// Seconds left on the scorecard before the next round starts on its own.
    pub results_left: f32,
    pub phase: Phase,
    pub ranking: Ranking,
    /// How the round just played moved you on the ladder.
    pub rank_change: Option<RankChange>,
    pub is_dragging: bool,
    /// What solo play serves next, when there is no server.
    solo: SoloRotation,
    /// A round saved before the page was last closed, waiting to be picked back
    /// up. A shared one needs the server's clock; see `live`.
    pub pending_resume: Option<SavedRound>,
    /// Seconds of play since the round was last saved.
    since_saved: f32,

    // Presentation state the UI animates against.
    pub feedback: Feedback,
    pub feedback_word: String,
    pub feedback_points: u32,
    pub feedback_timer: f32,
    pub feedback_cells: Vec<Position>,
}

impl Game {
    pub fn new() -> Self {
        let dictionary = Dictionary::new();
        let themes = Themes::load();
        let mut rng = StdRng::from_entropy();
        // Only the backdrop behind the start screen; every round replaces it.
        let (grid, words, theme) = generate_board_local(&dictionary, &mut rng);
        let solo = SoloRotation::new(&mut rng);
        let ranking = Ranking::load();
        let pending_resume = storage::read(storage::ROUND).and_then(|t| SavedRound::from_text(&t));

        Game {
            dictionary,
            grid,
            path: Vec::new(),
            found: Vec::new(),
            found_set: HashSet::new(),
            words,
            tile_uses: [[0; SIZE]; SIZE],
            theme,
            themes,
            round: None,
            score: 0,
            time_left: ROUND_SECONDS,
            results_left: RESULTS_SECONDS,
            phase: Phase::Ready,
            ranking,
            rank_change: None,
            is_dragging: false,
            solo,
            pending_resume,
            since_saved: 0.0,
            feedback: Feedback::None,
            feedback_word: String::new(),
            feedback_points: 0,
            feedback_timer: 0.0,
            feedback_cells: Vec::new(),
        }
    }

    /// Start a solo round on a board generated here.
    pub fn start_round(&mut self) {
        let mut rng = StdRng::from_entropy();
        let (grid, words, theme) = self.solo.next_board(&self.dictionary, &mut rng);
        self.begin(grid, words, theme, None, ROUND_SECONDS);
    }

    /// Start the round everyone is playing, on the server's board, with however
    /// much of its clock is left. Returns false if the board is not one this
    /// client can read, so the caller can fall back rather than play garbage.
    pub fn start_shared_round(&mut self, round: u64, board: &net::Board, time_left: f32) -> bool {
        let Some(grid) = parse_grid(&board.grid) else { return false };
        let mut words = solve_board(&self.dictionary, &grid);
        let theme = board.theme.clone();
        if let Some(def) = theme.as_ref().and_then(|t| self.themes.list.iter().find(|d| &d.name == t)) {
            words.theme = collect_theme_words(&words, def);
        }
        self.begin(grid, words, theme, Some(round), time_left.clamp(0.0, ROUND_SECONDS));
        true
    }

    fn begin(
        &mut self,
        grid: [[Cell; SIZE]; SIZE],
        words: BoardWords,
        theme: Option<String>,
        round: Option<u64>,
        time_left: f32,
    ) {
        self.rank_change = None;
        self.grid = grid;
        self.words = words;
        self.theme = theme;
        self.round = round;
        self.tile_uses = [[0; SIZE]; SIZE];
        self.path.clear();
        self.found.clear();
        self.found_set.clear();
        self.score = 0;
        self.time_left = time_left;
        self.results_left = RESULTS_SECONDS;
        self.phase = Phase::Playing;
        self.is_dragging = false;
        self.clear_feedback();
        self.save_round();
    }

    /// Pick up a solo round saved before the page closed. It resumes with the time
    /// it had: a solo clock does not run while the page is closed. A shared round
    /// waits for the server's clock instead.
    pub fn resume_solo(&mut self) -> bool {
        let Some(saved) = self.pending_resume.take_if(|s| s.round.is_none()) else { return false };
        let Some(grid) = parse_grid(&saved.grid) else { return false };
        let words = solve_board(&self.dictionary, &grid);
        self.begin(grid, words, saved.theme.clone(), None, saved.time_left.clamp(1.0, ROUND_SECONDS));
        self.restore_found(&saved.found);
        true
    }

    /// Pick up a shared round saved before the page closed, on the server's board
    /// for it, with whatever time the round has left. Refused if the saved board is
    /// not that round's board.
    pub fn resume_shared(&mut self, round: u64, board: &net::Board, time_left: f32) -> bool {
        let Some(saved) = self.pending_resume.take_if(|s| s.round == Some(round)) else { return false };
        if saved.grid.iter().map(|t| t.to_ascii_lowercase()).ne(board.grid.iter().map(|t| t.to_ascii_lowercase())) {
            return false;
        }
        if !self.start_shared_round(round, board, time_left) {
            return false;
        }
        self.restore_found(&saved.found);
        true
    }

    /// A saved shared round whose time ran out while the page was closed: bank it
    /// as it stood, so the words found are not lost. Returns the round, for the
    /// caller to hand in while the server still takes it.
    pub fn finish_pending(&mut self) -> Option<u64> {
        let saved = self.pending_resume.take()?;
        let grid = parse_grid(&saved.grid)?;
        let words = solve_board(&self.dictionary, &grid);
        self.begin(grid, words, saved.theme.clone(), saved.round, 0.0);
        self.restore_found(&saved.found);
        self.end_round();
        self.phase = Phase::Ready;
        saved.round
    }

    /// Put saved words back on the board: their points, their stars, the score.
    /// A word whose path no longer spells it here is dropped.
    fn restore_found(&mut self, found: &[(String, Vec<Position>)]) {
        for (word, path) in found {
            let spelled: String = path
                .iter()
                .filter(|p| p.row < SIZE && p.col < SIZE)
                .map(|p| self.grid[p.row][p.col].letters)
                .collect();
            let steps = path.windows(2).all(|w| is_adjacent(w[0], w[1]));
            if spelled != *word || !steps || self.found_set.contains(word) || !self.dictionary.contains(word) {
                continue;
            }
            let points = self.word_score(word);
            for at in path {
                self.tile_uses[at.row][at.col] += 1;
            }
            self.found_set.insert(word.clone());
            self.found.push(FoundWord { word: word.clone(), points, path: path.clone() });
        }
        self.score = self.found.iter().map(|w| w.points).sum::<u32>() + self.letter_bonus();
        self.save_round();
    }

    /// Write the round in play to storage, so a refresh does not lose it.
    fn save_round(&mut self) {
        self.since_saved = 0.0;
        if self.phase != Phase::Playing {
            return;
        }
        let saved = SavedRound {
            round: self.round,
            grid: self.grid.iter().flatten().map(|c| c.letters.to_string()).collect(),
            theme: self.theme.clone(),
            time_left: self.time_left,
            found: self.found.iter().map(|w| (w.word.clone(), w.path.clone())).collect(),
        };
        storage::write(storage::ROUND, &saved.to_text());
    }

    fn clear_saved_round(&mut self) {
        storage::write(storage::ROUND, "");
    }

    pub fn tick(&mut self, dt: f32) {
        if self.feedback_timer > 0.0 {
            self.feedback_timer -= dt;
            if self.feedback_timer <= 0.0 {
                self.clear_feedback();
            }
        }

        // The scorecard is a timed window, not a screen you dismiss. What starts
        // when it runs out -- the shared round or a solo one -- is decided by
        // `live`, which knows whether there is a server to follow.
        if self.phase == Phase::Over {
            self.results_left = (self.results_left - dt).max(0.0);
            return;
        }

        if self.phase != Phase::Playing {
            return;
        }

        self.time_left -= dt;
        // A solo round's clock is its own, so keep it saved as it runs down.
        self.since_saved += dt;
        if self.since_saved >= 5.0 {
            self.save_round();
        }
        if self.time_left <= 0.0 {
            self.time_left = 0.0;
            self.end_round();
        }
    }

    pub fn end_round(&mut self) {
        if self.phase != Phase::Playing {
            return;
        }
        self.phase = Phase::Over;
        self.results_left = RESULTS_SECONDS;
        self.path.clear();
        self.is_dragging = false;
        // Finished and banked: nothing left to resume.
        self.clear_saved_round();
        // A round with nothing found was not played -- a tab left open, a player
        // who wandered off -- and a zero would drag the rank average down for it.
        // It still goes on the leaderboard; it just is not banked.
        self.rank_change = None;
        if self.counts_toward_rank() {
            let best = self.found.iter().max_by_key(|w| (w.points, w.word.len()));
            let best = best.map(|w| (w.word.as_str(), w.points));
            self.rank_change = Some(self.ranking.record_round(self.score, self.found.len() as u32, best));
            self.ranking.store();
        }
    }

    /// Whether this round is banked into the rank average. Only a round in which
    /// the player actually scored is.
    pub fn counts_toward_rank(&self) -> bool {
        self.score > 0
    }

    // --- tracing -----------------------------------------------------------

    pub fn begin_path(&mut self, pos: Position) {
        if self.phase != Phase::Playing {
            return;
        }
        self.path.clear();
        self.path.push(pos);
    }

    pub fn extend_path(&mut self, pos: Position) {
        if self.phase != Phase::Playing {
            return;
        }

        // Dragging back over an earlier tile rewinds the trail to that point.
        if let Some(i) = self.path.iter().position(|p| *p == pos) {
            self.path.truncate(i + 1);
            return;
        }

        if let Some(&last) = self.path.last() {
            if is_adjacent(last, pos) {
                self.path.push(pos);
            }
        } else {
            self.path.push(pos);
        }
    }

    pub fn clear_path(&mut self) {
        self.path.clear();
    }

    pub fn path_contains(&self, pos: Position) -> bool {
        self.path.contains(&pos)
    }

    pub fn current_word(&self) -> String {
        self.path
            .iter()
            .map(|p| self.grid[p.row][p.col].letters)
            .collect()
    }

    /// True while the traced letters could still grow into a word — used to keep
    /// the trail lit rather than to score anything.
    pub fn current_word_is_valid(&self) -> bool {
        let word = self.current_word();
        word.len() >= MIN_WORD_LEN
            && !self.found_set.contains(&word)
            && self.dictionary.contains(&word)
    }

    /// Score the traced word and clear the trail.
    pub fn submit_path(&mut self) {
        if self.phase != Phase::Playing || self.path.is_empty() {
            self.path.clear();
            return;
        }

        let word = self.current_word();
        let cells = std::mem::take(&mut self.path);

        if word.len() < MIN_WORD_LEN {
            self.set_feedback(Feedback::TooShort, word, 0, cells);
            return;
        }
        if self.found_set.contains(&word) {
            self.set_feedback(Feedback::Duplicate, word, 0, cells);
            return;
        }
        if !self.dictionary.contains(&word) {
            self.set_feedback(Feedback::NotAWord, word, 0, cells);
            return;
        }

        let points = self.word_score(&word);
        for at in &cells {
            self.tile_uses[at.row][at.col] += 1;
        }
        self.found_set.insert(word.clone());
        self.found.push(FoundWord { word: word.clone(), points, path: cells.clone() });
        // Words, plus whatever the letter bonus has grown to with this one.
        self.score = self.found.iter().map(|w| w.points).sum::<u32>() + self.letter_bonus();
        self.set_feedback(Feedback::Accepted, word, points, cells);
        self.save_round();
    }

    /// The bonus the letters used so far have earned.
    pub fn letter_bonus(&self) -> u32 {
        letter_bonus(&self.tile_uses)
    }

    /// Leave a round part way through. Whatever was scored is banked now, exactly
    /// as if the round had ended, and the player goes back to the home screen
    /// rather than on to the scorecard.
    pub fn leave_round(&mut self) {
        if self.phase != Phase::Playing {
            return;
        }
        self.end_round();
        self.phase = Phase::Ready;
    }

    /// Put the scorecard away. The round was banked when it ended.
    pub fn close_results(&mut self) {
        if self.phase == Phase::Over {
            self.phase = Phase::Ready;
        }
    }

    fn set_feedback(&mut self, kind: Feedback, word: String, points: u32, cells: Vec<Position>) {
        self.feedback = kind;
        self.feedback_word = word;
        self.feedback_points = points;
        self.feedback_timer = if kind == Feedback::Accepted { 0.9 } else { 0.7 };
        self.feedback_cells = cells;
    }

    fn clear_feedback(&mut self) {
        self.feedback = Feedback::None;
        self.feedback_word.clear();
        self.feedback_points = 0;
        self.feedback_timer = 0.0;
        self.feedback_cells.clear();
    }

    // --- round summary -----------------------------------------------------

    /// Everything the board was worth: the score if every common word were found.
    /// Everything on offer: every common word, and the letter bonus they would
    /// earn together -- a tile in one common word is a yellow star, in two a green.
    pub fn board_par(&self) -> u32 {
        let words: u32 = self.words.common.iter().map(|w| score_word(w, false)).sum();
        words + letter_bonus(&self.words.coverage)
    }

    /// What a word on this board scores, by its length and its tier.
    pub fn word_score(&self, word: &str) -> u32 {
        score_word(word, !self.dictionary.is_common(word))
    }

    pub fn found_count(&self) -> usize {
        self.found.len()
    }

    /// Denominator for "words found": the common tier, since holding anyone to the
    /// obscure list would be absurd.
    pub fn findable_count(&self) -> usize {
        self.words.common.len()
    }

    pub fn found_fraction(&self) -> f32 {
        if self.findable_count() == 0 {
            return 0.0;
        }
        (self.found_count() as f32 / self.findable_count() as f32).min(1.0)
    }

    /// Points per word found: the words' own points, not the letter bonus.
    pub fn average_points(&self) -> f32 {
        if self.found.is_empty() {
            return 0.0;
        }
        self.found.iter().map(|w| w.points).sum::<u32>() as f32 / self.found.len() as f32
    }

    /// Seconds of the round spent per word found.
    pub fn seconds_per_word(&self) -> f32 {
        if self.found.is_empty() {
            return 0.0;
        }
        ROUND_SECONDS / self.found.len() as f32
    }

    pub fn average_word_length(&self) -> f32 {
        if self.found.is_empty() {
            return 0.0;
        }
        self.found.iter().map(|w| w.word.len()).sum::<usize>() as f32 / self.found.len() as f32
    }

    /// Words that fill the whole board. A superword board has one; any other
    /// board has none.
    pub fn superwords(&self) -> Vec<&String> {
        self.words
            .common
            .iter()
            .chain(self.words.obscure.iter())
            .filter(|w| crate::rotation::word_tiles(w).is_some_and(|t| t.len() >= SUPERWORD_TILES))
            .collect()
    }

    /// The results screen's third tab: the theme's words on a themed board, the
    /// superword on a superword board, otherwise the board's longest words.
    pub fn highlight_tab(&self) -> (String, Vec<&String>) {
        if let Some(theme) = &self.theme {
            return (theme.clone(), self.words.theme.iter().collect());
        }
        let supers = self.superwords();
        if !supers.is_empty() {
            return ("Superword".to_string(), supers);
        }
        let long = self.words.common.iter().filter(|w| w.len() >= 8).collect();
        ("Longest".to_string(), long)
    }

    /// What kind of board this is: its theme or "Superword" when it was built as
    /// one, otherwise a shape read off its own solution set.
    pub fn board_type(&self) -> String {
        if let Some(theme) = &self.theme {
            return theme.clone();
        }
        if !self.superwords().is_empty() {
            return "Superword".to_string();
        }

        let longest = self.words.common.first().map(|w| w.len()).unwrap_or(0);
        let count = self.words.common.len();
        match () {
            _ if longest >= LONG_BOARD_LEN => "Long".to_string(),
            _ if count >= 120 => "Dense".to_string(),
            _ if count < 70 => "Sparse".to_string(),
            _ => "Standard".to_string(),
        }
    }

    /// Every word on the board -- common and obscure -- that can be traced through
    /// `at`, each with one such path, longest words first. The scorecard shows
    /// these when a letter is tapped, so a player can see what a letter was good
    /// for and how each word runs.
    pub fn words_through(&self, at: Position) -> Vec<(String, Vec<Position>)> {
        let mut out: Vec<(String, Vec<Position>)> = self
            .words
            .common
            .iter()
            .chain(self.words.obscure.iter())
            .filter_map(|word| path_through(&self.grid, word, at).map(|path| (word.clone(), path)))
            .collect();
        out.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then_with(|| a.0.cmp(&b.0)));
        out.dedup_by(|a, b| a.0 == b.0);
        out
    }

    /// The words found this round, in the order they were found.
    pub fn found_words(&self) -> Vec<String> {
        self.found.iter().map(|w| w.word.clone()).collect()
    }

    /// The path traced for each found word, in the same order, as tile indices
    /// (row * 4 + column): what the server checks the letter bonus against.
    pub fn found_paths(&self) -> Vec<Vec<u8>> {
        self.found
            .iter()
            .map(|w| w.path.iter().map(|p| (p.row * SIZE + p.col) as u8).collect())
            .collect()
    }

    pub fn has_found(&self, word: &str) -> bool {
        self.found_set.contains(word)
    }

    pub fn best_score(&self) -> u32 {
        self.ranking.best_score
    }

    pub fn is_new_best(&self) -> bool {
        self.score > 0 && self.score >= self.ranking.best_score
    }
}

/// WordHero's payout curve: length is worth far more than volume, so a single
/// seven-letter find beats a fistful of threes.
pub fn word_points(len: usize) -> u32 {
    match len {
        0..=2 => 0,
        3 => 100,
        4 => 400,
        5 => 800,
        6 => 1_400,
        7 => 1_800,
        8 => 2_200,
        n => 2_200 + 400 * (n as u32 - 8),
    }
}

/// A round in play, as saved to storage.
#[derive(Clone, Debug, PartialEq)]
pub struct SavedRound {
    /// The server's round, or `None` for a solo round.
    pub round: Option<u64>,
    /// Sixteen tiles, row by row.
    pub grid: Vec<String>,
    pub theme: Option<String>,
    pub time_left: f32,
    /// Each found word and the path it was traced along.
    pub found: Vec<(String, Vec<Position>)>,
}

impl SavedRound {
    pub fn to_text(&self) -> String {
        let found: Vec<String> = self
            .found
            .iter()
            .map(|(word, path)| {
                let tiles: Vec<String> = path.iter().map(|p| (p.row * SIZE + p.col).to_string()).collect();
                format!("{word}:{}", tiles.join("."))
            })
            .collect();
        format!(
            "round={}\ngrid={}\ntheme={}\ntime_left={}\nfound={}\n",
            self.round.map(|r| r.to_string()).unwrap_or_else(|| "solo".to_string()),
            self.grid.join(","),
            self.theme.clone().unwrap_or_default(),
            self.time_left,
            found.join(";"),
        )
    }

    /// Read a saved round; `None` for an empty or damaged save.
    pub fn from_text(text: &str) -> Option<Self> {
        let mut saved = SavedRound { round: None, grid: Vec::new(), theme: None, time_left: 0.0, found: Vec::new() };
        let mut has_round = false;
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else { continue };
            match key {
                "round" => {
                    has_round = true;
                    saved.round = if value == "solo" { None } else { Some(value.parse().ok()?) };
                }
                "grid" => saved.grid = value.split(',').map(str::to_string).collect(),
                "theme" => saved.theme = (!value.is_empty()).then(|| value.to_string()),
                "time_left" => saved.time_left = value.parse().unwrap_or(0.0),
                "found" => {
                    saved.found = value
                        .split(';')
                        .filter(|w| !w.is_empty())
                        .filter_map(|entry| {
                            let (word, tiles) = entry.split_once(':')?;
                            let path = tiles
                                .split('.')
                                .map(|t| t.parse::<usize>().ok().map(|i| Position { row: i / SIZE, col: i % SIZE }))
                                .collect::<Option<Vec<_>>>()?;
                            Some((word.to_string(), path))
                        })
                        .collect();
                }
                _ => {}
            }
        }
        (has_round && saved.grid.len() == SIZE * SIZE).then_some(saved)
    }
}

/// The letter bonus for how often each tile has been used: every used letter,
/// every reused letter, and the whole board. The server works it out the same way
/// from the paths it is sent.
pub fn letter_bonus(uses: &[[u32; SIZE]; SIZE]) -> u32 {
    let used = uses.iter().flatten().filter(|u| **u >= 1).count() as u32;
    let reused = uses.iter().flatten().filter(|u| **u >= 2).count() as u32;
    let full = if used == (SIZE * SIZE) as u32 { FULL_BOARD_BONUS } else { 0 };
    used * LETTER_USED_BONUS + reused * LETTER_REUSED_BONUS + full
}

/// What a word scores: a superword its flat bonus, anything else its length's
/// points, with 10% more for an obscure word. The server scores submissions the
/// same way; the two must agree or the leaderboard will not match the scorecard.
pub fn score_word(word: &str, obscure: bool) -> u32 {
    if crate::rotation::word_tiles(word).is_some_and(|t| t.len() >= SUPERWORD_TILES) {
        return SUPERWORD_POINTS;
    }
    let points = word_points(word.len());
    if obscure { points * OBSCURE_TENTHS / 10 } else { points }
}

/// A path spelling `word` across `grid`, if there is one.
pub fn find_path(grid: &[[Cell; SIZE]; SIZE], word: &str) -> Option<Vec<Position>> {
    trace(grid, word, None)
}

/// A board as the server sends it -- sixteen tiles, row by row -- in the form the
/// game plays on. Themed boards plant any letter, so a tile is any single letter
/// or the `qu` die; anything else is refused.
pub fn parse_grid(tiles: &[String]) -> Option<[[Cell; SIZE]; SIZE]> {
    if tiles.len() != SIZE * SIZE {
        return None;
    }
    let mut cells = Vec::with_capacity(SIZE * SIZE);
    for tile in tiles {
        let tile = tile.to_ascii_lowercase();
        let letters = LETTERS.iter().chain(["qu"].iter()).find(|face| **face == tile)?;
        cells.push(Cell { letters });
    }
    Some(std::array::from_fn(|row| std::array::from_fn(|col| cells[row * SIZE + col])))
}

/// A path spelling `word` across `grid` that passes through `through`, if any.
pub fn path_through(grid: &[[Cell; SIZE]; SIZE], word: &str, through: Position) -> Option<Vec<Position>> {
    trace(grid, word, Some(through))
}

/// A path spelling `word`, through `through` when one is given.
fn trace(grid: &[[Cell; SIZE]; SIZE], word: &str, through: Option<Position>) -> Option<Vec<Position>> {
    fn step(
        grid: &[[Cell; SIZE]; SIZE],
        rest: &str,
        at: Position,
        through: Option<Position>,
        path: &mut Vec<Position>,
    ) -> bool {
        if path.contains(&at) {
            return false;
        }
        let Some(tail) = rest.strip_prefix(grid[at.row][at.col].letters) else { return false };
        path.push(at);
        if tail.is_empty() {
            if through.is_none_or(|t| path.contains(&t)) {
                return true;
            }
        } else {
            for dr in -1i32..=1 {
                for dc in -1i32..=1 {
                    let (r, c) = (at.row as i32 + dr, at.col as i32 + dc);
                    if (dr, dc) != (0, 0) && (0..SIZE as i32).contains(&r) && (0..SIZE as i32).contains(&c) {
                        let next = Position { row: r as usize, col: c as usize };
                        if step(grid, tail, next, through, path) {
                            return true;
                        }
                    }
                }
            }
        }
        path.pop();
        false
    }

    let mut path = Vec::with_capacity(word.len());
    for row in 0..SIZE {
        for col in 0..SIZE {
            if step(grid, word, Position { row, col }, through, &mut path) {
                return Some(path);
            }
        }
    }
    None
}

pub fn is_adjacent(a: Position, b: Position) -> bool {
    let dr = (a.row as i32 - b.row as i32).abs();
    let dc = (a.col as i32 - b.col as i32).abs();
    dr <= 1 && dc <= 1 && (dr != 0 || dc != 0)
}

// --- board generation ------------------------------------------------------

/// Build the board for a given seed.
///
/// Synchronised play needs every client to arrive at the same board without the
/// board itself being sent, so generation must be a pure function of the seed:
/// no thread RNG, and every collection sorted into a total order before use.
pub fn generate_board_seeded(
    dict: &Dictionary,
    seed: u64,
) -> ([[Cell; SIZE]; SIZE], BoardWords, Option<String>) {
    // Walk a deterministic sequence of sub-seeds until one gives a board that
    // clears the bar, so the same seed always lands on the same good board.
    let mut best: Option<([[Cell; SIZE]; SIZE], BoardWords, Option<String>, u32)> = None;

    for attempt in 0..SEED_ATTEMPTS {
        let sub = seed
            .wrapping_mul(0x9E37_79B9_7F4A_7C15)
            .wrapping_add(attempt as u64);
        let grid = derive_grid(sub);
        let words = solve_board(dict, &grid);

        if board_is_good(&words) {
            return (grid, words, None);
        }

        let rank = words.common.len() as u32 + worst_tile(&words) * 20;
        if best.as_ref().is_none_or(|(_, _, _, r)| rank > *r) {
            best = Some((grid, words, None, rank));
        }
    }

    let (grid, words, theme, _) = best.expect("at least one board was derived");
    (grid, words, theme)
}

/// A plain board rolled from the dice, rerolled until it plays well.
pub fn generate_board_local(
    dict: &Dictionary,
    rng: &mut impl Rng,
) -> ([[Cell; SIZE]; SIZE], BoardWords, Option<String>) {
    let mut best: Option<([[Cell; SIZE]; SIZE], BoardWords, usize)> = None;

    for _ in 0..MAX_BOARD_ATTEMPTS {
        let grid = roll_dice(rng);
        let words = solve_board(dict, &grid);

        if board_is_good(&words) {
            return (grid, words, None);
        }

        let quality = words.common.len() + worst_tile(&words) as usize * 20;
        if best.as_ref().is_none_or(|(_, _, q)| quality > *q) {
            best = Some((grid, words, quality));
        }
    }

    let (grid, words, _) = best.expect("at least one board was rolled");
    (grid, words, None)
}

fn roll_dice(rng: &mut impl Rng) -> [[Cell; SIZE]; SIZE] {
    let mut order: Vec<usize> = (0..16).collect();
    order.shuffle(rng);

    std::array::from_fn(|row| {
        std::array::from_fn(|col| {
            let die = DICE[order[row * SIZE + col]];
            Cell { letters: die[rng.gen_range(0..6)] }
        })
    })
}

/// Everything hiding on a board, split by tier.
#[derive(Default, Clone)]
pub struct BoardWords {
    /// Ordinary words: what the board is scored and judged against.
    pub common: Vec<String>,
    /// Accepted when traced, but too obscure to hold anyone to.
    pub obscure: Vec<String>,
    /// Words belonging to the board's theme, if it has one.
    pub theme: Vec<String>,
    /// For each tile, how many common words can be traced through it. A tile no
    /// word can use is a dead tile, and one only a single word uses can never be
    /// reused for the bonus.
    pub coverage: [[u32; SIZE]; SIZE],
}

/// Every word reachable by tracing adjacent tiles, each list longest first.
pub fn solve_board(dict: &Dictionary, grid: &[[Cell; SIZE]; SIZE]) -> BoardWords {
    // Each common word maps to the union of every path that spells it, so a tile
    // counts as covering a word if any route to that word goes through it.
    let mut common: HashMap<String, u16> = HashMap::new();
    let mut obscure = HashSet::new();
    let mut word = String::new();

    for row in 0..SIZE {
        for col in 0..SIZE {
            walk(dict, grid, row, col, 0, dict.root(), &mut word, &mut common, &mut obscure);
        }
    }

    let mut coverage = [[0u32; SIZE]; SIZE];
    for mask in common.values() {
        for slot in 0..(SIZE * SIZE) {
            if mask & (1 << slot) != 0 {
                coverage[slot / SIZE][slot % SIZE] += 1;
            }
        }
    }

    let mut common_words: Vec<String> = common.into_keys().collect();
    common_words.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
    let mut obscure_words: Vec<String> = obscure.into_iter().collect();
    obscure_words.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));

    BoardWords {
        common: common_words,
        obscure: obscure_words,
        theme: Vec::new(),
        coverage,
    }
}

fn walk(
    dict: &Dictionary,
    grid: &[[Cell; SIZE]; SIZE],
    row: usize,
    col: usize,
    visited: u16,
    node: u32,
    word: &mut String,
    common: &mut HashMap<String, u16>,
    obscure: &mut HashSet<String>,
) {
    let bit = 1u16 << (row * SIZE + col);
    if visited & bit != 0 {
        return;
    }

    let letters = grid[row][col].letters;
    let Some(node) = dict.step_str(node, letters) else {
        return; // No word continues this way; drop the whole branch.
    };

    word.push_str(letters);
    let visited = visited | bit;

    if word.len() >= MIN_WORD_LEN && dict.is_word_node(node) {
        // The tier decides which list it lands in: the board gate and the headline
        // scorecard read `common`, so neither ever cites junk.
        if dict.is_common_node(node) {
            *common.entry(word.clone()).or_insert(0) |= visited;
        } else {
            obscure.insert(word.clone());
        }
    }

    if !dict.is_dead_end(node) {
        for dr in -1i32..=1 {
            for dc in -1i32..=1 {
                if dr == 0 && dc == 0 {
                    continue;
                }
                let r = row as i32 + dr;
                let c = col as i32 + dc;
                if r < 0 || c < 0 || r >= SIZE as i32 || c >= SIZE as i32 {
                    continue;
                }
                walk(dict, grid, r as usize, c as usize, visited, node, word, common, obscure);
            }
        }
    }

    word.truncate(word.len() - letters.len());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn board(rows: [&'static str; 4]) -> [[Cell; SIZE]; SIZE] {
        std::array::from_fn(|row| std::array::from_fn(|col| Cell { letters: &rows[row][col..col + 1] }))
    }

    #[test]
    fn scoring_rewards_length() {
        assert_eq!(word_points(2), 0);
        assert_eq!(word_points(3), 100);
        assert_eq!(word_points(7), 1_800);
        assert_eq!(word_points(9), 2_600);
        assert!(word_points(7) > word_points(3) * 4);
    }

    #[test]
    fn adjacency_includes_diagonals_but_not_self() {
        let a = Position { row: 1, col: 1 };
        assert!(is_adjacent(a, Position { row: 0, col: 0 }));
        assert!(is_adjacent(a, Position { row: 2, col: 1 }));
        assert!(!is_adjacent(a, a));
        assert!(!is_adjacent(a, Position { row: 3, col: 1 }));
    }

    #[test]
    fn solver_reports_only_common_words() {
        let dict = Dictionary::new();

        let common = board(["wars", "zzzz", "zzzz", "zzzz"]);
        assert!(solve_board(&dict, &common).common.iter().any(|w| w == "wars"));

        // "adit" is a real word and playable, but not an everyday one: it belongs
        // in the obscure list, not the common one.
        let grid = board(["adit", "zzzz", "zzzz", "zzzz"]);
        let words = solve_board(&dict, &grid);
        assert!(dict.contains("adit"));
        assert!(!words.common.iter().any(|w| w == "adit"));
        assert!(words.obscure.iter().any(|w| w == "adit"));
    }

    #[test]
    fn solver_finds_traceable_words_only() {
        let dict = Dictionary::new();
        // "herd" traces across the top row; "hero" would need a non-adjacent jump.
        let grid = board(["herd", "zzzz", "zzzo", "zzzz"]);
        let words = solve_board(&dict, &grid).common;
        assert!(words.iter().any(|w| w == "herd"));
        assert!(!words.iter().any(|w| w == "hero"));
    }

    #[test]
    fn solver_returns_longest_first() {
        let dict = Dictionary::new();
        let grid = board(["herd", "zzzz", "zzzz", "zzzz"]);
        let words = solve_board(&dict, &grid).common;
        assert!(words.windows(2).all(|w| w[0].len() >= w[1].len()));
    }

    #[test]
    fn generated_boards_are_word_rich() {
        let dict = Dictionary::new();
        let (_, words, _) = generate_board_local(&dict, &mut rand::thread_rng());
        assert!(words.common.len() >= MIN_SOLUTIONS, "got {} words", words.common.len());
    }

    #[test]
    fn tile_use_is_counted_per_letter() {
        let mut game = Game::new();
        game.grid = board(["herd", "zzzz", "zzzz", "zzzz"]);
        game.words = solve_board(&game.dictionary, &game.grid);
        game.tile_uses = [[0; SIZE]; SIZE];
        game.phase = Phase::Playing;

        for (row, col) in [(0, 0), (0, 1), (0, 2), (0, 3)] {
            game.extend_path(Position { row, col });
        }
        game.submit_path();

        // The four letters of "herd" were each used once; nothing else was touched.
        assert_eq!(game.tile_uses[0], [1, 1, 1, 1]);
        assert_eq!(game.tile_uses[1], [0, 0, 0, 0]);

        // "her" reuses three of them.
        for (row, col) in [(0, 0), (0, 1), (0, 2)] {
            game.extend_path(Position { row, col });
        }
        game.submit_path();
        assert_eq!(game.tile_uses[0], [2, 2, 2, 1]);
    }

    #[test]
    fn a_rejected_word_leaves_tile_use_alone() {
        let mut game = Game::new();
        game.grid = board(["zqxj", "zzzz", "zzzz", "zzzz"]);
        game.words = solve_board(&game.dictionary, &game.grid);
        game.tile_uses = [[0; SIZE]; SIZE];
        game.phase = Phase::Playing;

        for (row, col) in [(0, 0), (0, 1), (0, 2)] {
            game.extend_path(Position { row, col });
        }
        game.submit_path();

        assert_eq!(game.feedback, Feedback::NotAWord);
        assert_eq!(game.tile_uses[0], [0, 0, 0, 0]);
    }

    #[test]
    fn round_statistics_describe_the_round() {
        let mut game = Game::new();
        game.grid = board(["herd", "zzzz", "zzzz", "zzzz"]);
        game.words = solve_board(&game.dictionary, &game.grid);
        game.phase = Phase::Playing;

        for (row, col) in [(0, 0), (0, 1), (0, 2), (0, 3)] {
            game.extend_path(Position { row, col });
        }
        game.submit_path();

        assert_eq!(game.found_count(), 1);
        assert_eq!(game.average_points(), word_points(4) as f32);
        assert_eq!(game.average_word_length(), 4.0);
        assert_eq!(game.seconds_per_word(), ROUND_SECONDS);
        assert!(game.board_par() >= game.score);
        assert!(game.found_fraction() > 0.0 && game.found_fraction() <= 1.0);
    }

    #[test]
    fn statistics_are_safe_on_a_blank_round() {
        // Every one of these divides by the word count.
        let game = Game::new();
        assert_eq!(game.found_count(), 0);
        assert_eq!(game.average_points(), 0.0);
        assert_eq!(game.seconds_per_word(), 0.0);
        assert_eq!(game.average_word_length(), 0.0);
        assert_eq!(game.found_fraction(), 0.0);
    }

    /// Walk a word's letters over the grid and confirm it can be traced: adjacent
    /// steps, no tile used twice.
    fn is_traceable(grid: &[[Cell; SIZE]; SIZE], word: &str) -> bool {
        fn step(grid: &[[Cell; SIZE]; SIZE], rest: &str, at: Position, used: u16) -> bool {
            let bit = 1u16 << (at.row * SIZE + at.col);
            if used & bit != 0 {
                return false;
            }
            let letters = grid[at.row][at.col].letters;
            let Some(tail) = rest.strip_prefix(letters) else { return false };
            if tail.is_empty() {
                return true;
            }
            neighbours(at).into_iter().any(|n| step(grid, tail, n, used | bit))
        }
        (0..SIZE).flat_map(|r| (0..SIZE).map(move |c| Position { row: r, col: c }))
            .any(|start| step(grid, word, start, 0))
    }

    #[test]
    fn obscure_words_score_ten_percent_more_and_superwords_a_flat_bonus() {
        assert_eq!(score_word("cat", false), 100);
        assert_eq!(score_word("adit", true), 440);
        assert_eq!(score_word("oxen", false), 400);
        for len in 3..=12 {
            let word = "a".repeat(len);
            assert_eq!(score_word(&word, true), word_points(len) * 11 / 10, "{len} letters");
            assert_eq!(word_points(len) * 11 % 10, 0, "10% more is not a whole number at {len} letters");
        }
        // Sixteen tiles, "qu" being one of them, whatever the tier.
        assert_eq!(score_word("characterization", false), SUPERWORD_POINTS);
        assert_eq!(score_word("characterization", true), SUPERWORD_POINTS);
        assert!(SUPERWORD_POINTS > word_points(16) * 3, "a superword should be worth far more than its length");

        // The game scores a found word by its tier.
        let mut game = Game::new();
        game.grid = board(["adit", "zzzz", "zzzz", "zzzz"]);
        game.words = solve_board(&game.dictionary, &game.grid);
        game.phase = Phase::Playing;
        for col in 0..4 {
            game.extend_path(Position { row: 0, col });
        }
        game.submit_path();
        assert_eq!(game.found[0].points, 440, "an obscure word did not score 10% more");
        assert_eq!(game.score, 440 + game.letter_bonus());
    }

    #[test]
    fn the_letter_bonus_pays_for_used_letters_reused_ones_and_the_whole_board() {
        let mut uses = [[0u32; SIZE]; SIZE];
        assert_eq!(letter_bonus(&uses), 0);
        uses[0][0] = 1;
        assert_eq!(letter_bonus(&uses), 25, "a yellow star is 25");
        uses[0][0] = 2;
        assert_eq!(letter_bonus(&uses), 125, "a green star adds 100");
        uses[0][0] = 7;
        assert_eq!(letter_bonus(&uses), 125, "more uses earn nothing further");
        let all_once = [[1u32; SIZE]; SIZE];
        assert_eq!(letter_bonus(&all_once), 16 * 25 + 500, "every letter used adds 500");
        let all_twice = [[2u32; SIZE]; SIZE];
        assert_eq!(letter_bonus(&all_twice), 16 * 125 + 500);

        // A found word's score carries the bonus its letters earn.
        let mut game = Game::new();
        game.grid = board(["herd", "zzzz", "zzzz", "zzzz"]);
        game.words = solve_board(&game.dictionary, &game.grid);
        game.tile_uses = [[0; SIZE]; SIZE];
        game.phase = Phase::Playing;
        for col in 0..4 {
            game.extend_path(Position { row: 0, col });
        }
        game.submit_path();
        assert_eq!(game.score, word_points(4) + 4 * 25);
        assert_eq!(game.found_paths(), vec![vec![0u8, 1, 2, 3]]);
        // Three of its letters again, in "her": three green stars, 3 * 100 more.
        for col in 0..3 {
            game.extend_path(Position { row: 0, col });
        }
        game.submit_path();
        assert_eq!(game.score, word_points(4) + word_points(3) + 4 * 25 + 3 * 100);
    }

    #[test]
    fn a_saved_round_reads_back_and_resumes_with_its_words_and_stars() {
        let mut game = Game::new();
        game.start_round();
        game.grid = board(["herd", "zzzz", "zzzz", "zzzz"]);
        game.words = solve_board(&game.dictionary, &game.grid);
        game.round = Some(42);
        for col in 0..4 {
            game.extend_path(Position { row: 0, col });
        }
        game.submit_path();
        let saved = SavedRound {
            round: game.round,
            grid: game.grid.iter().flatten().map(|c| c.letters.to_string()).collect(),
            theme: Some("Animals".into()),
            time_left: 90.0,
            found: game.found.iter().map(|w| (w.word.clone(), w.path.clone())).collect(),
        };
        let back = SavedRound::from_text(&saved.to_text()).expect("reads back");
        assert_eq!(back, saved);
        assert!(SavedRound::from_text("").is_none(), "an empty save is no round");

        // The page comes back: the server says round 42 has 60 seconds left.
        let score = game.score;
        let mut fresh = Game::new();
        fresh.pending_resume = Some(back.clone());
        let board_msg = net::Board { grid: back.grid.clone(), theme: Some("Animals".into()) };
        assert!(fresh.resume_shared(42, &board_msg, 60.0));
        assert_eq!((fresh.phase, fresh.round, fresh.time_left), (Phase::Playing, Some(42), 60.0));
        assert_eq!(fresh.found_words(), vec!["herd".to_string()]);
        assert_eq!(fresh.score, score, "the resumed score differs");
        assert_eq!(fresh.tile_uses[0][..4], [1, 1, 1, 1], "the stars were not restored");

        // A different board for that round is not resumed onto.
        let mut other = Game::new();
        other.pending_resume = Some(back.clone());
        let wrong = net::Board { grid: vec!["a".into(); 16], theme: None };
        assert!(!other.resume_shared(42, &wrong, 60.0));

        // The round ran out while the page was closed: banked as it stood.
        let mut late = Game::new();
        late.pending_resume = Some(back);
        let games = late.ranking.lifetime.games;
        assert_eq!(late.finish_pending(), Some(42));
        assert_eq!(late.phase, Phase::Ready);
        assert_eq!(late.ranking.lifetime.games, games + 1, "the saved round was not banked");
    }

    #[test]
    fn leaving_a_round_banks_it_and_goes_home() {
        let mut game = Game::new();
        game.start_round();
        game.score = 3_000;
        let games = game.ranking.lifetime.games;
        game.leave_round();
        assert_eq!(game.phase, Phase::Ready, "leaving should go home, not to the scorecard");
        assert_eq!(game.ranking.lifetime.games, games + 1, "the score was not banked");
        assert_eq!(game.ranking.recent.back().copied(), Some(3_000));

        game.start_round();
        game.time_left = 0.0;
        game.tick(0.2);
        assert_eq!(game.phase, Phase::Over);
        game.close_results();
        assert_eq!(game.phase, Phase::Ready);
    }

    #[test]
    fn any_word_on_the_board_has_a_path() {
        let grid = board(["cats", "zazz", "zzzz", "zzzz"]);
        let path = find_path(&grid, "cats").expect("cats is on the board");
        let spelled: String = path.iter().map(|p| grid[p.row][p.col].letters).collect();
        assert_eq!(spelled, "cats");
        assert!(find_path(&grid, "dogs").is_none());
    }

    #[test]
    fn a_letter_lists_exactly_the_words_that_run_through_it() {
        let mut game = Game::new();
        game.grid = board(["cats", "zazz", "zzzz", "zzzz"]);
        game.words = solve_board(&game.dictionary, &game.grid);

        let t = Position { row: 0, col: 2 };
        let through_t = game.words_through(t);
        let words: Vec<&str> = through_t.iter().map(|(w, _)| w.as_str()).collect();
        assert!(words.contains(&"cats") && words.contains(&"cat"), "missing words through T: {words:?}");

        for (word, path) in &through_t {
            assert!(path.contains(&t), "{word}'s path skips the tapped letter");
            let spelled: String = path.iter().map(|p| game.grid[p.row][p.col].letters).collect();
            assert_eq!(&spelled, word, "the path does not spell the word");
            assert!(path.windows(2).all(|w| is_adjacent(w[0], w[1])), "{word}'s path jumps");
        }

        // A word that never touches the tile is not listed: nothing here reaches
        // the bottom corner.
        assert!(game.words_through(Position { row: 3, col: 3 }).is_empty());
    }

    #[test]
    fn a_planted_word_can_actually_be_traced() {
        let mut rng = rand::thread_rng();
        for word in ["otter", "cuba", "violin", "battle", "sun"] {
            let mut cells: [[Option<&'static str>; SIZE]; SIZE] = Default::default();
            assert!(plant_word(&mut cells, word, &mut rng), "could not plant {word}");

            let grid: [[Cell; SIZE]; SIZE] = std::array::from_fn(|row| {
                std::array::from_fn(|col| Cell { letters: cells[row][col].unwrap_or("z") })
            });
            assert!(is_traceable(&grid, word), "{word} was planted but cannot be traced");
        }
    }

    #[test]
    fn planting_leaves_the_grid_clean_when_it_fails() {
        let mut rng = rand::thread_rng();
        let mut cells: [[Option<&'static str>; SIZE]; SIZE] = Default::default();
        // Seventeen letters cannot fit on sixteen tiles without reuse.
        assert!(!plant_word(&mut cells, "abcdefghijklmnopq", &mut rng));
        assert!(
            cells.iter().flatten().all(|c| c.is_none()),
            "a failed planting left letters behind"
        );
    }

    /// Synchronised play stands on this: two clients given the same seed must build
    /// the identical board, or they are not playing the same game.
    #[test]
    fn a_seed_always_builds_the_same_board() {
        let dict = Dictionary::new();

        for seed in [0u64, 1, 42, 9_999, u64::MAX] {
            let (grid_a, words_a, theme_a) = generate_board_seeded(&dict, seed);
            let (grid_b, words_b, theme_b) = generate_board_seeded(&dict, seed);

            let letters = |g: &[[Cell; SIZE]; SIZE]| {
                (0..SIZE)
                    .map(|r| (0..SIZE).map(|c| g[r][c].letters).collect::<String>())
                    .collect::<Vec<_>>()
            };
            assert_eq!(letters(&grid_a), letters(&grid_b), "seed {seed} drifted");
            assert_eq!(theme_a, theme_b, "seed {seed} changed theme");
            assert_eq!(words_a.common, words_b.common, "seed {seed} changed solutions");
        }
    }

    /// Pinned boards. Synchronised clients only agree if generation is byte-stable,
    /// so a change here means old and new clients would silently play different
    /// boards from the same seed -- a deliberate decision, never an accident.
    #[test]
    fn seeded_boards_are_pinned() {
        let dict = Dictionary::new();

        let board_of = |seed| {
            let (g, w, t) = generate_board_seeded(&dict, seed);
            let rows: Vec<String> = (0..SIZE)
                .map(|r| (0..SIZE).map(|c| g[r][c].letters).collect::<Vec<_>>().join("|"))
                .collect();
            (rows, t, w.common.len(), worst_tile(&w))
        };

        let (rows, theme, count, worst) = board_of(42);
        assert_eq!(rows, ["e|a|s|u", "a|a|r|g", "t|l|d|c", "f|e|r|h"]);
        assert_eq!(theme, None, "seeded boards are plain; themes come from the rotation");
        assert_eq!(count, 77);
        assert!(worst >= MIN_TILE_COVERAGE);

        let (rows, theme, count, worst) = board_of(1234);
        assert_eq!(rows, ["n|a|t|e", "s|e|b|t", "n|s|e|a", "e|s|n|t"]);
        assert_eq!(theme, None);
        assert_eq!(count, 81);
        assert!(worst >= MIN_TILE_COVERAGE);
    }

    /// The rule that every letter earns its place: no tile may be stranded, and
    /// each must appear in enough words to be reused for the bonus.
    #[test]
    fn every_tile_appears_in_at_least_two_words() {
        let dict = Dictionary::new();

        for seed in 0..25u64 {
            let (grid, words, _) = generate_board_seeded(&dict, seed);
            let worst = worst_tile(&words);
            assert!(
                worst >= MIN_TILE_COVERAGE,
                "seed {seed}: a tile appears in only {worst} words\n{:?}",
                (0..SIZE)
                    .map(|r| (0..SIZE).map(|c| grid[r][c].letters).collect::<String>())
                    .collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn a_dead_tile_fails_the_quality_bar() {
        let dict = Dictionary::new();
        // A board of nothing but z has no words at all, so every tile is dead.
        let grid = board(["zzzz", "zzzz", "zzzz", "zzzz"]);
        let words = solve_board(&dict, &grid);
        assert_eq!(worst_tile(&words), 0);
        assert!(!board_is_good(&words));
    }

    #[test]
    fn different_seeds_build_different_boards() {
        let dict = Dictionary::new();
        let letters = |seed| {
            let (g, _, _) = generate_board_seeded(&dict, seed);
            (0..SIZE)
                .map(|r| (0..SIZE).map(|c| g[r][c].letters).collect::<String>())
                .collect::<Vec<_>>()
                .join("/")
        };
        let boards: std::collections::HashSet<String> = (0..20).map(letters).collect();
        // Infinite boards means consecutive rounds must not repeat themselves.
        assert!(boards.len() >= 19, "only {} distinct boards from 20 seeds", boards.len());
    }

    #[test]
    fn a_round_is_three_minutes_then_half_a_minute_of_results() {
        assert_eq!(ROUND_SECONDS, 180.0);
        assert_eq!(RESULTS_SECONDS, 30.0);
        // One three-and-a-half-minute cycle: what the server's clock ticks on too.
        assert_eq!(ROUND_SECONDS + RESULTS_SECONDS, 210.0);
    }

    #[test]
    fn board_type_names_the_theme_when_there_is_one() {
        let mut game = Game::new();
        game.grid = board(["herd", "zzzz", "zzzz", "zzzz"]);
        game.words = solve_board(&game.dictionary, &game.grid);

        // Untethered from a theme, the type describes the board's shape.
        game.theme = None;
        assert!(
            ["Superword", "Long", "Dense", "Sparse", "Standard"].contains(&game.board_type().as_str()),
            "unexpected board type {}",
            game.board_type()
        );

        // With a theme, the theme names it.
        game.theme = Some("Animals".to_string());
        assert_eq!(game.board_type(), "Animals");
        assert_eq!(game.highlight_tab().0, "Animals");

        // A nine-letter word is long, not a superword; only filling the board is.
        game.theme = None;
        game.words.common.insert(0, "aeroplane".to_string());
        assert_ne!(game.board_type(), "Superword");
        game.words.common.insert(0, "characterization".to_string());
        assert_eq!(game.board_type(), "Superword");
        assert_eq!(game.highlight_tab().0, "Superword");
    }

    #[test]
    fn duplicate_words_do_not_score_twice() {
        let dict = Dictionary::new();
        let mut game = Game::new();
        game.dictionary = dict;
        game.grid = board(["herd", "zzzz", "zzzz", "zzzz"]);
        game.phase = Phase::Playing;

        let trace = [(0, 0), (0, 1), (0, 2), (0, 3)];
        for _ in 0..2 {
            for (row, col) in trace {
                game.extend_path(Position { row, col });
            }
            game.submit_path();
        }

        assert_eq!(game.found.len(), 1);
        assert_eq!(game.score, word_points(4) + letter_bonus(&game.tile_uses));
        assert_eq!(game.feedback, Feedback::Duplicate);
    }
}

/// Is this board worth playing? Rich enough, with a long word or two, and with
/// no letter so stranded that it cannot be reused.
pub fn board_is_good(words: &BoardWords) -> bool {
    let long = words.common.iter().filter(|w| w.len() >= LONG_WORD_LEN).count();
    words.common.len() >= MIN_SOLUTIONS
        && long >= MIN_LONG_SOLUTIONS
        && worst_tile(words) >= MIN_TILE_COVERAGE
}

/// The least-used tile on the board.
pub fn worst_tile(words: &BoardWords) -> u32 {
    words.coverage.iter().flatten().copied().min().unwrap_or(0)
}

// --- seed to grid, without a dictionary ------------------------------------

/// Build the grid for a seed using no word list at all.
///
/// The server has to rebuild the same grid to check a submitted score, and
/// shipping 4.8MB of word lists into a Worker is not on. So the seed decides the
/// grid through planting and dice alone, and playability is judged by a cheap
/// letter-shape heuristic rather than by solving the board. The client still
/// solves it afterwards -- it has the dictionary -- but that solve no longer
/// influences which grid the seed produces.
pub fn derive_grid(seed: u64) -> [[Cell; SIZE]; SIZE] {
    let mut rng = StdRng::seed_from_u64(seed);
    let mut fallback: Option<[[Cell; SIZE]; SIZE]> = None;

    for _ in 0..DERIVE_ATTEMPTS {
        let grid = fill_grid(Default::default(), &mut rng);
        if looks_playable(&grid) {
            return grid;
        }
        fallback.get_or_insert(grid);
    }

    fallback.expect("at least one grid was derived")
}

pub fn fill_grid(
    cells: [[Option<&'static str>; SIZE]; SIZE],
    rng: &mut impl Rng,
) -> [[Cell; SIZE]; SIZE] {
    std::array::from_fn(|row| {
        std::array::from_fn(|col| Cell {
            letters: cells[row][col].unwrap_or_else(|| roll_face(rng)),
        })
    })
}

/// A stand-in for solving the board: boards starved of vowels or stuffed with
/// awkward letters are the ones that come out thin, and both are visible from the
/// letters alone.
fn looks_playable(grid: &[[Cell; SIZE]; SIZE]) -> bool {
    let mut vowels = 0;
    let mut awkward = 0;
    for row in grid {
        for cell in row {
            for letter in cell.letters.bytes() {
                match letter {
                    b'a' | b'e' | b'i' | b'o' | b'u' => vowels += 1,
                    b'j' | b'q' | b'v' | b'x' | b'z' => awkward += 1,
                    _ => {}
                }
            }
        }
    }
    (4..=8).contains(&vowels) && awkward <= 2
}

// --- themed board construction ---------------------------------------------

/// Trace `word` across the grid, writing its letters into empty cells and reusing
/// any that already match. Returns false and leaves the grid untouched if the word
/// cannot be laid out along adjacent cells.
pub fn plant_word(
    grid: &mut [[Option<&'static str>; SIZE]; SIZE],
    word: &str,
    rng: &mut impl Rng,
) -> bool {
    let letters: Vec<&'static str> = word
        .bytes()
        .map(|b| LETTERS[(b - b'a') as usize])
        .collect();

    let mut starts: Vec<Position> = (0..SIZE)
        .flat_map(|row| (0..SIZE).map(move |col| Position { row, col }))
        .collect();
    starts.shuffle(rng);

    for start in starts {
        if extend_plant(grid, &letters, 0, start, 0, rng) {
            return true;
        }
    }
    false
}

fn extend_plant(
    grid: &mut [[Option<&'static str>; SIZE]; SIZE],
    letters: &[&'static str],
    idx: usize,
    at: Position,
    visited: u16,
    rng: &mut impl Rng,
) -> bool {
    let bit = 1u16 << (at.row * SIZE + at.col);
    if visited & bit != 0 {
        return false; // a word cannot reuse a tile
    }

    let want = letters[idx];
    let claimed = match grid[at.row][at.col] {
        Some(existing) if existing == want => false, // share a tile already spelling this
        Some(_) => return false,
        None => {
            grid[at.row][at.col] = Some(want);
            true
        }
    };

    if idx + 1 == letters.len() {
        return true;
    }

    let mut next = neighbours(at);
    next.shuffle(rng);
    for step in next {
        if extend_plant(grid, letters, idx + 1, step, visited | bit, rng) {
            return true;
        }
    }

    if claimed {
        grid[at.row][at.col] = None; // nothing downstream worked; give the tile back
    }
    false
}

fn neighbours(at: Position) -> Vec<Position> {
    let mut out = Vec::with_capacity(8);
    for dr in -1i32..=1 {
        for dc in -1i32..=1 {
            if dr == 0 && dc == 0 {
                continue;
            }
            let (r, c) = (at.row as i32 + dr, at.col as i32 + dc);
            if r >= 0 && c >= 0 && r < SIZE as i32 && c < SIZE as i32 {
                out.push(Position { row: r as usize, col: c as usize });
            }
        }
    }
    out
}

pub fn collect_theme_words(words: &BoardWords, theme: &Theme) -> Vec<String> {
    let mut found: Vec<String> = words
        .common
        .iter()
        .chain(words.obscure.iter())
        .filter(|w| theme.all.contains(*w))
        .cloned()
        .collect();
    found.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
    found.dedup();
    found
}

fn roll_face(rng: &mut impl Rng) -> &'static str {
    let die = DICE[rng.gen_range(0..DICE.len())];
    die[rng.gen_range(0..6)]
}
