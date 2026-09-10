//! Competitive ladder: six leagues, five-round seasons, promotion and relegation.
//!
//! There is no server, so the table is filled with local rivals. Their scores are
//! a fraction of the board's *par* -- the total on offer if you found every common
//! word -- rather than fixed numbers. A word-rich board lifts everyone, so a lucky
//! draw never hands you the season, and a sparse one never buries you.

use crate::storage;
use rand::seq::SliceRandom;
use rand::Rng;

pub const SEASON_ROUNDS: u32 = 5;
pub const RIVAL_COUNT: usize = 5;
/// Of the six in a table, the top two go up and the bottom two go down.
const PROMOTION_PLACES: usize = 2;
const RELEGATION_PLACES: usize = 2;

pub struct LeagueDef {
    pub name: &'static str,
    /// Share of board par a typical rival here converts.
    pub rival_skill: f32,
}

pub const LEAGUES: [LeagueDef; 6] = [
    LeagueDef { name: "Bronze", rival_skill: 0.080 },
    LeagueDef { name: "Silver", rival_skill: 0.115 },
    LeagueDef { name: "Gold", rival_skill: 0.150 },
    LeagueDef { name: "Platinum", rival_skill: 0.190 },
    LeagueDef { name: "Diamond", rival_skill: 0.240 },
    LeagueDef { name: "Hero", rival_skill: 0.300 },
];

pub const TOP_LEAGUE: usize = LEAGUES.len() - 1;

const RIVAL_NAMES: [&str; 16] = [
    "quillfox", "glyphhound", "verbatim", "syntaxpip", "rookandpen", "letterbox",
    "vowelthief", "anagrammer", "inkwell", "cluecrow", "spellbound", "wordwyrm",
    "lexipedia", "tilehoarder", "pangrams", "scriptor",
];

#[derive(Clone)]
pub struct Rival {
    pub name: String,
    skill: f32,
    pub total: u32,
    pub last: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Movement {
    Promoted,
    Held,
    Relegated,
}

#[derive(Clone, Copy)]
pub struct SeasonOutcome {
    pub place: usize,
    pub movement: Movement,
    pub from: usize,
    pub to: usize,
}

impl SeasonOutcome {
    /// Finishing top two in the highest league is a title defence, not a promotion.
    pub fn defended(&self) -> bool {
        self.movement == Movement::Held && self.from == TOP_LEAGUE && self.place <= PROMOTION_PLACES
    }

    pub fn headline(&self) -> String {
        match self.movement {
            Movement::Promoted => format!("PROMOTED TO {}", LEAGUES[self.to].name.to_uppercase()),
            Movement::Relegated => format!("RELEGATED TO {}", LEAGUES[self.to].name.to_uppercase()),
            Movement::Held if self.defended() => "TITLE DEFENDED".to_string(),
            Movement::Held => format!("STAYING IN {}", LEAGUES[self.to].name.to_uppercase()),
        }
    }
}

pub struct Standing {
    pub place: usize,
    pub name: String,
    pub total: u32,
    pub last: u32,
    pub is_you: bool,
}

pub struct Season {
    pub league: usize,
    /// Rounds already played, so the round in progress is `round + 1`.
    pub round: u32,
    pub your_total: u32,
    pub your_last: u32,
    pub rivals: Vec<Rival>,
}

impl Season {
    pub fn new(league: usize) -> Self {
        let league = league.min(TOP_LEAGUE);
        let mut rng = rand::thread_rng();
        let base = LEAGUES[league].rival_skill;

        let mut names: Vec<&str> = RIVAL_NAMES.to_vec();
        names.shuffle(&mut rng);

        let rivals = names
            .into_iter()
            .take(RIVAL_COUNT)
            .map(|name| Rival {
                name: name.to_string(),
                // Spread rivals around the league's level so the table isn't uniform.
                skill: base * rng.gen_range(0.80..1.20),
                total: 0,
                last: 0,
            })
            .collect();

        Season { league, round: 0, your_total: 0, your_last: 0, rivals }
    }

    pub fn round_number(&self) -> u32 {
        (self.round + 1).min(SEASON_ROUNDS)
    }

    pub fn is_complete(&self) -> bool {
        self.round >= SEASON_ROUNDS
    }

    /// Bank your score and roll the rivals' scores for the same board.
    pub fn record_round(&mut self, your_score: u32, par: u32) {
        if self.is_complete() {
            return;
        }
        let mut rng = rand::thread_rng();

        self.your_last = your_score;
        self.your_total += your_score;

        for rival in &mut self.rivals {
            let jitter = rng.gen_range(0.80..1.20);
            let score = (par as f32 * rival.skill * jitter).round() as u32;
            rival.last = score;
            rival.total += score;
        }

        self.round += 1;
    }

    pub fn standings(&self) -> Vec<Standing> {
        let mut rows: Vec<(String, u32, u32, bool)> = self
            .rivals
            .iter()
            .map(|r| (r.name.clone(), r.total, r.last, false))
            .collect();
        rows.push(("you".to_string(), self.your_total, self.your_last, true));

        // Ties go to you: a drawn season should not relegate the player.
        rows.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| b.3.cmp(&a.3)));

        rows.into_iter()
            .enumerate()
            .map(|(i, (name, total, last, is_you))| Standing {
                place: i + 1,
                name,
                total,
                last,
                is_you,
            })
            .collect()
    }

    pub fn your_place(&self) -> usize {
        self.standings()
            .iter()
            .find(|s| s.is_you)
            .map(|s| s.place)
            .unwrap_or(RIVAL_COUNT + 1)
    }

    /// Where the finished season leaves you. Only meaningful once complete.
    pub fn outcome(&self) -> SeasonOutcome {
        let place = self.your_place();
        let table_size = self.rivals.len() + 1;

        let promoting = place <= PROMOTION_PLACES && self.league < TOP_LEAGUE;
        let relegating = place > table_size - RELEGATION_PLACES && self.league > 0;

        let (movement, to) = if promoting {
            (Movement::Promoted, self.league + 1)
        } else if relegating {
            (Movement::Relegated, self.league - 1)
        } else {
            (Movement::Held, self.league)
        };

        SeasonOutcome { place, movement, from: self.league, to }
    }
}

// --- persistence -----------------------------------------------------------

pub struct Save {
    pub best_score: u32,
    pub season: Season,
}

impl Save {
    pub fn load() -> Self {
        let Some(text) = storage::read(storage::SAVE) else {
            return Save { best_score: load_legacy_best(), season: Season::new(0) };
        };

        let mut best_score = 0;
        let mut league = 0usize;
        let mut round = 0u32;
        let mut your_total = 0u32;
        let mut your_last = 0u32;
        let mut rivals = Vec::new();

        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else { continue };
            match key {
                "best" => best_score = value.parse().unwrap_or(0),
                "league" => league = value.parse::<usize>().unwrap_or(0).min(TOP_LEAGUE),
                "round" => round = value.parse::<u32>().unwrap_or(0).min(SEASON_ROUNDS),
                "you" => your_total = value.parse().unwrap_or(0),
                "you_last" => your_last = value.parse().unwrap_or(0),
                "rival" => {
                    let parts: Vec<&str> = value.split(':').collect();
                    if let [name, skill, total, last] = parts[..] {
                        rivals.push(Rival {
                            name: name.to_string(),
                            skill: skill.parse().unwrap_or(0.1),
                            total: total.parse().unwrap_or(0),
                            last: last.parse().unwrap_or(0),
                        });
                    }
                }
                _ => {}
            }
        }

        // A save with the wrong number of rivals can't produce a valid table.
        if rivals.len() != RIVAL_COUNT {
            return Save { best_score, season: Season::new(league) };
        }

        Save {
            best_score,
            season: Season { league, round, your_total, your_last, rivals },
        }
    }

    /// Borrows rather than owns, so saving after every round costs no clone.
    pub fn store(best_score: u32, season: &Season) {
        let mut out = String::from("version=1\n");
        out.push_str(&format!("best={}\n", best_score));
        out.push_str(&format!("league={}\n", season.league));
        out.push_str(&format!("round={}\n", season.round));
        out.push_str(&format!("you={}\n", season.your_total));
        out.push_str(&format!("you_last={}\n", season.your_last));
        for rival in &season.rivals {
            out.push_str(&format!(
                "rival={}:{}:{}:{}\n",
                rival.name, rival.skill, rival.total, rival.last
            ));
        }
        storage::write(storage::SAVE, &out);
    }
}

fn load_legacy_best() -> u32 {
    storage::read(storage::LEGACY_BEST)
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn season_with(league: usize, your: u32, rival_totals: [u32; RIVAL_COUNT]) -> Season {
        let mut season = Season::new(league);
        season.round = SEASON_ROUNDS;
        season.your_total = your;
        for (rival, total) in season.rivals.iter_mut().zip(rival_totals) {
            rival.total = total;
        }
        season
    }

    #[test]
    fn winning_the_table_promotes() {
        let season = season_with(1, 50_000, [10, 20, 30, 40, 50]);
        let outcome = season.outcome();
        assert_eq!(outcome.place, 1);
        assert_eq!(outcome.movement, Movement::Promoted);
        assert_eq!(outcome.to, 2);
    }

    #[test]
    fn propping_up_the_table_relegates() {
        let season = season_with(3, 0, [10, 20, 30, 40, 50]);
        let outcome = season.outcome();
        assert_eq!(outcome.place, 6);
        assert_eq!(outcome.movement, Movement::Relegated);
        assert_eq!(outcome.to, 2);
    }

    #[test]
    fn mid_table_holds() {
        let season = season_with(2, 35, [10, 20, 30, 40, 50]);
        let outcome = season.outcome();
        assert_eq!(outcome.place, 3);
        assert_eq!(outcome.movement, Movement::Held);
        assert_eq!(outcome.to, 2);
    }

    #[test]
    fn bottom_of_bronze_cannot_drop_further() {
        let season = season_with(0, 0, [10, 20, 30, 40, 50]);
        assert_eq!(season.outcome().movement, Movement::Held);
        assert_eq!(season.outcome().to, 0);
    }

    #[test]
    fn top_of_hero_league_defends_instead_of_promoting() {
        let season = season_with(TOP_LEAGUE, 99_000, [10, 20, 30, 40, 50]);
        let outcome = season.outcome();
        assert_eq!(outcome.movement, Movement::Held);
        assert_eq!(outcome.to, TOP_LEAGUE);
        assert!(outcome.defended());
        assert_eq!(outcome.headline(), "TITLE DEFENDED");
    }

    #[test]
    fn a_tie_favours_the_player() {
        // Level on points with the rival in second: you take the higher place.
        let season = season_with(1, 30, [30, 20, 10, 5, 1]);
        assert_eq!(season.outcome().place, 1);
    }

    #[test]
    fn rivals_scale_with_the_board_on_offer() {
        let mut lean = Season::new(2);
        let mut rich = Season::new(2);
        // Same rivals, so only par differs.
        rich.rivals = lean.rivals.clone();

        lean.record_round(0, 28_000);
        rich.record_round(0, 75_000);

        let lean_total: u32 = lean.rivals.iter().map(|r| r.last).sum();
        let rich_total: u32 = rich.rivals.iter().map(|r| r.last).sum();
        assert!(rich_total > lean_total, "{rich_total} vs {lean_total}");
    }

    #[test]
    fn a_season_runs_exactly_five_rounds() {
        let mut season = Season::new(0);
        assert_eq!(season.round_number(), 1);
        for _ in 0..SEASON_ROUNDS {
            season.record_round(1_000, 30_000);
        }
        assert!(season.is_complete());
        assert_eq!(season.your_total, 5_000);

        // Further rounds cannot inflate a finished season.
        season.record_round(1_000, 30_000);
        assert_eq!(season.your_total, 5_000);
        assert_eq!(season.round, SEASON_ROUNDS);
    }

    #[test]
    fn standings_are_ordered_and_complete() {
        let season = season_with(2, 35, [10, 20, 30, 40, 50]);
        let table = season.standings();
        assert_eq!(table.len(), RIVAL_COUNT + 1);
        assert!(table.windows(2).all(|w| w[0].total >= w[1].total));
        assert!(table.iter().filter(|s| s.is_you).count() == 1);
        assert_eq!(table[0].place, 1);
    }
}
