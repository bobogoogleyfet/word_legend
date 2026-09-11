//! Talking to the server.
//!
//! Every call is fire-and-forget: a request is sent, and the reply lands in
//! shared state that the UI reads next frame. Nothing here ever blocks, because
//! the game has a three-minute clock running and a stalled frame would cost the
//! player real time.
//!
//! The network is treated as optional throughout. If the server cannot be
//! reached, the game keeps working on locally generated boards -- a dead
//! connection degrades the game to solo play rather than stopping it.

use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

/// Where the server lives. Empty means solo play with no network at all.
pub const DEFAULT_SERVER: &str = "https://word-legend.wordlegend.workers.dev";

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Board {
    /// Sixteen tiles, row by row. Usually one letter, "qu" for that die.
    pub grid: Vec<String>,
    pub theme: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct RoundInfo {
    pub round: u64,
    /// "play" or "results".
    pub phase: String,
    #[serde(rename = "secondsLeft")]
    pub seconds_left: f32,
    #[serde(rename = "serverTime")]
    pub server_time: f64,
    pub board: Option<Board>,
    #[serde(default)]
    pub error: Option<String>,
}

impl RoundInfo {
    pub fn is_playing(&self) -> bool {
        self.phase == "play"
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Entry {
    pub name: String,
    pub score: u32,
    pub words: u32,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Leaderboard {
    pub round: u64,
    pub entries: Vec<Entry>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Submitted {
    pub score: u32,
    pub accepted: u32,
    #[serde(default)]
    pub rejected: Vec<String>,
    pub average: u32,
}

#[derive(Serialize)]
struct ClaimRequest<'a> {
    id: &'a str,
    name: &'a str,
}

#[derive(Serialize)]
struct ScoreRequest<'a> {
    id: &'a str,
    round: u64,
    words: &'a [String],
}

/// How the game is currently getting its boards.
#[derive(Clone, Debug, PartialEq)]
pub enum Link {
    /// No server configured: purely local play.
    Solo,
    /// Trying to reach the server.
    Connecting,
    /// Live, and in step with everyone else.
    Online,
    /// Configured but unreachable; playing locally until it returns.
    Down(String),
}

impl Link {
    pub fn label(&self) -> String {
        match self {
            Link::Solo => "Solo".to_string(),
            Link::Connecting => "Connecting…".to_string(),
            Link::Online => "Live".to_string(),
            Link::Down(_) => "Offline — playing solo".to_string(),
        }
    }
}

/// Everything the network has told us, read by the UI each frame.
#[derive(Default)]
pub struct Shared {
    pub round: Option<RoundInfo>,
    pub leaderboard: Option<Leaderboard>,
    pub submitted: Option<Submitted>,
    /// `Ok(name)` once a name is taken, `Err(reason)` if the server refused it.
    pub claim: Option<Result<String, String>>,
    pub link: Option<Link>,
    /// Local clock reading when `round` arrived, to age it between polls.
    pub fetched_at: f64,
}

#[derive(Clone)]
pub struct Client {
    base: String,
    shared: Arc<Mutex<Shared>>,
}

impl Client {
    pub fn new(base: impl Into<String>) -> Self {
        Client { base: base.into(), shared: Arc::new(Mutex::new(Shared::default())) }
    }

    pub fn is_configured(&self) -> bool {
        !self.base.trim().is_empty()
    }

    pub fn shared(&self) -> Arc<Mutex<Shared>> {
        Arc::clone(&self.shared)
    }

    fn set_link(&self, link: Link) {
        if let Ok(mut shared) = self.shared.lock() {
            shared.link = Some(link);
        }
    }

    /// Ask where we are in the cycle, and for the current board.
    pub fn poll_round(&self, now: f64) {
        if !self.is_configured() {
            self.set_link(Link::Solo);
            return;
        }
        let shared = Arc::clone(&self.shared);
        let request = ehttp::Request::get(format!("{}/round", self.base));

        ehttp::fetch(request, move |result| {
            let Ok(mut guard) = shared.lock() else { return };
            match parse::<RoundInfo>(result) {
                Ok(info) => {
                    guard.link = Some(Link::Online);
                    guard.fetched_at = now;
                    guard.round = Some(info);
                }
                Err(why) => guard.link = Some(Link::Down(why)),
            }
        });
    }

    pub fn claim_name(&self, id: &str, name: &str) {
        if !self.is_configured() {
            // Nothing to claim against; the name is simply ours locally.
            if let Ok(mut guard) = self.shared.lock() {
                guard.claim = Some(Ok(name.to_string()));
            }
            return;
        }
        let shared = Arc::clone(&self.shared);
        let body = serde_json::to_vec(&ClaimRequest { id, name }).unwrap_or_default();
        let wanted = name.to_string();

        ehttp::fetch(post(format!("{}/claim", self.base), body), move |result| {
            let Ok(mut guard) = shared.lock() else { return };
            guard.claim = Some(match parse::<serde_json::Value>(result) {
                Ok(_) => Ok(wanted),
                Err(why) => Err(why),
            });
        });
    }

    /// Hand the round's words over to be scored. The score comes back from the
    /// server; whatever the client thinks it earned is not part of the request.
    pub fn submit(&self, id: &str, round: u64, words: &[String]) {
        if !self.is_configured() {
            return;
        }
        let shared = Arc::clone(&self.shared);
        let body = serde_json::to_vec(&ScoreRequest { id, round, words }).unwrap_or_default();

        ehttp::fetch(post(format!("{}/score", self.base), body), move |result| {
            let Ok(mut guard) = shared.lock() else { return };
            match parse::<Submitted>(result) {
                Ok(done) => guard.submitted = Some(done),
                Err(why) => guard.link = Some(Link::Down(why)),
            }
        });
    }

    pub fn poll_leaderboard(&self, round: u64) {
        if !self.is_configured() {
            return;
        }
        let shared = Arc::clone(&self.shared);
        let url = format!("{}/leaderboard?round={round}", self.base);

        ehttp::fetch(ehttp::Request::get(url), move |result| {
            let Ok(mut guard) = shared.lock() else { return };
            if let Ok(table) = parse::<Leaderboard>(result) {
                guard.leaderboard = Some(table);
            }
        });
    }
}

fn post(url: String, body: Vec<u8>) -> ehttp::Request {
    let mut request = ehttp::Request::post(url, body);
    request.headers.insert("Content-Type", "application/json");
    request
}

/// Turn a reply into a value, or into a reason the UI can show.
fn parse<T: for<'de> Deserialize<'de>>(
    result: Result<ehttp::Response, String>,
) -> Result<T, String> {
    let response = result.map_err(|e| e.to_string())?;
    let text = response.text().unwrap_or_default();

    if !response.ok {
        // The server explains itself in `error`; fall back to the status code.
        let reason = serde_json::from_str::<serde_json::Value>(text)
            .ok()
            .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_string))
            .unwrap_or_else(|| format!("server said {}", response.status));
        return Err(reason);
    }
    serde_json::from_str::<T>(text).map_err(|e| format!("unreadable reply: {e}"))
}

/// Which round should be running, given the last reading and how long ago it was.
/// Polls are occasional, so the clock has to be carried forward locally between
/// them rather than frozen at whatever the server last said.
pub fn project(info: &RoundInfo, fetched_at: f64, now: f64, cycle: f32) -> (u64, f32, bool) {
    let elapsed = (now - fetched_at).max(0.0) as f32;
    let mut left = info.seconds_left - elapsed;
    let mut round = info.round;
    let mut playing = info.is_playing();

    // Walk forward through any phase boundaries the gap crossed.
    let mut guard = 0;
    while left <= 0.0 && guard < 64 {
        if playing {
            left += cycle - crate::game::ROUND_SECONDS; // into the results window
            playing = false;
        } else {
            left += crate::game::ROUND_SECONDS; // next round begins
            playing = true;
            round += 1;
        }
        guard += 1;
    }
    (round, left.max(0.0), playing)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{RESULTS_SECONDS, ROUND_SECONDS};

    const CYCLE: f32 = ROUND_SECONDS + RESULTS_SECONDS;

    fn info(round: u64, phase: &str, left: f32) -> RoundInfo {
        RoundInfo {
            round,
            phase: phase.to_string(),
            seconds_left: left,
            server_time: 0.0,
            board: None,
            error: None,
        }
    }

    #[test]
    fn a_round_reply_is_understood() {
        let json = r#"{"round":7,"phase":"play","secondsLeft":42,"serverTime":1000,
            "board":{"grid":["a","b","c","d","e","f","g","h","i","j","k","l","m","n","o","qu"],
            "theme":"Animals"}}"#;
        let info: RoundInfo = serde_json::from_str(json).expect("should parse");
        assert_eq!(info.round, 7);
        assert!(info.is_playing());
        assert_eq!(info.seconds_left, 42.0);
        let board = info.board.expect("a board");
        assert_eq!(board.grid.len(), 16);
        assert_eq!(board.grid[15], "qu", "the Qu tile must survive the round trip");
        assert_eq!(board.theme.as_deref(), Some("Animals"));
    }

    #[test]
    fn a_round_with_no_board_is_not_an_error() {
        let json = r#"{"round":1,"phase":"results","secondsLeft":5,"serverTime":1,
            "board":null,"error":"no rounds uploaded"}"#;
        let info: RoundInfo = serde_json::from_str(json).expect("should parse");
        assert!(info.board.is_none());
        assert_eq!(info.error.as_deref(), Some("no rounds uploaded"));
    }

    #[test]
    fn the_clock_runs_on_between_polls() {
        // Ten seconds after a reading, ten fewer seconds remain.
        let (round, left, playing) = project(&info(5, "play", 100.0), 1_000.0, 1_010.0, CYCLE);
        assert_eq!(round, 5);
        assert_eq!(left, 90.0);
        assert!(playing);
    }

    #[test]
    fn the_clock_crosses_into_the_results_window() {
        let (round, left, playing) = project(&info(5, "play", 10.0), 1_000.0, 1_015.0, CYCLE);
        assert_eq!(round, 5, "results belong to the round just played");
        assert!(!playing);
        assert_eq!(left, RESULTS_SECONDS - 5.0);
    }

    #[test]
    fn the_clock_rolls_into_the_next_round() {
        // A reading with 5s of results left, read 10s later: the next round is up.
        let (round, left, playing) = project(&info(5, "results", 5.0), 1_000.0, 1_010.0, CYCLE);
        assert_eq!(round, 6);
        assert!(playing);
        assert_eq!(left, ROUND_SECONDS - 5.0);
    }

    #[test]
    fn a_long_gap_does_not_spin_forever() {
        // Tab asleep for a day: the projection must terminate, not hang.
        let (round, left, _) = project(&info(5, "play", 100.0), 0.0, 86_400.0, CYCLE);
        assert!(round > 5);
        assert!(left >= 0.0 && left <= CYCLE);
    }

    #[test]
    fn a_client_with_no_server_is_solo() {
        let client = Client::new("");
        assert!(!client.is_configured());
        client.poll_round(0.0);
        assert_eq!(client.shared().lock().unwrap().link, Some(Link::Solo));
    }

    #[test]
    fn claiming_offline_keeps_the_name_locally() {
        let client = Client::new("");
        client.claim_name("0123456789ABCDEF", "tanner");
        let shared = client.shared();
        let guard = shared.lock().unwrap();
        assert_eq!(guard.claim.as_ref().unwrap().as_deref(), Ok("tanner"));
    }

    #[test]
    fn a_leaderboard_reply_is_understood() {
        let json = r#"{"round":3,"entries":[{"name":"tanner","score":5400,"words":12},
            {"name":"jax","score":3100,"words":9}]}"#;
        let table: Leaderboard = serde_json::from_str(json).expect("should parse");
        assert_eq!(table.entries.len(), 2);
        assert_eq!(table.entries[0].name, "tanner");
        assert_eq!(table.entries[0].score, 5400);
    }
}
