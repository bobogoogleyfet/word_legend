//! Following the shared round.
//!
//! Everyone online plays the same board on the same clock. This keeps the local
//! game in step with the server: it reads the clock, starts the board when its
//! round comes up, ends it when the server's clock does, hands the words in and
//! fetches the table.
//!
//! When the server cannot be reached it steps aside and the game runs solo boards
//! on its own clock, then picks the shared cycle back up once the server answers
//! again. The round in progress is never interrupted in either direction.

use crate::game::{Game, Phase, RESULTS_SECONDS, ROUND_SECONDS};
use crate::identity::Identity;
use crate::net::{self, Claim, Client, Leaderboard, Link};

const CYCLE: f32 = ROUND_SECONDS + RESULTS_SECONDS;
/// How often to re-read the clock while all is well. The clock is projected
/// forward between reads, so this only has to catch drift and a sleeping tab.
const POLL_ONLINE: f64 = 30.0;
/// Retry cadence while the server is not answering.
const POLL_DOWN: f64 = 15.0;
/// Minimum gap between reads made because a new board is needed right now.
const POLL_URGENT: f64 = 2.0;
/// How long to wait for a first answer before giving up and playing solo.
const CONNECT_GRACE: f64 = 6.0;
/// Scorecard refresh, so late submissions show up while it is still open.
const LEADERBOARD_EVERY: f64 = 5.0;
/// First table read after handing in, giving the server a moment to file it.
const LEADERBOARD_FIRST: f64 = 1.5;
/// Joining a round with less than this left is not worth it; wait for the next.
pub const MIN_JOIN_SECONDS: f32 = 15.0;

/// Where the shared cycle stands, carried forward from the last reading.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Clock {
    pub round: u64,
    /// Seconds left in the current phase.
    pub left: f32,
    pub playing: bool,
}

/// Whether the server has our display name down as ours.
#[derive(Clone, Debug, PartialEq)]
pub enum NameStatus {
    Unclaimed,
    Pending,
    Claimed,
    /// The server said no -- almost always because someone else has the name.
    Refused(String),
    /// The server could not be asked. Playing on; it is asked again later.
    Unreachable,
}

pub struct Live {
    client: Client,
    link: Link,
    clock: Option<Clock>,
    /// The board from the latest reading, and the round it belongs to.
    board: Option<(u64, net::Board)>,
    /// The latest reading came back without a board: the server is up but has no
    /// rounds to serve, which for playing purposes is the same as being down.
    boardless: bool,
    leaderboard: Option<Leaderboard>,

    first_poll: Option<f64>,
    last_poll: Option<f64>,
    /// The player has asked to play. From then on rounds follow one another.
    joined: bool,

    pub name: NameStatus,
    /// Who the current claim is for, so a new name starts a new claim.
    claim_for: Option<Identity>,
    last_claim: Option<f64>,

    /// The last round whose words were handed in.
    submitted: Option<u64>,
    next_leaderboard: f64,
}

impl Live {
    pub fn new(client: Client) -> Self {
        let link = if client.is_configured() { Link::Connecting } else { Link::Solo };
        Live {
            client,
            link,
            clock: None,
            board: None,
            boardless: false,
            leaderboard: None,
            first_poll: None,
            last_poll: None,
            joined: false,
            name: NameStatus::Unclaimed,
            claim_for: None,
            last_claim: None,
            submitted: None,
            next_leaderboard: 0.0,
        }
    }

    /// Once a frame: read what the network has said, then act on it.
    pub fn update(&mut self, game: &mut Game, identity: Option<&Identity>, now: f64) {
        self.read_shared(now);
        self.poll_if_due(game, now);
        if let Some(me) = identity {
            self.keep_name(me, now);
        }
        self.drive(game, now);
        if let Some(me) = identity {
            self.hand_in(game, me, now);
        }
    }

    /// The player pressed play (or Enter). Joins the live round if one can be
    /// joined; otherwise skips a solo scorecard, or waits for the next round.
    pub fn play_now(&mut self, game: &mut Game, now: f64) {
        self.joined = true;
        match game.phase {
            Phase::Playing => {}
            // The shared scorecard runs on everyone's clock and cannot be skipped.
            Phase::Over if game.round.is_some() => {}
            Phase::Ready | Phase::Over => self.start_next(game, now),
        }
    }

    /// Ask for a name now, rather than waiting for the next frame's housekeeping.
    /// Used by the signup screen, which wants an answer before it lets you in.
    pub fn claim(&mut self, me: &Identity, now: f64) {
        self.claim_for = Some(me.clone());
        self.name = NameStatus::Pending;
        self.last_claim = Some(now);
        self.client.claim_name(&me.id, &me.name);
    }

    // --- what the UI reads ---------------------------------------------------

    pub fn link(&self) -> &Link {
        &self.link
    }

    pub fn clock(&self) -> Option<Clock> {
        self.clock
    }

    pub fn joined(&self) -> bool {
        self.joined
    }

    /// Following the server, as opposed to playing solo boards.
    pub fn is_live(&self, now: f64) -> bool {
        !self.server_away(now) && !self.boardless
    }

    pub fn leaderboard_for(&self, round: u64) -> Option<&Leaderboard> {
        self.leaderboard.as_ref().filter(|t| t.round == round)
    }

    /// Put a table in place as if the server had sent it.
    #[cfg(test)]
    pub fn show_leaderboard(&mut self, table: Leaderboard) {
        if let Ok(mut shared) = self.client.shared().lock() {
            shared.leaderboard = Some(table.clone());
        }
        self.leaderboard = Some(table);
    }

    // --- housekeeping --------------------------------------------------------

    fn read_shared(&mut self, now: f64) {
        let shared = self.client.shared();
        let Ok(shared) = shared.lock() else { return };

        if let Some(link) = &shared.link {
            self.link = link.clone();
        }
        if let Some(info) = &shared.round {
            let (round, left, playing) = net::project(info, shared.fetched_at, now, CYCLE);
            self.clock = Some(Clock { round, left, playing });
            self.boardless = info.board.is_none();
            if let Some(board) = &info.board {
                if self.board.as_ref().is_none_or(|(r, _)| *r != info.round) {
                    self.board = Some((info.round, board.clone()));
                }
            }
        }
        if shared.leaderboard.as_ref().map(|t| (t.round, &t.entries))
            != self.leaderboard.as_ref().map(|t| (t.round, &t.entries))
        {
            self.leaderboard = shared.leaderboard.clone();
        }
        if self.name == NameStatus::Pending {
            match &shared.claim {
                Some(Claim::Accepted(_)) => self.name = NameStatus::Claimed,
                Some(Claim::Refused(why)) => self.name = NameStatus::Refused(why.clone()),
                Some(Claim::Unreachable(_)) => self.name = NameStatus::Unreachable,
                None => {}
            }
        }
    }

    /// True when there is no server to follow: none configured, or it is not
    /// answering, or it has not answered at all within the grace period.
    fn server_away(&self, now: f64) -> bool {
        match self.link {
            Link::Solo | Link::Down(_) => true,
            Link::Online => false,
            Link::Connecting => self.first_poll.is_some_and(|t| now - t > CONNECT_GRACE),
        }
    }

    fn poll_if_due(&mut self, game: &Game, now: f64) {
        if !self.client.is_configured() {
            return;
        }
        // A board is wanted when a round is running that we have no board for, and
        // we are about to need it: waiting to join, or on a scorecard.
        let stale = match self.clock {
            None => true,
            Some(c) => c.playing && self.board.as_ref().is_none_or(|(r, _)| *r != c.round),
        };
        let waiting = game.phase != Phase::Playing && (self.joined || game.phase == Phase::Ready);
        let gap = match self.link {
            Link::Down(_) => POLL_DOWN,
            _ if stale && waiting => POLL_URGENT,
            Link::Online => POLL_ONLINE,
            _ => POLL_URGENT,
        };
        if self.last_poll.is_none_or(|t| now - t >= gap) {
            self.client.poll_round(now);
            self.last_poll = Some(now);
            self.first_poll.get_or_insert(now);
        }
    }

    fn keep_name(&mut self, me: &Identity, now: f64) {
        if self.claim_for.as_ref() != Some(me) {
            self.claim_for = Some(me.clone());
            self.name = NameStatus::Unclaimed;
        }
        let retry = match self.name {
            NameStatus::Unclaimed => true,
            NameStatus::Unreachable => {
                self.link == Link::Online && self.last_claim.is_none_or(|t| now - t >= POLL_DOWN)
            }
            _ => false,
        };
        // No point asking a server that is known to be away; the next good poll
        // flips the link and the claim goes out then.
        if retry && !matches!(self.link, Link::Down(_)) {
            self.claim(me, now);
        }
    }

    // --- the round cycle -----------------------------------------------------

    fn drive(&mut self, game: &mut Game, now: f64) {
        match game.phase {
            Phase::Ready => {
                if self.joined {
                    self.start_next(game, now);
                }
            }
            Phase::Playing => {
                // Solo rounds keep their own time.
                let (Some(round), Some(clock)) = (game.round, self.clock) else { return };
                if clock.round == round && clock.playing {
                    game.time_left = clock.left;
                } else {
                    game.end_round();
                    if clock.round == round {
                        game.results_left = clock.left;
                    }
                }
            }
            Phase::Over => match (game.round, self.clock) {
                (Some(round), Some(clock)) if clock.round == round && !clock.playing => {
                    game.results_left = clock.left;
                }
                (Some(_), _) => {
                    game.results_left = 0.0;
                    self.start_next(game, now);
                }
                (None, _) => {
                    if game.results_left <= 0.0 {
                        self.start_next(game, now);
                    }
                }
            },
        }
    }

    /// Put the next round in front of the player: the shared one when there is a
    /// server, a solo one when there is not, and nothing while a shared round is
    /// merely between boards.
    fn start_next(&mut self, game: &mut Game, now: f64) {
        if self.server_away(now) || self.boardless {
            game.start_round();
            return;
        }
        let Some(clock) = self.clock.filter(|c| c.playing) else { return };
        // Already played this one -- the local clock can run out a hair before the
        // server's does, and the round must not start over.
        if game.round == Some(clock.round) || clock.left < MIN_JOIN_SECONDS {
            return;
        }
        let Some((_, board)) = self.board.as_ref().filter(|(r, _)| *r == clock.round) else {
            return; // The read that brings this board is on its way.
        };
        if !game.start_shared_round(clock.round, board, clock.left) {
            // A board this client cannot read is a bug somewhere, but the player
            // should still get to play something.
            game.start_round();
        }
    }

    /// Hand the words in once the shared round is over, then keep the table fresh.
    fn hand_in(&mut self, game: &Game, me: &Identity, now: f64) {
        let Some(round) = game.round else { return };
        // The server only takes a round during its own scorecard window.
        let open = game.phase == Phase::Over
            && self.clock.is_some_and(|c| c.round == round && !c.playing);
        if !open {
            return;
        }

        if self.submitted != Some(round) && self.name == NameStatus::Claimed {
            self.client.submit(&me.id, round, &game.found_words());
            self.submitted = Some(round);
            self.next_leaderboard = now + LEADERBOARD_FIRST;
        }
        if self.submitted == Some(round) && now >= self.next_leaderboard {
            self.client.poll_leaderboard(round);
            self.next_leaderboard = now + LEADERBOARD_EVERY;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::RoundInfo;

    /// A real board from the deployed server.
    const GRID: [&str; 16] = ["c", "a", "m", "e", "e", "r", "s", "w", "a", "h", "m", "o", "l", "i", "e", "r"];

    fn me() -> Identity {
        Identity { id: "0123456789ABCDEF".into(), name: "wordfan".into() }
    }

    fn online() -> Live {
        Live::new(Client::recording("https://server"))
    }

    /// Pretend the server just answered a round read.
    fn serve(live: &Live, round: u64, phase: &str, left: f32, at: f64) {
        let shared = live.client.shared();
        let mut shared = shared.lock().unwrap();
        shared.link = Some(Link::Online);
        shared.fetched_at = at;
        shared.round = Some(RoundInfo {
            round,
            phase: phase.into(),
            seconds_left: left,
            server_time: 0.0,
            board: Some(net::Board {
                grid: GRID.iter().map(|s| s.to_string()).collect(),
                theme: Some("Colors".into()),
            }),
            error: None,
        });
    }

    fn accept_name(live: &Live) {
        let shared = live.client.shared();
        shared.lock().unwrap().claim = Some(Claim::Accepted("wordfan".into()));
    }

    fn sent_matching(live: &Live, prefix: &str) -> Vec<String> {
        live.client.sent().into_iter().filter(|r| r.starts_with(prefix)).collect()
    }

    #[test]
    fn joining_mid_round_plays_the_shared_board_on_the_shared_clock() {
        let mut game = Game::new();
        let mut live = online();
        serve(&live, 40, "play", 120.0, 0.0);

        live.update(&mut game, Some(&me()), 10.0);
        assert_eq!(game.phase, Phase::Ready, "nothing starts until the player asks");

        live.play_now(&mut game, 10.0);
        assert_eq!(game.phase, Phase::Playing);
        assert_eq!(game.round, Some(40));
        assert_eq!(game.grid[0][0].letters, "c");
        assert_eq!(game.theme.as_deref(), Some("Colors"));
        assert_eq!(game.time_left, 110.0, "a late joiner gets what is left, not a full round");
    }

    #[test]
    fn the_round_ends_on_the_servers_clock_and_is_handed_in_once() {
        let mut game = Game::new();
        let mut live = online();
        serve(&live, 40, "play", 100.0, 0.0);
        live.update(&mut game, Some(&me()), 0.0);
        accept_name(&live);
        live.play_now(&mut game, 0.0);

        // Find a word on the real board: C-A-M-E across the top row.
        for col in 0..4 {
            game.extend_path(crate::game::Position { row: 0, col });
        }
        game.submit_path();
        assert_eq!(game.found_words(), vec!["came".to_string()]);

        live.update(&mut game, Some(&me()), 50.0);
        assert_eq!(game.phase, Phase::Playing);
        assert_eq!(game.time_left, 50.0);

        // The server's clock runs out; the scorecard opens on everyone's timer.
        live.update(&mut game, Some(&me()), 101.0);
        assert_eq!(game.phase, Phase::Over);
        assert_eq!(game.results_left, RESULTS_SECONDS - 1.0);

        for t in [102.0, 103.0, 110.0] {
            live.update(&mut game, Some(&me()), t);
        }
        let scores = sent_matching(&live, "POST /score");
        assert_eq!(scores.len(), 1, "handed in {scores:?}");
        assert!(scores[0].contains("\"round\":40") && scores[0].contains("\"came\""), "{}", scores[0]);
        assert!(!sent_matching(&live, "GET /leaderboard?round=40").is_empty());
    }

    #[test]
    fn the_next_shared_round_follows_the_scorecard_without_a_click() {
        let mut game = Game::new();
        let mut live = online();
        serve(&live, 40, "play", 20.0, 0.0);
        live.update(&mut game, Some(&me()), 0.0);
        live.play_now(&mut game, 0.0);
        live.update(&mut game, Some(&me()), 30.0);
        assert_eq!(game.phase, Phase::Over);

        // Round 41 starts, but until its board arrives the scorecard just waits.
        let reads = sent_matching(&live, "GET /round").len();
        live.update(&mut game, Some(&me()), 81.0);
        assert_eq!(game.phase, Phase::Over);
        assert!(sent_matching(&live, "GET /round").len() > reads, "should be asking for the new board");

        serve(&live, 41, "play", 179.0, 81.5);
        live.update(&mut game, Some(&me()), 82.0);
        assert_eq!(game.phase, Phase::Playing);
        assert_eq!(game.round, Some(41));
    }

    #[test]
    fn a_round_is_never_restarted_when_the_local_clock_runs_out_first() {
        let mut game = Game::new();
        let mut live = online();
        serve(&live, 40, "play", 60.0, 0.0);
        live.update(&mut game, Some(&me()), 0.0);
        live.play_now(&mut game, 0.0);

        // Local tick ends the round a moment before the server's clock does.
        game.time_left = 0.0;
        game.tick(0.016);
        assert_eq!(game.phase, Phase::Over);
        live.update(&mut game, Some(&me()), 59.9);
        assert_eq!(game.phase, Phase::Over, "the finished round came back");
    }

    #[test]
    fn between_rounds_an_online_player_waits_for_the_board() {
        let mut game = Game::new();
        let mut live = online();
        serve(&live, 40, "results", 30.0, 0.0);
        live.update(&mut game, Some(&me()), 1.0);
        live.play_now(&mut game, 1.0);
        assert_eq!(game.phase, Phase::Ready, "no solo board while the server is fine");
        assert!(live.joined());
    }

    #[test]
    fn too_little_time_left_waits_for_the_next_round() {
        let mut game = Game::new();
        let mut live = online();
        serve(&live, 40, "play", MIN_JOIN_SECONDS - 1.0, 0.0);
        live.update(&mut game, Some(&me()), 0.0);
        live.play_now(&mut game, 0.0);
        assert_eq!(game.phase, Phase::Ready);
    }

    #[test]
    fn an_unreachable_server_falls_back_to_solo() {
        let mut game = Game::new();
        let mut live = online();
        live.update(&mut game, Some(&me()), 0.0);
        live.play_now(&mut game, 0.0);
        assert_eq!(game.phase, Phase::Ready, "gives the server a moment to answer");

        live.update(&mut game, Some(&me()), CONNECT_GRACE + 1.0);
        assert_eq!(game.phase, Phase::Playing, "no answer should mean solo play, not a hang");
        assert_eq!(game.round, None);
        assert!(!live.is_live(CONNECT_GRACE + 1.0));
    }

    #[test]
    fn a_solo_scorecard_rejoins_the_shared_cycle_when_the_server_returns() {
        let mut game = Game::new();
        let mut live = online();
        {
            let shared = live.client.shared();
            shared.lock().unwrap().link = Some(Link::Down("offline".into()));
        }
        live.update(&mut game, Some(&me()), 0.0);
        live.play_now(&mut game, 0.0);
        assert_eq!(game.round, None);
        game.time_left = 0.0;
        game.tick(0.016);
        assert_eq!(game.phase, Phase::Over);

        serve(&live, 40, "play", 150.0, 5.0);
        game.results_left = 0.0;
        live.update(&mut game, Some(&me()), 5.0);
        assert_eq!(game.round, Some(40), "should be back on the shared board");
        assert!(sent_matching(&live, "POST /score").is_empty(), "solo rounds are never handed in");
    }

    #[test]
    fn a_taken_name_is_reported_not_swallowed() {
        let mut game = Game::new();
        let mut live = online();
        serve(&live, 40, "play", 100.0, 0.0);
        live.update(&mut game, Some(&me()), 0.0);
        assert_eq!(live.name, NameStatus::Pending);
        assert_eq!(sent_matching(&live, "POST /claim").len(), 1);

        {
            let shared = live.client.shared();
            shared.lock().unwrap().claim = Some(Claim::Refused("That name is taken".into()));
        }
        live.update(&mut game, Some(&me()), 1.0);
        assert_eq!(live.name, NameStatus::Refused("That name is taken".into()));
        live.update(&mut game, Some(&me()), 2.0);
        assert_eq!(sent_matching(&live, "POST /claim").len(), 1, "a refusal is not retried");
    }

    #[test]
    fn an_unclaimed_player_is_not_handed_in() {
        let mut game = Game::new();
        let mut live = online();
        serve(&live, 40, "play", 20.0, 0.0);
        live.update(&mut game, Some(&me()), 0.0);
        live.play_now(&mut game, 0.0);
        live.update(&mut game, Some(&me()), 30.0);
        assert_eq!(game.phase, Phase::Over);
        assert!(sent_matching(&live, "POST /score").is_empty(), "the server would refuse it anyway");

        accept_name(&live);
        live.update(&mut game, Some(&me()), 31.0);
        assert_eq!(sent_matching(&live, "POST /score").len(), 1, "handed in once the name lands");
    }
}
