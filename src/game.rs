//! WordHero-style round logic: trace adjacent letters, score by word length,
//! beat the clock.

use crate::dictionary::{Dictionary, MIN_WORD_LEN};
use crate::league::{Save, Season, SeasonOutcome};
use crate::themes::{Theme, Themes};
use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::{Rng, SeedableRng};
use std::collections::HashSet;

pub const SIZE: usize = 4;
/// A live round. Everyone playing is on the same one at the same time.
pub const ROUND_SECONDS: f32 = 180.0;
/// How long the scorecard holds before the next round begins.
pub const RESULTS_SECONDS: f32 = 60.0;
/// One full cycle: play, then read the results.
pub const CYCLE_SECONDS: f32 = ROUND_SECONDS + RESULTS_SECONDS;

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
/// The bar for a board-defining "superword".
pub const SUPERWORD_LEN: usize = 7;
/// How many words from a category a board needs before it counts as themed.
const THEME_TARGET: usize = 3;
/// Tries at planting a theme before falling back to a plain roll.
const THEME_ATTEMPTS: usize = 60;

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
    /// Every word hiding on the current board, split by tier.
    pub words: BoardWords,
    /// How many times each tile was used across the words you found.
    pub tile_uses: [[u32; SIZE]; SIZE],
    /// The category this board was built around, if any.
    pub theme: Option<String>,
    themes: Themes,
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
        let themes = Themes::load();
        let (grid, words, theme) = generate_board(&dictionary, &themes);
        let save = Save::load();

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

        let (grid, words, theme) = generate_board(&self.dictionary, &self.themes);
        self.grid = grid;
        self.words = words;
        self.theme = theme;
        self.tile_uses = [[0; SIZE]; SIZE];
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
        for at in &cells {
            self.tile_uses[at.row][at.col] += 1;
        }
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

    /// Everything the board was worth: the score if every common word were found.
    pub fn board_par(&self) -> u32 {
        self.words.common.iter().map(|w| word_points(w.len())).sum()
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

    pub fn average_points(&self) -> f32 {
        if self.found.is_empty() {
            return 0.0;
        }
        self.score as f32 / self.found.len() as f32
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

    /// A word long enough to define the board it sits on.
    pub fn superwords(&self) -> Vec<&String> {
        self.words
            .common
            .iter()
            .filter(|w| w.len() >= SUPERWORD_LEN)
            .collect()
    }

    /// The board's theme words, or its superwords when it has no theme.
    pub fn theme_words(&self) -> Vec<&String> {
        if self.words.theme.is_empty() {
            self.superwords()
        } else {
            self.words.theme.iter().collect()
        }
    }

    /// What kind of board this is: its theme if it has one, otherwise a shape read
    /// off its own solution set.
    pub fn board_type(&self) -> String {
        if let Some(theme) = &self.theme {
            // A themed board that also hides a long word is worth calling out.
            return match self.words.common.first().map(|w| w.len()).unwrap_or(0) {
                n if n >= SUPERWORD_LEN => format!("{theme} · Superword"),
                _ => theme.clone(),
            };
        }

        let longest = self.words.common.first().map(|w| w.len()).unwrap_or(0);
        let count = self.words.common.len();
        match () {
            _ if longest >= SUPERWORD_LEN + 1 => "Superword".to_string(),
            _ if longest >= SUPERWORD_LEN => "Long".to_string(),
            _ if count >= 120 => "Dense".to_string(),
            _ if count < 70 => "Sparse".to_string(),
            _ => "Standard".to_string(),
        }
    }

    pub fn has_found(&self, word: &str) -> bool {
        self.found_set.contains(word)
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
fn generate_board(
    dict: &Dictionary,
    themes: &Themes,
) -> ([[Cell; SIZE]; SIZE], BoardWords, Option<String>) {
    generate_board_seeded(dict, themes, rand::random::<u64>())
}

/// Build the board for a given seed.
///
/// Synchronised play needs every client to arrive at the same board without the
/// board itself being sent, so generation must be a pure function of the seed:
/// no thread RNG, and every collection sorted into a total order before use.
pub fn generate_board_seeded(
    dict: &Dictionary,
    themes: &Themes,
    seed: u64,
) -> ([[Cell; SIZE]; SIZE], BoardWords, Option<String>) {
    let mut rng = StdRng::seed_from_u64(seed);

    // Build a themed board when we can; a plain roll is the fallback, not the plan.
    if let Some(theme) = themes.list.choose(&mut rng) {
        if let Some((grid, words)) = build_themed_board(dict, theme, &mut rng) {
            return (grid, words, Some(theme.name.clone()));
        }
    }

    let mut best: Option<([[Cell; SIZE]; SIZE], BoardWords, usize)> = None;

    for _ in 0..MAX_BOARD_ATTEMPTS {
        let grid = roll_dice(&mut rng);
        let words = solve_board(dict, &grid);
        let long = words.common.iter().filter(|w| w.len() >= LONG_WORD_LEN).count();

        if words.common.len() >= MIN_SOLUTIONS && long >= MIN_LONG_SOLUTIONS {
            return (grid, words, None);
        }

        let quality = words.common.len() + long * 10;
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
}

/// Every word reachable by tracing adjacent tiles, each list longest first.
pub fn solve_board(dict: &Dictionary, grid: &[[Cell; SIZE]; SIZE]) -> BoardWords {
    let mut common = HashSet::new();
    let mut obscure = HashSet::new();
    let mut word = String::new();

    for row in 0..SIZE {
        for col in 0..SIZE {
            walk(dict, grid, row, col, 0, dict.root(), &mut word, &mut common, &mut obscure);
        }
    }

    let sorted = |set: HashSet<String>| {
        let mut v: Vec<String> = set.into_iter().collect();
        v.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
        v
    };
    BoardWords { common: sorted(common), obscure: sorted(obscure), theme: Vec::new() }
}

fn walk(
    dict: &Dictionary,
    grid: &[[Cell; SIZE]; SIZE],
    row: usize,
    col: usize,
    visited: u16,
    node: u32,
    word: &mut String,
    common: &mut HashSet<String>,
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
            common.insert(word.clone());
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

        // "abac" is accepted when traced, but is not scorecard material: it belongs
        // in the obscure list, not the common one.
        let grid = board(["abac", "zzzz", "zzzz", "zzzz"]);
        let words = solve_board(&dict, &grid);
        assert!(dict.contains("abac"));
        assert!(!words.common.iter().any(|w| w == "abac"));
        assert!(words.obscure.iter().any(|w| w == "abac"));
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
        let (_, words, _) = generate_board(&dict, &Themes::load());
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

    #[test]
    fn themed_boards_carry_their_theme_and_stay_playable() {
        let dict = Dictionary::new();
        let themes = Themes::load();

        for theme in themes.list.iter().take(6) {
            let mut rng = rand::thread_rng();
            let Some((grid, words)) = build_themed_board(&dict, theme, &mut rng) else {
                panic!("could not build a {} board", theme.name);
            };

            assert!(words.theme.len() >= THEME_TARGET, "{}: too few theme words", theme.name);
            for word in &words.theme {
                assert!(theme.all.contains(word), "{word} is not a {} word", theme.name);
                assert!(is_traceable(&grid, word), "{word} is listed but not traceable");
            }
            // A themed board is still a board: it has to be worth playing.
            assert!(words.common.len() >= MIN_SOLUTIONS, "{}: only {} words", theme.name, words.common.len());
        }
    }

    #[test]
    fn geography_themes_work_despite_being_impossible_by_chance() {
        // Countries turned up on 0 of 300 rolled boards; planting is what makes
        // them viable, so this is the case most worth guarding.
        let dict = Dictionary::new();
        let themes = Themes::load();
        let theme = themes.get("Countries").expect("Countries theme");
        let mut rng = rand::thread_rng();

        let (grid, words) = build_themed_board(&dict, theme, &mut rng).expect("no Countries board");
        assert!(words.theme.len() >= THEME_TARGET);
        for word in &words.theme {
            assert!(is_traceable(&grid, word));
        }
    }

    /// Synchronised play stands on this: two clients given the same seed must build
    /// the identical board, or they are not playing the same game.
    #[test]
    fn a_seed_always_builds_the_same_board() {
        let dict = Dictionary::new();
        let themes = Themes::load();

        for seed in [0u64, 1, 42, 9_999, u64::MAX] {
            let (grid_a, words_a, theme_a) = generate_board_seeded(&dict, &themes, seed);
            let (grid_b, words_b, theme_b) = generate_board_seeded(&dict, &themes, seed);

            let letters = |g: &[[Cell; SIZE]; SIZE]| {
                (0..SIZE)
                    .map(|r| (0..SIZE).map(|c| g[r][c].letters).collect::<String>())
                    .collect::<Vec<_>>()
            };
            assert_eq!(letters(&grid_a), letters(&grid_b), "seed {seed} drifted");
            assert_eq!(theme_a, theme_b, "seed {seed} changed theme");
            assert_eq!(words_a.common, words_b.common, "seed {seed} changed solutions");
            assert_eq!(words_a.theme, words_b.theme, "seed {seed} changed theme words");
        }
    }

    /// Pinned boards. Synchronised clients only agree if generation is byte-stable,
    /// so a change here means old and new clients would silently play different
    /// boards from the same seed -- a deliberate decision, never an accident.
    #[test]
    fn seeded_boards_are_pinned() {
        let dict = Dictionary::new();
        let themes = Themes::load();

        let board_of = |seed| {
            let (g, w, t) = generate_board_seeded(&dict, &themes, seed);
            let rows: Vec<String> = (0..SIZE)
                .map(|r| (0..SIZE).map(|c| g[r][c].letters).collect::<Vec<_>>().join("|"))
                .collect();
            (rows, t, w.common.len())
        };

        let (rows, theme, count) = board_of(42);
        assert_eq!(rows, ["r|l|d|h", "u|e|c|i", "r|l|s|r", "k|n|a|c"]);
        assert_eq!(theme.as_deref(), Some("Tools"));
        assert_eq!(count, 117);

        let (rows, theme, count) = board_of(1234);
        assert_eq!(rows, ["s|a|l|t", "c|h|e|p", "e|m|s|w", "e|t|e|h"]);
        assert_eq!(theme.as_deref(), Some("Food"));
        assert_eq!(count, 131);
    }

    #[test]
    fn different_seeds_build_different_boards() {
        let dict = Dictionary::new();
        let themes = Themes::load();
        let letters = |seed| {
            let (g, _, _) = generate_board_seeded(&dict, &themes, seed);
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
    fn a_round_is_three_minutes_then_a_minute_of_results() {
        assert_eq!(ROUND_SECONDS, 180.0);
        assert_eq!(RESULTS_SECONDS, 60.0);
        assert_eq!(CYCLE_SECONDS, 240.0);
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
        assert!(game.board_type().starts_with("Animals"));
        assert!(game.superwords().iter().all(|w| w.len() >= SUPERWORD_LEN));
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

// --- themed board construction ---------------------------------------------

/// Trace `word` across the grid, writing its letters into empty cells and reusing
/// any that already match. Returns false and leaves the grid untouched if the word
/// cannot be laid out along adjacent cells.
fn plant_word(
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

/// Plant several words from one category, then fill the rest from the dice so the
/// board still plays like a normal one.
fn build_themed_board(
    dict: &Dictionary,
    theme: &Theme,
    rng: &mut impl Rng,
) -> Option<([[Cell; SIZE]; SIZE], BoardWords)> {
    for _ in 0..THEME_ATTEMPTS {
        let mut cells: [[Option<&'static str>; SIZE]; SIZE] = Default::default();
        let mut candidates = theme.plantable.clone();
        candidates.shuffle(rng);

        let mut planted = 0;
        for word in candidates.iter().take(40) {
            if planted >= THEME_TARGET {
                break;
            }
            if plant_word(&mut cells, word, rng) {
                planted += 1;
            }
        }
        if planted < THEME_TARGET {
            continue;
        }

        let grid: [[Cell; SIZE]; SIZE] = std::array::from_fn(|row| {
            std::array::from_fn(|col| Cell {
                letters: cells[row][col].unwrap_or_else(|| roll_face(rng)),
            })
        });

        let mut words = solve_board(dict, &grid);
        let long = words.common.iter().filter(|w| w.len() >= LONG_WORD_LEN).count();
        // A themed board still has to be worth playing.
        if words.common.len() < MIN_SOLUTIONS || long < MIN_LONG_SOLUTIONS {
            continue;
        }

        words.theme = collect_theme_words(&words, theme);
        if words.theme.len() < THEME_TARGET {
            continue; // the planted words must survive as findable words
        }
        return Some((grid, words));
    }
    None
}

fn collect_theme_words(words: &BoardWords, theme: &Theme) -> Vec<String> {
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
