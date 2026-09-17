//! Ranking: six leagues, and a rolling average that moves you between them.
//!
//! Your rank is the average of your last ten rounds, not your last one. A single
//! terrible board should not cost a league, and a single lucky one should not win
//! it -- averaging over ten lets good and bad rounds cancel out.

use crate::storage;
use std::collections::VecDeque;

/// How many recent rounds the rank average is taken over.
pub const FORM_GAMES: usize = 10;
/// Rounds needed before the ladder will move you, so one good round cannot
/// launch a new player straight to the top.
pub const MIN_GAMES_TO_MOVE: usize = 5;
/// Games kept for the stats chart, oldest dropped first.
pub const HISTORY_GAMES: usize = 50;

/// One banked game, as the stats chart draws it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GameRecord {
    pub score: u32,
    pub words: u32,
    /// The rank average straight after this game.
    pub average: u32,
}

/// Everything ever banked, for the stats page.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Lifetime {
    pub games: u64,
    pub score: u64,
    pub words: u64,
    pub most_words: u32,
    /// The best-scoring word ever found, and what it was worth.
    pub best_word: Option<(String, u32)>,
}

pub struct LeagueDef {
    pub name: &'static str,
    /// Average score needed to sit in this league.
    pub threshold: u32,
}

/// Each league asks more than the last, and the gaps widen toward the top, so the
/// upper leagues are for players who are consistently very good rather than ones
/// who had a lucky run. Set with superwords at 20,000 every fifth round and the
/// letter bonus in mind: finding every superword lifts an average by about 4,000,
/// so Hero still takes 26,000 a round from everything else. Retune with real
/// scores once there are enough of them.
pub const LEAGUES: [LeagueDef; 6] = [
    LeagueDef { name: "Bronze", threshold: 0 },
    LeagueDef { name: "Silver", threshold: 4_000 },
    LeagueDef { name: "Gold", threshold: 8_000 },
    LeagueDef { name: "Platinum", threshold: 13_000 },
    LeagueDef { name: "Diamond", threshold: 20_000 },
    LeagueDef { name: "Hero", threshold: 30_000 },
];

pub const TOP_LEAGUE: usize = LEAGUES.len() - 1;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Movement {
    Promoted,
    Held,
    Relegated,
}

#[derive(Clone, Copy)]
pub struct RankChange {
    pub movement: Movement,
    pub from: usize,
    pub to: usize,
    pub average: u32,
}

impl RankChange {
    /// "Promoted to GOLD" or "Dropped to SILVER". In words: an arrow between the
    /// two leagues is not in every phone's font and drew as an empty box, and the
    /// average is already on the scorecard.
    pub fn headline(&self) -> String {
        match self.movement {
            Movement::Promoted => format!("Promoted to {}", LEAGUES[self.to].name.to_uppercase()),
            Movement::Relegated => format!("Dropped to {}", LEAGUES[self.to].name.to_uppercase()),
            Movement::Held => String::new(),
        }
    }
}

pub struct Ranking {
    pub league: usize,
    /// Recent round scores, oldest first, at most `FORM_GAMES` of them.
    pub recent: VecDeque<u32>,
    pub best_score: u32,
    /// The last `HISTORY_GAMES` games, oldest first.
    pub history: VecDeque<GameRecord>,
    pub lifetime: Lifetime,
}

impl Ranking {
    pub fn new() -> Self {
        Ranking {
            league: 0,
            recent: VecDeque::new(),
            best_score: 0,
            history: VecDeque::new(),
            lifetime: Lifetime::default(),
        }
    }

    /// Bank a played round: the rank, and everything the stats page shows.
    pub fn record_round(&mut self, score: u32, words: u32, best_word: Option<(&str, u32)>) -> RankChange {
        let change = self.record(score);

        self.history.push_back(GameRecord { score, words, average: change.average });
        while self.history.len() > HISTORY_GAMES {
            self.history.pop_front();
        }

        let life = &mut self.lifetime;
        life.games += 1;
        life.score += u64::from(score);
        life.words += u64::from(words);
        life.most_words = life.most_words.max(words);
        if let Some((word, points)) = best_word {
            if life.best_word.as_ref().is_none_or(|(_, best)| points > *best) {
                life.best_word = Some((word.to_string(), points));
            }
        }
        change
    }

    /// The rank itself: the mean of the rounds on record.
    pub fn average(&self) -> u32 {
        if self.recent.is_empty() {
            return 0;
        }
        let total: u64 = self.recent.iter().map(|s| *s as u64).sum();
        (total / self.recent.len() as u64) as u32
    }

    pub fn games_on_record(&self) -> usize {
        self.recent.len()
    }

    /// Bank a round and move up or down if the new average calls for it.
    pub fn record(&mut self, score: u32) -> RankChange {
        self.recent.push_back(score);
        while self.recent.len() > FORM_GAMES {
            self.recent.pop_front();
        }
        self.best_score = self.best_score.max(score);

        let average = self.average();
        let from = self.league;

        // Promotion waits for enough rounds to judge; relegation does not. An
        // average below the league's floor drops a league straight away.
        let can_promote = self.recent.len() >= MIN_GAMES_TO_MOVE;

        if can_promote && self.league < TOP_LEAGUE && average >= LEAGUES[self.league + 1].threshold {
            self.league += 1;
            return RankChange {
                movement: Movement::Promoted,
                from,
                to: self.league,
                average,
            };
        }
        if self.league > 0 && average < LEAGUES[self.league].threshold {
            self.league -= 1;
            return RankChange {
                movement: Movement::Relegated,
                from,
                to: self.league,
                average,
            };
        }
        RankChange { movement: Movement::Held, from, to: from, average }
    }

    /// The rank average after each game in the history: the mean of that game and
    /// up to nine before it. The first game's average is its own score.
    pub fn history_averages(&self) -> Vec<u32> {
        let scores: Vec<u64> = self.history.iter().map(|g| u64::from(g.score)).collect();
        (0..scores.len())
            .map(|i| {
                let window = &scores[(i + 1).saturating_sub(FORM_GAMES)..=i];
                (window.iter().sum::<u64>() / window.len() as u64) as u32
            })
            .collect()
    }

    /// A league the average no longer holds up is left: one league at a time,
    /// until the average is at or above the league's floor.
    fn settle_league(&mut self) {
        while self.league > 0 && !self.recent.is_empty() && self.average() < LEAGUES[self.league].threshold {
            self.league -= 1;
        }
    }

    pub fn next_threshold(&self) -> Option<u32> {
        (self.league < TOP_LEAGUE).then(|| LEAGUES[self.league + 1].threshold)
    }

    /// How far the average has come from this league's floor toward the next.
    pub fn progress(&self) -> f32 {
        let Some(next) = self.next_threshold() else { return 1.0 };
        let floor = LEAGUES[self.league].threshold;
        if next <= floor {
            return 1.0;
        }
        ((self.average().saturating_sub(floor)) as f32 / (next - floor) as f32).clamp(0.0, 1.0)
    }

    // --- persistence -------------------------------------------------------

    pub fn load() -> Self {
        let Some(text) = storage::read(storage::SAVE) else {
            let mut fresh = Ranking::new();
            fresh.best_score = legacy_best();
            return fresh;
        };
        Ranking::from_text(&text)
    }

    /// Read a save. Older saves lack the history and lifetime lines; they load
    /// with those empty, and everything they did have intact.
    pub fn from_text(text: &str) -> Self {
        let mut rank = Ranking::new();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else { continue };
            let number = || value.trim().parse::<u64>().unwrap_or(0);
            match key {
                "best" => rank.best_score = value.parse().unwrap_or(0),
                "league" => rank.league = value.parse::<usize>().unwrap_or(0).min(TOP_LEAGUE),
                "recent" => {
                    rank.recent = value
                        .split(',')
                        .filter_map(|s| s.trim().parse().ok())
                        .collect();
                    while rank.recent.len() > FORM_GAMES {
                        rank.recent.pop_front();
                    }
                }
                "history" => {
                    rank.history = value
                        .split(',')
                        .filter_map(|game| {
                            let mut parts = game.split(':').map(|p| p.trim().parse::<u32>().ok());
                            Some(GameRecord { score: parts.next()??, words: parts.next()??, average: parts.next()?? })
                        })
                        .collect();
                    while rank.history.len() > HISTORY_GAMES {
                        rank.history.pop_front();
                    }
                }
                "games" => rank.lifetime.games = number(),
                "total_score" => rank.lifetime.score = number(),
                "total_words" => rank.lifetime.words = number(),
                "most_words" => rank.lifetime.most_words = number() as u32,
                "best_word" => {
                    rank.lifetime.best_word = value
                        .rsplit_once(':')
                        .and_then(|(word, points)| Some((word.to_string(), points.parse().ok()?)))
                        .filter(|(word, _)| !word.is_empty());
                }
                _ => {}
            }
        }

        // Saves from before the history kept only the last ten scores. Carry those
        // into the history, so the chart starts where the rank does and its first
        // game's average is that game's own score.
        if rank.history.len() < rank.recent.len() {
            let missing = rank.recent.len() - rank.history.len();
            for &score in rank.recent.iter().take(missing).rev() {
                rank.history.push_front(GameRecord { score, words: 0, average: score });
                rank.lifetime.games += 1;
                rank.lifetime.score += u64::from(score);
            }
            let averages = rank.history_averages();
            for (game, average) in rank.history.iter_mut().zip(averages) {
                game.average = average;
            }
        }

        rank.settle_league();
        rank
    }

    pub fn to_text(&self) -> String {
        let recent: Vec<String> = self.recent.iter().map(|s| s.to_string()).collect();
        let history: Vec<String> =
            self.history.iter().map(|g| format!("{}:{}:{}", g.score, g.words, g.average)).collect();
        let life = &self.lifetime;
        let best_word = life.best_word.as_ref().map(|(w, p)| format!("{w}:{p}")).unwrap_or_default();
        format!(
            "version=3\nbest={}\nleague={}\nrecent={}\nhistory={}\ngames={}\ntotal_score={}\ntotal_words={}\nmost_words={}\nbest_word={}\n",
            self.best_score,
            self.league,
            recent.join(","),
            history.join(","),
            life.games,
            life.score,
            life.words,
            life.most_words,
            best_word,
        )
    }

    pub fn store(&self) {
        storage::write(storage::SAVE, &self.to_text());
    }
}

fn legacy_best() -> u32 {
    storage::read(storage::LEGACY_BEST)
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ranked(league: usize, scores: &[u32]) -> Ranking {
        let mut r = Ranking::new();
        r.league = league;
        r.recent = scores.iter().copied().collect();
        r
    }

    #[test]
    fn falling_below_the_floor_drops_a_league_without_waiting_for_five_rounds() {
        let mut r = ranked(2, &[9_000]);
        let change = r.record(1_000);
        assert_eq!(change.movement, Movement::Relegated, "an average under Gold's floor kept Gold");
        assert_eq!(r.league, 1);
    }

    #[test]
    fn a_saved_league_the_average_no_longer_holds_is_left_on_load() {
        let r = Ranking::from_text("version=3\nbest=9000\nleague=3\nrecent=5000,5000\n");
        assert_eq!(r.league, 1, "Platinum on paper with a 5,000 average should settle in Silver");
        let r = Ranking::from_text("version=3\nleague=3\nrecent=\n");
        assert_eq!(r.league, 3, "no rounds on record is no evidence to move on");
    }

    #[test]
    fn the_first_game_on_the_chart_averages_to_its_own_score() {
        let mut r = Ranking::new();
        for score in [3_000, 5_000, 1_000] {
            r.record_round(score, 10, None);
        }
        assert_eq!(r.history_averages(), [3_000, 4_000, 3_000]);

        // A save from before the history: its recent scores become the history.
        let old = Ranking::from_text("version=2\nleague=0\nrecent=2000,4000,6000\n");
        assert_eq!(old.history.iter().map(|g| g.score).collect::<Vec<_>>(), [2_000, 4_000, 6_000]);
        assert_eq!(old.history_averages(), [2_000, 3_000, 4_000]);
        assert_eq!(old.history_averages().last().copied(), Some(old.average()), "the chart ends where the rank is");

        // A save whose history began after its recent scores: the older ones are
        // carried in front, so the first point is a game averaged with itself.
        let mut mixed = Ranking::from_text("version=2\nleague=0\nrecent=8000,9000\n");
        mixed.record_round(1_000, 5, None);
        let back = Ranking::from_text(&mixed.to_text().replace("history=8000:0:8000,9000:0:8500,", "history="));
        assert_eq!(back.history.front().map(|g| (g.score, g.average)), Some((8_000, 8_000)));
        assert_eq!(back.history_averages().last().copied(), Some(back.average()));
    }

    #[test]
    fn a_played_round_feeds_the_history_and_the_lifetime_totals() {
        let mut r = Ranking::new();
        r.record_round(3_000, 12, Some(("otters", 1_400)));
        r.record_round(5_000, 20, Some(("hat", 100)));
        assert_eq!(
            r.history.iter().copied().collect::<Vec<_>>(),
            [
                GameRecord { score: 3_000, words: 12, average: 3_000 },
                GameRecord { score: 5_000, words: 20, average: 4_000 },
            ]
        );
        assert_eq!(r.lifetime.games, 2);
        assert_eq!(r.lifetime.score, 8_000);
        assert_eq!(r.lifetime.words, 32);
        assert_eq!(r.lifetime.most_words, 20);
        assert_eq!(r.lifetime.best_word, Some(("otters".to_string(), 1_400)), "a weaker word replaced the best");
    }

    #[test]
    fn history_keeps_only_the_last_fifty_games() {
        let mut r = Ranking::new();
        for i in 1..=60 {
            r.record_round(i * 100, 1, None);
        }
        assert_eq!(r.history.len(), HISTORY_GAMES);
        assert_eq!(r.history.front().unwrap().score, 1_100);
        assert_eq!(r.lifetime.games, 60, "lifetime totals are all-time, not windowed");
    }

    #[test]
    fn a_save_round_trips_and_an_old_save_still_loads() {
        let mut r = Ranking::new();
        r.league = 2;
        r.best_score = 9_100;
        r.record_round(4_000, 11, None);
        r.record_round(6_000, 18, None);
        r.record_round(7_000, 25, Some(("quizzers", 2_200)));
        let back = Ranking::from_text(&r.to_text());
        assert_eq!(back.league, r.league);
        assert_eq!(back.recent, r.recent);
        assert_eq!(back.best_score, 9_100);
        assert_eq!(back.history, r.history);
        assert_eq!(back.lifetime, r.lifetime);

        let old = Ranking::from_text("version=2\nbest=5000\nleague=1\nrecent=1000,2000\n");
        // Silver on paper with a 1,500 average: it settles back into Bronze.
        assert_eq!((old.league, old.best_score, old.recent.len()), (0, 5_000, 2));
        // Its recent scores become the start of its history.
        assert_eq!(old.history.len(), 2);
        assert_eq!(old.lifetime.games, 2);
    }

    #[test]
    fn the_leagues_get_harder_toward_the_top() {
        let gaps: Vec<u32> = LEAGUES.windows(2).map(|w| w[1].threshold - w[0].threshold).collect();
        assert!(gaps.windows(2).all(|g| g[1] >= g[0]), "each league should ask at least as much more as the last: {gaps:?}");
        assert!(LEAGUES[TOP_LEAGUE].threshold >= 30_000, "the top league should be hard to reach");
    }

    #[test]
    fn a_move_is_announced_in_words() {
        let mut up = ranked(0, &[5_000; 9]);
        assert_eq!(up.record(5_000).headline(), "Promoted to SILVER");
        let mut down = ranked(2, &[5_000; 9]);
        assert_eq!(down.record(5_000).headline(), "Dropped to SILVER");
    }

    #[test]
    fn rank_is_the_mean_of_recent_rounds() {
        let r = ranked(0, &[1_000, 2_000, 3_000]);
        assert_eq!(r.average(), 2_000);
    }

    #[test]
    fn only_the_last_ten_rounds_count() {
        let mut r = Ranking::new();
        for i in 1..=15 {
            r.record(i * 1_000);
        }
        assert_eq!(r.games_on_record(), FORM_GAMES);
        // Rounds 6..15 survive; 1..5 have aged out.
        assert_eq!(r.average(), (6..=15).map(|i| i * 1_000).sum::<u32>() / 10);
    }

    #[test]
    fn a_single_bad_round_does_not_cost_a_league() {
        // Comfortably in Gold on average, then one disaster.
        let mut r = ranked(2, &[10_000; 9]);
        let change = r.record(0);
        assert_eq!(change.movement, Movement::Held);
        assert_eq!(r.league, 2);
    }

    #[test]
    fn one_round_moves_the_rank_by_a_tenth_of_its_excess() {
        // This is the whole point of averaging: a round pulls the rank by a tenth
        // of how far it sits from it, so good and bad rounds even out.
        let mut r = ranked(0, &[1_000; 9]);
        assert_eq!(r.average(), 1_000);
        let change = r.record(11_000);
        assert_eq!(change.average, 2_000, "a 10k excess should lift the rank by 1k");
    }

    #[test]
    fn a_good_round_alone_does_not_win_a_league() {
        let mut r = ranked(0, &[500; 9]);
        // Ten times their usual, and the rank still only reaches 950.
        let change = r.record(5_000);
        assert_eq!(change.average, 950);
        assert_eq!(change.movement, Movement::Held);
        assert_eq!(r.league, 0);
    }

    #[test]
    fn a_sustained_run_promotes() {
        let mut r = Ranking::new();
        let mut change = r.record(9_000);
        for _ in 0..5 {
            change = r.record(9_000);
        }
        // 9000 clears Silver (4000) and Gold (8000) but not Platinum (13000).
        assert_eq!(change.movement, Movement::Promoted);
        assert!(r.league >= 1);
        assert!(LEAGUES[r.league].threshold <= 9_000);
    }

    #[test]
    fn falling_below_the_floor_relegates() {
        let mut r = ranked(3, &[1_000; 9]); // Platinum on paper, Bronze in form
        let change = r.record(1_000);
        assert_eq!(change.movement, Movement::Relegated);
        assert_eq!(r.league, 2);
    }

    #[test]
    fn the_ladder_waits_for_enough_rounds() {
        let mut r = Ranking::new();
        for _ in 0..MIN_GAMES_TO_MOVE - 1 {
            assert_eq!(r.record(50_000).movement, Movement::Held);
            assert_eq!(r.league, 0, "moved before there was enough to judge");
        }
        assert_eq!(r.record(50_000).movement, Movement::Promoted);
    }

    #[test]
    fn bronze_cannot_drop_and_hero_cannot_climb() {
        let mut bottom = ranked(0, &[0; 9]);
        assert_eq!(bottom.record(0).movement, Movement::Held);
        assert_eq!(bottom.league, 0);

        let mut top = ranked(TOP_LEAGUE, &[99_000; 9]);
        assert_eq!(top.record(99_000).movement, Movement::Held);
        assert_eq!(top.league, TOP_LEAGUE);
    }

    #[test]
    fn progress_runs_from_this_floor_to_the_next() {
        let r = ranked(1, &[LEAGUES[1].threshold]); // exactly on the Silver floor
        assert_eq!(r.progress(), 0.0);
        let r = ranked(1, &[LEAGUES[2].threshold]); // at the Gold line
        assert_eq!(r.progress(), 1.0);
        let top = ranked(TOP_LEAGUE, &[1_000]);
        assert_eq!(top.progress(), 1.0);
    }
}
