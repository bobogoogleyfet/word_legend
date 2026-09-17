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
use crate::net::{self, Claim, Client, Leaderboard, Link, Standings};

const CYCLE: f32 = ROUND_SECONDS + RESULTS_SECONDS;
/// How often to re-read the clock while all is well. The clock runs on from one
/// reading, so this only catches drift and a tab that slept; each request counts
/// against the server's daily allowance.
const POLL_ONLINE: f64 = 120.0;
/// Retry cadence while the server is not answering.
const POLL_DOWN: f64 = 30.0;
/// Minimum gap between reads made because a new board is needed right now.
const POLL_URGENT: f64 = 3.0;
/// How long to wait for a first answer before giving up and playing solo. Phones
/// on a slow connection can take several seconds; giving up too soon drops a
/// player into a solo board while everyone else is on the shared one.
const CONNECT_GRACE: f64 = 10.0;
/// When to read the table after handing in, in seconds: soon, while final scores
/// land, then twice more for stragglers, and no more -- the scorecard is only open
/// for thirty seconds, and every read is a request. Everyone who played is already
/// on the table from their progress, so the first read is nearly complete.
const LEADERBOARD_READS: [f64; 4] = [1.0, 4.0, 10.0, 20.0];
/// How often the all-time table is re-read while the stats page is open.
const STANDINGS_EVERY: f64 = 30.0;
/// How often progress is reported while a round is played, when there is news.
/// Joining reports straight away, which is what puts a player on the table; after
/// that, progress only keeps a player's score roughly current in case their final
/// one never arrives.
const PROGRESS_EVERY: f64 = 60.0;
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
    /// The all-time table, and when it was last asked for.
    standings: Option<Standings>,
    standings_asked: Option<f64>,

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
    /// When the words were handed in.
    submitted_at: f64,
    /// How many of `LEADERBOARD_READS` have been made for that round.
    leaderboard_reads: usize,
    /// The round progress is being reported for, when it was last sent, and how
    /// many words that report held.
    progress: Option<(u64, f64, usize)>,
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
            standings: None,
            standings_asked: None,
            first_poll: None,
            last_poll: None,
            joined: false,
            name: NameStatus::Unclaimed,
            claim_for: None,
            last_claim: None,
            submitted: None,
            submitted_at: 0.0,
            leaderboard_reads: 0,
            progress: None,
        }
    }

    /// Once a frame: read what the network has said, then act on it.
    pub fn update(&mut self, game: &mut Game, identity: Option<&Identity>, now: f64) {
        self.read_shared(now);
        self.poll_if_due(game, now);
        if let Some(me) = identity {
            self.keep_name(me, now);
            self.resume(game, me, now);
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

    /// Leave the round being played. The score is banked and, on a shared round,
    /// handed in as final straight away, so it is on the leaderboard now rather
    /// than when the round ends. The player is not put into the next round.
    pub fn leave(&mut self, game: &mut Game, me: Option<&Identity>, now: f64) {
        self.joined = false;
        if game.phase != Phase::Playing {
            return;
        }
        if let (Some(round), Some(me)) = (game.round, me) {
            if self.name == NameStatus::Claimed && self.submitted != Some(round) {
                self.client.submit(&me.id, round, &game.found_words(), &game.found_paths(), game.ranking.league);
                self.submitted = Some(round);
                self.submitted_at = now;
                self.leaderboard_reads = LEADERBOARD_READS.len();
            }
        }
        game.leave_round();
    }

    /// Put the scorecard away and go home, out of the cycle of rounds. Nothing is
    /// lost: the round was banked and handed in when it ended.
    pub fn close_results(&mut self, game: &mut Game) {
        self.joined = false;
        game.close_results();
    }

    /// Stop waiting for the next round.
    pub fn leave_queue(&mut self) {
        self.joined = false;
    }

    /// Asked to play, but waiting for the next shared round to start.
    pub fn queued(&self, game: &Game, now: f64) -> bool {
        self.joined
            && game.phase == Phase::Ready
            && self.is_live(now)
            && self.clock.is_some_and(|c| !c.playing || c.left < MIN_JOIN_SECONDS || game.round == Some(c.round))
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

    /// The all-time table, once it has arrived.
    pub fn standings(&self) -> Option<&Standings> {
        self.standings.as_ref()
    }

    /// Ask for the all-time table, at most every half minute: it only changes as
    /// rounds end, and the stats page is open for as long as a player reads it.
    pub fn want_standings(&mut self, now: f64) {
        if self.standings_asked.is_none_or(|t| now - t >= STANDINGS_EVERY) {
            self.standings_asked = Some(now);
            self.client.poll_standings();
        }
    }

    pub fn leaderboard_for(&self, round: u64) -> Option<&Leaderboard> {
        self.leaderboard.as_ref().filter(|t| t.round == round)
    }

    /// Put an all-time table in place as if the server had sent it.
    #[cfg(test)]
    pub fn show_standings(&mut self, table: Standings) {
        self.standings_asked = Some(f64::MAX);
        self.standings = Some(table);
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
        if shared.standings.is_some() {
            self.standings = shared.standings.clone();
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
        // Only a player waiting to play needs the new board at once; someone idling on
        // the home screen gets it with the next routine read.
        let waiting = game.phase != Phase::Playing && self.joined;
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

    /// Pick a shared round back up after the page was refreshed. Still running:
    /// back on its board, with its words, for the time it has left, and in the
    /// cycle of rounds again. Over already: banked as it stood, and handed in if
    /// the server still takes it.
    fn resume(&mut self, game: &mut Game, me: &Identity, now: f64) {
        let Some(saved) = game.pending_resume.as_ref() else { return };
        // A solo round needs no server: it carries on with the time it had.
        let Some(round) = saved.round else {
            if game.phase == Phase::Ready {
                game.resume_solo();
            } else {
                game.pending_resume = None;
            }
            return;
        };
        if game.phase != Phase::Ready {
            game.pending_resume = None; // something else is already on
            return;
        }
        let Some(clock) = self.clock else {
            if self.server_away(now) {
                game.finish_pending();
            }
            return;
        };
        if clock.round == round && clock.playing {
            let board = self.board.as_ref().filter(|(r, _)| *r == round).map(|(_, b)| b.clone());
            let Some(board) = board else { return }; // its board is on its way
            if game.resume_shared(round, &board, clock.left) {
                self.joined = true;
            } else {
                game.pending_resume = None;
            }
        } else if clock.round >= round {
            game.finish_pending();
            if clock.round == round && self.name == NameStatus::Claimed && self.submitted != Some(round) {
                self.client.submit(&me.id, round, &game.found_words(), &game.found_paths(), game.ranking.league);
                self.submitted = Some(round);
                self.submitted_at = now;
                self.leaderboard_reads = LEADERBOARD_READS.len();
            }
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

    /// Report progress while the shared round is played, hand the words in once
    /// it is over, then keep the table fresh.
    fn hand_in(&mut self, game: &Game, me: &Identity, now: f64) {
        let Some(round) = game.round else { return };

        // On the table from the moment of joining, then kept roughly current: a
        // report straight away, then one every few seconds when there are new words.
        if game.phase == Phase::Playing && self.name == NameStatus::Claimed {
            let found = game.found.len();
            let due = match self.progress {
                Some((r, sent_at, sent_words)) if r == round => {
                    found != sent_words && now - sent_at >= PROGRESS_EVERY
                }
                _ => true,
            };
            if due {
                self.client.progress(&me.id, round, &game.found_words(), &game.found_paths(), game.ranking.league);
                self.progress = Some((round, now, found));
            }
        }

        // The server only takes a round during its own scorecard window.
        let open = game.phase == Phase::Over
            && self.clock.is_some_and(|c| c.round == round && !c.playing);
        if !open {
            return;
        }

        if self.submitted != Some(round) && self.name == NameStatus::Claimed {
            self.client.submit(&me.id, round, &game.found_words(), &game.found_paths(), game.ranking.league);
            self.submitted = Some(round);
            self.submitted_at = now;
            self.leaderboard_reads = 0;
        }
        if self.submitted == Some(round)
            && LEADERBOARD_READS.get(self.leaderboard_reads).is_some_and(|at| now - self.submitted_at >= *at)
        {
            self.client.poll_leaderboard(round);
            self.leaderboard_reads += 1;
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
    fn a_player_is_on_the_table_from_joining_and_it_is_read_straight_after_the_round() {
        let mut game = Game::new();
        let mut live = online();
        serve(&live, 40, "play", 150.0, 0.0);
        live.update(&mut game, Some(&me()), 0.0);
        accept_name(&live);
        live.update(&mut game, Some(&me()), 0.1);
        live.play_now(&mut game, 0.1);

        // Joining reports at once, with nothing found yet.
        live.update(&mut game, Some(&me()), 0.2);
        let reports = sent_matching(&live, "POST /progress");
        assert_eq!(reports.len(), 1, "joining should report straight away: {reports:?}");
        assert!(reports[0].contains("\"round\":40") && reports[0].contains("\"words\":[]"));

        // New words are reported, but no more than once a minute.
        for col in 0..4 {
            game.extend_path(crate::game::Position { row: 0, col });
        }
        game.submit_path();
        live.update(&mut game, Some(&me()), 30.0);
        assert_eq!(sent_matching(&live, "POST /progress").len(), 1, "reported again within the minute");
        live.update(&mut game, Some(&me()), 61.0);
        assert_eq!(sent_matching(&live, "POST /progress").len(), 2, "a new word was not reported after a minute");
        live.update(&mut game, Some(&me()), 125.0);
        assert_eq!(sent_matching(&live, "POST /progress").len(), 2, "reported with nothing new");

        // The round ends: the final score goes in, and the table is read four times
        // over the scorecard, then not again.
        live.update(&mut game, Some(&me()), 150.5);
        assert_eq!(game.phase, Phase::Over);
        assert_eq!(sent_matching(&live, "POST /score").len(), 1);
        let reads = |live: &Live| sent_matching(live, "GET /leaderboard").len();
        let mut seen = Vec::new();
        for t in [151.0, 151.6, 154.6, 160.6, 170.6, 175.0, 179.0] {
            live.update(&mut game, Some(&me()), t);
            seen.push(reads(&live));
        }
        assert_eq!(seen, [0, 1, 2, 3, 4, 4, 4], "leaderboard reads over the scorecard");
        assert_eq!(sent_matching(&live, "POST /progress").len(), 2, "progress was sent after the round ended");
    }

    #[test]
    fn a_player_idling_on_the_home_screen_is_not_polled_urgently() {
        let mut game = Game::new();
        let mut live = online();
        serve(&live, 40, "play", 5.0, 0.0);
        live.update(&mut game, Some(&me()), 0.0);
        // Round 41 starts while the player sits on the home screen, not joined.
        let before = sent_matching(&live, "GET /round").len();
        for t in [40.0, 43.0, 46.0, 49.0, 60.0] {
            live.update(&mut game, Some(&me()), t);
        }
        assert_eq!(sent_matching(&live, "GET /round").len(), before, "an idle player's game kept asking for boards");
    }

    /// Someone who joins with seconds to spare is on the table from the moment
    /// they join, and sees everyone else's when the round ends.
    #[test]
    fn a_late_joiner_is_on_the_table_and_reads_it_like_everyone_else() {
        let mut game = Game::new();
        let mut live = online();
        // Twenty seconds left of a three-minute round: late, but joinable.
        serve(&live, 40, "play", 20.0, 0.0);
        live.update(&mut game, Some(&me()), 0.0);
        accept_name(&live);
        live.update(&mut game, Some(&me()), 0.1);
        live.play_now(&mut game, 0.1);
        assert_eq!(game.phase, Phase::Playing, "a joinable round was not joined");
        assert!(game.time_left <= 20.0);

        // Reported at once, so the others see them before the round ends.
        live.update(&mut game, Some(&me()), 0.2);
        let reports = sent_matching(&live, "POST /progress");
        assert_eq!(reports.len(), 1, "a late joiner was not put on the table: {reports:?}");
        assert!(reports[0].contains("\"round\":40"));

        // The round ends: their score is handed in, and they read the table.
        live.update(&mut game, Some(&me()), 21.0);
        assert_eq!(game.phase, Phase::Over);
        assert_eq!(sent_matching(&live, "POST /score").len(), 1);
        live.update(&mut game, Some(&me()), 22.5);
        assert_eq!(sent_matching(&live, "GET /leaderboard?round=40").len(), 1, "a late joiner does not read the table");
    }

    #[test]
    fn leaving_a_round_hands_the_score_in_now_and_stays_out_of_the_next() {
        let mut game = Game::new();
        let mut live = online();
        serve(&live, 40, "play", 100.0, 0.0);
        live.update(&mut game, Some(&me()), 0.0);
        accept_name(&live);
        live.update(&mut game, Some(&me()), 0.1);
        live.play_now(&mut game, 0.1);
        for col in 0..4 {
            game.extend_path(crate::game::Position { row: 0, col });
        }
        game.submit_path();

        live.leave(&mut game, Some(&me()), 30.0);
        assert_eq!(game.phase, Phase::Ready);
        let scores = sent_matching(&live, "POST /score");
        assert_eq!(scores.len(), 1, "leaving did not hand the score in");
        assert!(scores[0].contains("\"came\"") && scores[0].contains("\"paths\":[[0,1,2,3]]"), "{}", scores[0]);

        // The round ends and the next begins: a player who left is not pulled back in.
        live.update(&mut game, Some(&me()), 101.0);
        serve(&live, 41, "play", 179.0, 131.0);
        live.update(&mut game, Some(&me()), 132.0);
        assert_eq!(game.phase, Phase::Ready, "a player who left was put into the next round");
        assert_eq!(sent_matching(&live, "POST /score").len(), 1, "the round was handed in twice");
    }

    #[test]
    fn a_player_who_asks_to_play_between_rounds_is_queued() {
        let mut game = Game::new();
        let mut live = online();
        serve(&live, 40, "results", 20.0, 0.0);
        live.update(&mut game, Some(&me()), 0.0);
        assert!(!live.queued(&game, 0.0));
        live.play_now(&mut game, 0.0);
        assert!(live.queued(&game, 0.0), "waiting for the next round should read as queued");
        live.leave_queue();
        assert!(!live.queued(&game, 0.0));
    }

    #[test]
    fn a_refreshed_page_picks_the_shared_round_back_up() {
        let mut game = Game::new();
        let mut live = online();
        serve(&live, 40, "play", 90.0, 0.0);
        let grid: Vec<String> = GRID.iter().map(|s| s.to_string()).collect();
        let c = crate::game::Position { row: 0, col: 0 };
        let path: Vec<_> = (0..4).map(|col| crate::game::Position { row: 0, col }).collect();
        game.pending_resume = Some(crate::game::SavedRound {
            round: Some(40),
            grid: grid.clone(),
            theme: Some("Colors".into()),
            time_left: 120.0,
            found: vec![("came".into(), path)],
        });
        let _ = c;
        live.update(&mut game, Some(&me()), 5.0);
        assert_eq!(game.phase, Phase::Playing, "the saved round was not resumed");
        assert_eq!(game.round, Some(40));
        assert_eq!(game.time_left, 85.0, "resumed with the wrong time");
        assert_eq!(game.found_words(), vec!["came".to_string()]);
        assert!(live.joined(), "a resumed player should carry on into the next round");
    }

    #[test]
    fn a_refreshed_page_picks_a_solo_round_back_up_with_its_own_time() {
        let mut game = Game::new();
        let mut live = online();
        let grid: Vec<String> = GRID.iter().map(|s| s.to_string()).collect();
        let path: Vec<_> = (0..4).map(|col| crate::game::Position { row: 0, col }).collect();
        game.pending_resume = Some(crate::game::SavedRound { round: None, grid, theme: None, time_left: 42.0, found: vec![("came".into(), path)] });
        live.update(&mut game, Some(&me()), 0.0);
        assert_eq!((game.phase, game.round, game.time_left), (Phase::Playing, None, 42.0));
        assert_eq!(game.found_words(), vec!["came".to_string()]);
    }

    #[test]
    fn a_round_that_ended_while_the_page_was_closed_is_banked_and_handed_in() {
        let mut game = Game::new();
        let mut live = online();
        serve(&live, 40, "results", 20.0, 0.0);
        live.update(&mut game, Some(&me()), 0.0);
        accept_name(&live);
        let grid: Vec<String> = GRID.iter().map(|s| s.to_string()).collect();
        let path: Vec<_> = (0..4).map(|col| crate::game::Position { row: 0, col }).collect();
        game.pending_resume = Some(crate::game::SavedRound {
            round: Some(40),
            grid,
            theme: None,
            time_left: 30.0,
            found: vec![("came".into(), path)],
        });
        let games = game.ranking.lifetime.games;
        live.update(&mut game, Some(&me()), 1.0);
        assert_eq!(game.phase, Phase::Ready);
        assert!(game.pending_resume.is_none());
        assert_eq!(game.ranking.lifetime.games, games + 1, "the saved round was not banked");
        let scores = sent_matching(&live, "POST /score");
        assert_eq!(scores.len(), 1, "the saved round was not handed in");
        assert!(scores[0].contains("\"came\""));
    }

    #[test]
    fn the_all_time_table_is_asked_for_at_most_twice_a_minute() {
        let mut game = Game::new();
        let mut live = online();
        serve(&live, 40, "play", 100.0, 0.0);
        live.update(&mut game, Some(&me()), 0.0);
        assert!(live.standings().is_none());
        live.want_standings(1.0);
        live.want_standings(10.0);
        assert_eq!(sent_matching(&live, "GET /standings").len(), 1, "asked again within the half minute");
        live.want_standings(40.0);
        assert_eq!(sent_matching(&live, "GET /standings").len(), 2);
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
