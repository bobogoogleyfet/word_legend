//! WordHero-style round logic: trace adjacent letters, score by word length,
//! beat the clock.

use crate::dictionary::{Dictionary, MIN_WORD_LEN};
use crate::league::{Save, Season, SeasonOutcome};
use rand::seq::SliceRandom;
use rand::Rng;
use std::collections::HashSet;

pub const SIZE: usize = 4;
pub const ROUND_SECONDS: f32 = 120.0;

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
const MIN_SOLUTIONS: usize = 60;
const MIN_LONG_SOLUTIONS: usize = 4;
const LONG_WORD_LEN: usize = 6;
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

#[derive(Clone, Copy, PartialEq, Eq)]
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
}

pub struct Game {
    pub dictionary: Dictionary,
    pub grid: [[Cell; SIZE]; SIZE],
    pub path: Vec<Position>,
    pub found: Vec<FoundWord>,
    found_set: HashSet<String>,
    /// Every common word hiding on the current board, longest first.
    pub solutions: Vec<String>,
    pub score: u32,
    pub time_left: f32,
    pub phase: Phase,
    pub best_score: u32,
    pub season: Season,
    /// Set when the round just played closed out a season; applied on the next start.
    pub season_outcome: Option<SeasonOutcome>,
    pub is_dragging: bool,

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
        let (grid, solutions) = generate_board(&dictionary);
        let save = Save::load();

        Game {
            dictionary,
            grid,
            path: Vec::new(),
            found: Vec::new(),
            found_set: HashSet::new(),
            solutions,
            score: 0,
            time_left: ROUND_SECONDS,
            phase: Phase::Ready,
            best_score: save.best_score,
            season: save.season,
            season_outcome: None,
            is_dragging: false,
            feedback: Feedback::None,
            feedback_word: String::new(),
            feedback_points: 0,
            feedback_timer: 0.0,
            feedback_cells: Vec::new(),
        }
    }

    pub fn start_round(&mut self) {
        // A finished season is settled here rather than at the final whistle, so the
        // scorecard can show the final table before the ladder moves.
        if let Some(outcome) = self.season_outcome.take() {
            self.season = Season::new(outcome.to);
            self.persist();
        }

        let (grid, solutions) = generate_board(&self.dictionary);
        self.grid = grid;
        self.solutions = solutions;
        self.path.clear();
        self.found.clear();
        self.found_set.clear();
        self.score = 0;
        self.time_left = ROUND_SECONDS;
        self.phase = Phase::Playing;
        self.is_dragging = false;
        self.clear_feedback();
    }

    pub fn tick(&mut self, dt: f32) {
        if self.feedback_timer > 0.0 {
            self.feedback_timer -= dt;
            if self.feedback_timer <= 0.0 {
                self.clear_feedback();
            }
        }

        if self.phase != Phase::Playing {
            return;
        }

        self.time_left -= dt;
        if self.time_left <= 0.0 {
            self.time_left = 0.0;
            self.end_round();
        }
    }

    fn end_round(&mut self) {
        self.phase = Phase::Over;
        self.path.clear();
        self.is_dragging = false;
        self.best_score = self.best_score.max(self.score);

        self.season.record_round(self.score, self.board_par());
        if self.season.is_complete() {
            self.season_outcome = Some(self.season.outcome());
        }
        self.persist();
    }

    fn persist(&self) {
        Save::store(self.best_score, &self.season);
    }

    /// Everything the board is worth if every common word on it is found.
    pub fn board_par(&self) -> u32 {
        self.solutions.iter().map(|w| word_points(w.len())).sum()
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

        let points = word_points(word.len());
        self.score += points;
        self.found_set.insert(word.clone());
        self.found.push(FoundWord { word: word.clone(), points });
        self.set_feedback(Feedback::Accepted, word, points, cells);
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

    pub fn longest_found(&self) -> Option<&FoundWord> {
        self.found.iter().max_by_key(|w| w.word.len())
    }

    /// Words on the board the player never traced, longest first.
    pub fn missed_words(&self, limit: usize) -> Vec<&String> {
        self.solutions
            .iter()
            .filter(|w| !self.found_set.contains(*w))
            .take(limit)
            .collect()
    }

    pub fn rank(&self) -> &'static str {
        match self.score {
            s if s >= 12_000 => "WORD HERO",
            s if s >= 8_000 => "Word Master",
            s if s >= 5_000 => "Wordsmith",
            s if s >= 2_500 => "Apprentice",
            s if s >= 800 => "Novice",
            _ => "Rookie",
        }
    }

    pub fn is_new_best(&self) -> bool {
        self.score > 0 && self.score >= self.best_score
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

pub fn is_adjacent(a: Position, b: Position) -> bool {
    let dr = (a.row as i32 - b.row as i32).abs();
    let dc = (a.col as i32 - b.col as i32).abs();
    dr <= 1 && dc <= 1 && (dr != 0 || dc != 0)
}

// --- board generation ------------------------------------------------------

/// Roll boards until one is rich enough to play, keeping the best seen so far so
/// this always terminates with something reasonable.
fn generate_board(dict: &Dictionary) -> ([[Cell; SIZE]; SIZE], Vec<String>) {
    let mut rng = rand::thread_rng();
    let mut best: Option<([[Cell; SIZE]; SIZE], Vec<String>, usize)> = None;

    for _ in 0..MAX_BOARD_ATTEMPTS {
        let grid = roll_dice(&mut rng);
        let solutions = solve_board(dict, &grid);
        let long = solutions.iter().filter(|w| w.len() >= LONG_WORD_LEN).count();

        if solutions.len() >= MIN_SOLUTIONS && long >= MIN_LONG_SOLUTIONS {
            return (grid, solutions);
        }

        let quality = solutions.len() + long * 10;
        if best.as_ref().is_none_or(|(_, _, q)| quality > *q) {
            best = Some((grid, solutions, quality));
        }
    }

    let (grid, solutions, _) = best.expect("at least one board was rolled");
    (grid, solutions)
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

/// Every common word reachable by tracing adjacent tiles, returned longest first.
/// Obscure words are still accepted in play -- they just don't appear here.
pub fn solve_board(dict: &Dictionary, grid: &[[Cell; SIZE]; SIZE]) -> Vec<String> {
    let mut found = HashSet::new();
    let mut word = String::new();

    for row in 0..SIZE {
        for col in 0..SIZE {
            walk(dict, grid, row, col, 0, dict.root(), &mut word, &mut found);
        }
    }

    let mut words: Vec<String> = found.into_iter().collect();
    words.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
    words
}

fn walk(
    dict: &Dictionary,
    grid: &[[Cell; SIZE]; SIZE],
    row: usize,
    col: usize,
    visited: u16,
    node: u32,
    word: &mut String,
    found: &mut HashSet<String>,
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

    // Only the curated tier counts as a "word on the board": the board gate and
    // the end-of-round scorecard both read this, and neither should cite junk.
    if word.len() >= MIN_WORD_LEN && dict.is_common_node(node) {
        found.insert(word.clone());
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
                walk(dict, grid, r as usize, c as usize, visited, node, word, found);
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
        assert!(solve_board(&dict, &common).iter().any(|w| w == "wars"));

        // "abac" is accepted when traced, but is not scorecard material.
        let obscure = board(["abac", "zzzz", "zzzz", "zzzz"]);
        assert!(dict.contains("abac"));
        assert!(!solve_board(&dict, &obscure).iter().any(|w| w == "abac"));
    }

    #[test]
    fn solver_finds_traceable_words_only() {
        let dict = Dictionary::new();
        // "herd" traces across the top row; "hero" would need a non-adjacent jump.
        let grid = board(["herd", "zzzz", "zzzo", "zzzz"]);
        let words = solve_board(&dict, &grid);
        assert!(words.iter().any(|w| w == "herd"));
        assert!(!words.iter().any(|w| w == "hero"));
    }

    #[test]
    fn solver_returns_longest_first() {
        let dict = Dictionary::new();
        let grid = board(["herd", "zzzz", "zzzz", "zzzz"]);
        let words = solve_board(&dict, &grid);
        assert!(words.windows(2).all(|w| w[0].len() >= w[1].len()));
    }

    #[test]
    fn generated_boards_are_word_rich() {
        let dict = Dictionary::new();
        let (_, solutions) = generate_board(&dict);
        assert!(solutions.len() >= MIN_SOLUTIONS, "got {} words", solutions.len());
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
        assert_eq!(game.score, word_points(4));
        assert_eq!(game.feedback, Feedback::Duplicate);
    }
}
