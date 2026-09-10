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

pub struct LeagueDef {
    pub name: &'static str,
    /// Average score needed to sit in this league.
    pub threshold: u32,
}

/// Thresholds are a starting point, not a measurement: they want retuning once
/// there are real scores from real three-minute rounds to look at.
pub const LEAGUES: [LeagueDef; 6] = [
    LeagueDef { name: "Bronze", threshold: 0 },
    LeagueDef { name: "Silver", threshold: 2_500 },
    LeagueDef { name: "Gold", threshold: 5_000 },
    LeagueDef { name: "Platinum", threshold: 8_000 },
    LeagueDef { name: "Diamond", threshold: 12_000 },
    LeagueDef { name: "Hero", threshold: 18_000 },
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
    pub fn headline(&self) -> String {
        match self.movement {
            Movement::Promoted => format!(
                "{} \u{2192} {}  ·  {} avg",
                LEAGUES[self.from].name.to_uppercase(),
                LEAGUES[self.to].name.to_uppercase(),
                self.average
            ),
            Movement::Relegated => format!(
                "{} \u{2192} {}  ·  {} avg",
                LEAGUES[self.from].name.to_uppercase(),
                LEAGUES[self.to].name.to_uppercase(),
                self.average
            ),
            Movement::Held => String::new(),
        }
    }
}

pub struct Ranking {
    pub league: usize,
    /// Recent round scores, oldest first, at most `FORM_GAMES` of them.
    pub recent: VecDeque<u32>,
    pub best_score: u32,
}

impl Ranking {
    pub fn new() -> Self {
        Ranking { league: 0, recent: VecDeque::new(), best_score: 0 }
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

        // Too early to judge anyone.
        if self.recent.len() < MIN_GAMES_TO_MOVE {
            return RankChange { movement: Movement::Held, from, to: from, average };
        }

        if self.league < TOP_LEAGUE && average >= LEAGUES[self.league + 1].threshold {
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

        let mut rank = Ranking::new();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else { continue };
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
                _ => {}
            }
        }
        rank
    }

    pub fn store(&self) {
        let recent: Vec<String> = self.recent.iter().map(|s| s.to_string()).collect();
        let out = format!(
            "version=2\nbest={}\nleague={}\nrecent={}\n",
            self.best_score,
            self.league,
            recent.join(",")
        );
        storage::write(storage::SAVE, &out);
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
        let mut r = ranked(2, &[6_000; 9]);
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
        let mut change = r.record(6_000);
        for _ in 0..5 {
            change = r.record(6_000);
        }
        // 6000 clears Silver (2500) and Gold (5000) but not Platinum (8000).
        assert_eq!(change.movement, Movement::Promoted);
        assert!(r.league >= 1);
        assert!(LEAGUES[r.league].threshold <= 6_000);
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
