//! egui front end: the board, the clock, and the round-end scorecard.

use crate::clipboard::Paste;
use crate::game::{Feedback, Game, Phase, Position, RESULTS_SECONDS, ROUND_SECONDS, SIZE};
use crate::identity::{self, Identity};
use crate::league::{GameRecord, Movement, FORM_GAMES, HISTORY_GAMES, LEAGUES, MIN_GAMES_TO_MOVE};
use crate::live::{Live, NameStatus, MIN_JOIN_SECONDS};
use crate::net::{self, Link};
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, Vec2};

const BG: Color32 = Color32::from_rgb(0x12, 0x14, 0x1c);
const PANEL: Color32 = Color32::from_rgb(0x1b, 0x1e, 0x2b);
const TILE: Color32 = Color32::from_rgb(0x2a, 0x2f, 0x42);
const TILE_EDGE: Color32 = Color32::from_rgb(0x3a, 0x41, 0x5c);
const TEXT: Color32 = Color32::from_rgb(0xe8, 0xec, 0xf5);
const MUTED: Color32 = Color32::from_rgb(0x8b, 0x93, 0xab);
const ACCENT: Color32 = Color32::from_rgb(0x5b, 0x8c, 0xff);
const GREEN: Color32 = Color32::from_rgb(0x3d, 0xdc, 0x84);
const RED: Color32 = Color32::from_rgb(0xff, 0x5c, 0x5c);
const AMBER: Color32 = Color32::from_rgb(0xff, 0xb4, 0x54);
const GOLD: Color32 = Color32::from_rgb(0xff, 0xd1, 0x66);
const BLUE: Color32 = Color32::from_rgb(0x5b, 0x8c, 0xff);

/// The history chart's two series: each game's score, and the rank average after
/// it. Blue and yellow from the reference palette's dark steps, validated against
/// PANEL: both clear 3:1, and they stay far apart under every colour-vision type.
const SERIES_SCORES: Color32 = Color32::from_rgb(0x39, 0x87, 0xe5);
const SERIES_AVERAGE: Color32 = Color32::from_rgb(0xc9, 0x85, 0x00);

/// The logo's tiles: black, with gold letters and a gold edge.
const LOGO_TILE: Color32 = Color32::from_rgb(0x0a, 0x0a, 0x0c);

/// The phone scorecard's tabs: the leaderboard, which opens first on a shared
/// round, or the word lists side by side.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ResultsTab {
    Leaderboard,
    Words,
}

/// A letter tapped on the scorecard's board, and the words that run through it.
struct LetterFocus {
    at: Position,
    words: Vec<(String, Vec<Position>)>,
}

/// First-run screen state, kept out of the app struct's main body.
struct Signup {
    pending: Option<Identity>,
    name: String,
    code: String,
    restoring: bool,
    /// When the code was last copied, so the copy icon can show a tick for a moment.
    copied_at: Option<f64>,
    /// The account whose name the server is being asked about, and since when.
    checking: Option<(Identity, f64)>,
    /// Why the last name was not accepted, shown under the name field.
    notice: Option<String>,
    /// Where the Paste button's clipboard read lands.
    paste: Paste,
    /// Why the clipboard could not be read, if it could not.
    paste_failed: Option<String>,
}

impl Signup {
    fn new(_fresh: bool) -> Self {
        Signup {
            pending: None,
            name: String::new(),
            code: String::new(),
            restoring: false,
            copied_at: None,
            checking: None,
            notice: None,
            paste: Paste::default(),
            paste_failed: None,
        }
    }
}

/// Below this window height the results card cannot fit and has to scroll.
const SHORT_WINDOW: f32 = 720.0;

/// Below this width -- a phone held upright -- layouts stack instead of sitting
/// side by side, and the found-words panel gives its room to the board.
const NARROW: f32 = 600.0;

fn is_narrow(ctx: &egui::Context) -> bool {
    ctx.screen_rect().width() < NARROW
}

/// Seconds left at which the clock starts pulsing red.
const PANIC_TIME: f32 = 15.0;

/// How long the copy icon shows its tick after copying.
const COPIED_TICK: f64 = 2.0;

/// The recovery code field and its copy icon, by fixed id so tests can find them.
const RECOVERY_CODE_ID: &str = "recovery_code";
const COPY_CODE_ID: &str = "copy_recovery_code";

/// How long signup waits on the server's answer about a name before letting the
/// player in anyway. The name is claimed again once the server is back.
const NAME_CHECK_TIMEOUT: f64 = 8.0;

pub struct WordLegendApp {
    game: Game,
    /// Keeps the game on the shared round, when there is a server to follow.
    live: Live,
    /// egui's clock, read once a frame; what `live` measures its timings against.
    now: f64,
    /// The account. Absent until the first-run screen has been through.
    identity: Option<Identity>,
    /// First-run screen state.
    signup: Signup,
    /// Which tab the scorecard is showing.
    results_tab: ResultsTab,
    /// The stats page is open.
    show_stats: bool,
    /// Asking whether to leave the round being played.
    confirm_leave: bool,
    /// A letter tapped on the scorecard's board, if any.
    letter_focus: Option<LetterFocus>,
    /// A word tapped in any of the scorecard's lists, and its path, drawn on the
    /// board.
    shown_word: Option<(String, Vec<Position>)>,
    /// Where the cursor was last frame, so a drag can be traced as a segment
    /// rather than sampled as isolated points.
    drag_from: Option<Pos2>,
    /// Drives the tile "pop" when a word lands, independent of game state.
    elapsed: f32,
    /// When the recovery code was last copied from the start screen, to confirm it.
    code_copied_at: Option<f64>,
    /// The home screen's PLAY tiles, traced like a board.
    play_swipe: PlaySwipe,
}

/// A swipe across the home screen's PLAY tiles. Starting a game the way a word is
/// played teaches the gesture before the first round.
#[derive(Default)]
struct PlaySwipe {
    /// Tiles traced so far, by index into P-L-A-Y.
    trail: Vec<usize>,
    from: Option<Pos2>,
    /// The player tapped, or swiped something other than PLAY: say how it works.
    show_hint: bool,
}

impl WordLegendApp {
    pub fn new() -> Self {
        let identity = Identity::load();
        Self {
            game: Game::new(),
            live: Live::new(net::Client::new(net::DEFAULT_SERVER)),
            now: 0.0,
            signup: Signup::new(identity.is_none()),
            identity,
            results_tab: ResultsTab::Leaderboard,
            show_stats: false,
            confirm_leave: false,
            letter_focus: None,
            shown_word: None,
            drag_from: None,
            elapsed: 0.0,
            code_copied_at: None,
            play_swipe: PlaySwipe::default(),
        }
    }
}

impl eframe::App for WordLegendApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.frame(ctx);
    }
}

impl WordLegendApp {
    /// One whole frame. Separate from `update` so tests can run it headlessly.
    fn frame(&mut self, ctx: &egui::Context) {
        // egui's own frame delta, not `Instant`: `Instant::now()` panics outright on
        // wasm32-unknown-unknown, which would take the whole web build down.
        // Clamped so a stalled tab (backgrounded, or a slow first frame) cannot
        // swallow a chunk of the round clock in one step.
        let dt = ctx.input(|i| i.stable_dt).min(0.1);
        self.elapsed += dt;
        self.now = ctx.input(|i| i.time);
        self.game.tick(dt);

        // Nobody plays before they have a name, but the clock is read regardless so
        // the start screen can say what round is on.
        let me = self.identity.clone().filter(|_| !self.needs_signup());
        self.live.update(&mut self.game, me.as_ref(), self.now);
        self.follow_name_check();
        // Network replies land outside egui, so nothing else would wake a quiet frame.
        ctx.request_repaint_after(std::time::Duration::from_millis(250));

        style(ctx);
        self.handle_keys(ctx);

        let narrow = is_narrow(ctx);
        let hud_margin = if narrow { egui::Margin::symmetric(12, 8) } else { egui::Margin::symmetric(20, 14) };
        egui::TopBottomPanel::top("hud")
            .resizable(false)
            .frame(egui::Frame::default().fill(PANEL).inner_margin(hud_margin))
            // The one-line bar needs room for LEAVE as well; below 900px the two-row
            // phone bar fits better.
            .show(ctx, |ui| if ctx.screen_rect().width() < 900.0 { self.hud_narrow(ui) } else { self.hud(ui) });

        // On a narrow window (a phone in the web build) the side panel would squeeze
        // the board into nothing, so drop it and let the board have the room.
        let wide = ctx.screen_rect().width() >= 760.0;
        if wide {
            egui::SidePanel::right("found")
                .exact_width(230.0)
                .resizable(false)
                .frame(egui::Frame::default().fill(PANEL).inner_margin(egui::Margin::same(16)))
                .show(ctx, |ui| self.found_panel(ui));
        }

        let board_margin = if narrow { egui::Margin::same(4) } else { egui::Margin::same(16) };
        egui::CentralPanel::default()
            .frame(egui::Frame::default().fill(BG).inner_margin(board_margin))
            .show(ctx, |ui| self.board_area(ui));

        if self.needs_signup() {
            self.overlay_signup(ctx);
            ctx.request_repaint();
            return;
        }

        // A round starting takes over the screen, and the next scorecard opens on
        // its leaderboard again.
        if self.game.phase == Phase::Playing {
            self.show_stats = false;
            self.results_tab = ResultsTab::Leaderboard;
            self.letter_focus = None;
            self.shown_word = None;
        }

        if self.game.phase != Phase::Playing {
            self.confirm_leave = false;
        }

        match self.game.phase {
            Phase::Playing if self.confirm_leave => self.overlay_confirm_leave(ctx),
            _ if self.show_stats => self.overlay_stats(ctx),
            Phase::Ready => self.overlay_ready(ctx),
            Phase::Over => self.overlay_results(ctx),
            Phase::Playing => {}
        }

        // The clock and the word animations both need a live frame rate.
        if matches!(self.game.phase, Phase::Playing | Phase::Over) || self.game.feedback_timer > 0.0 {
            ctx.request_repaint();
        }
    }
}

impl WordLegendApp {
    fn needs_signup(&self) -> bool {
        self.identity.as_ref().map(|i| i.name.is_empty()).unwrap_or(true)
    }

    /// Ask the server for a name; `follow_name_check` lets the player in once it
    /// answers.
    fn check_name(&mut self, me: Identity) {
        self.signup.notice = None;
        self.live.claim(&me, self.now);
        self.signup.checking = Some((me, self.now));
    }

    fn follow_name_check(&mut self) {
        if let Some((me, since)) = self.signup.checking.clone() {
            let timed_out = self.now - since > NAME_CHECK_TIMEOUT;
            match &self.live.name {
                NameStatus::Refused(why) => {
                    self.signup.notice = Some(why.clone());
                    self.signup.checking = None;
                }
                // No answer is not a no: play on, and the claim is retried later.
                NameStatus::Claimed | NameStatus::Unreachable => self.finish_signup(me),
                _ if timed_out => self.finish_signup(me),
                _ => {}
            }
            return;
        }

        // A saved account whose name someone else now holds: keep the account,
        // ask for a different name.
        if let (Some(me), NameStatus::Refused(why)) = (&self.identity, &self.live.name) {
            self.signup.restoring = true;
            self.signup.code = me.recovery_code();
            self.signup.name.clear();
            self.signup.notice = Some(format!("{why}. Pick another name."));
            self.identity = None;
        }
    }

    fn finish_signup(&mut self, me: Identity) {
        me.store();
        self.identity = Some(me);
        self.signup.checking = None;
        self.signup.notice = None;
    }

    /// The name field's error line: a local problem with the name, or the
    /// server's answer about it.
    fn name_problem(&self, ui: &mut egui::Ui, checked: &Result<String, identity::NameError>) {
        if let Err(problem) = checked {
            if !self.signup.name.trim().is_empty() {
                ui.label(egui::RichText::new(problem.message()).size(11.0).color(RED));
            }
        } else if let Some(notice) = &self.signup.notice {
            ui.label(egui::RichText::new(notice).size(11.0).color(RED));
        }
    }

    /// First run: hand over the recovery code, take a display name.
    fn overlay_signup(&mut self, ctx: &egui::Context) {
        self.page(ctx, true, |app, ui| {
            ui.vertical_centered(|ui| {
                let pending = app.signup.pending.get_or_insert_with(Identity::generate).clone();

                ui.label(egui::RichText::new("WORD LEGEND").size(38.0).color(TEXT).strong());
                ui.label(
                    egui::RichText::new("No email, no password. Just a code and a name.")
                        .size(13.0)
                        .color(MUTED),
                );
                ui.add_space(20.0);

                if app.signup.restoring {
                    app.signup_restore(ui);
                } else {
                    app.signup_new(ui, &pending);
                }
            });
        });
    }

    fn signup_new(&mut self, ui: &mut egui::Ui, pending: &Identity) {
        ui.label(egui::RichText::new("YOUR RECOVERY CODE").size(11.0).color(MUTED).strong());
        ui.add_space(6.0);

        // The code is the account. Make it impossible to miss. It is a read-only
        // text field rather than painted text, so it can be selected and copied
        // with Ctrl+C like any other text, and the icon beside it copies it whole.
        let code = pending.recovery_code();
        let box_width = ui.available_width().min(420.0);
        // Smaller type in a phone-width box, so the code and its icon still fit.
        let code_size = if box_width < 400.0 { 20.0 } else { 26.0 };
        let (rect, _) = ui.allocate_exact_size(Vec2::new(box_width, 56.0), Sense::hover());
        let painter = ui.painter();
        painter.rect_filled(rect, 10.0, TILE);
        painter.rect_stroke(rect, 10.0, Stroke::new(2.0_f32, GOLD), egui::StrokeKind::Inside);

        let icon = Rect::from_center_size(
            Pos2::new(rect.max.x - 30.0, rect.center().y),
            Vec2::splat(40.0),
        );
        let text = Rect::from_min_max(rect.min + Vec2::new(12.0, 0.0), Pos2::new(icon.min.x - 4.0, rect.max.y));
        let mut shown = code.as_str();
        ui.put(
            text,
            egui::TextEdit::singleline(&mut shown)
                .id(egui::Id::new(RECOVERY_CODE_ID))
                .font(FontId::monospace(code_size))
                .text_color(GOLD)
                .horizontal_align(egui::Align::Center)
                .vertical_align(egui::Align::Center)
                .frame(false),
        );

        let copied = self.signup.copied_at.is_some_and(|t| self.now - t < COPIED_TICK);
        if copy_icon(ui, icon, copied).clicked() {
            ui.ctx().copy_text(code.clone());
            self.signup.copied_at = Some(self.now);
        }

        ui.add_space(10.0);
        ui.label(
            egui::RichText::new("Write this down now.")
                .size(14.0)
                .color(AMBER)
                .strong(),
        );
        for line in [
            "It is the only way back into this account, on this device or any other.",
            "Nobody can recover it for you, because nothing else about you is stored.",
        ] {
            ui.label(egui::RichText::new(line).size(12.0).color(MUTED));
        }

        ui.add_space(18.0);
        ui.label(egui::RichText::new("DISPLAY NAME").size(11.0).color(MUTED).strong());
        ui.label(
            egui::RichText::new("Shown on the leaderboard. Must be unique.")
                .size(11.0)
                .color(MUTED),
        );
        ui.add_space(4.0);
        ui.add(
            egui::TextEdit::singleline(&mut self.signup.name)
                .desired_width(ui.available_width().min(280.0))
                .hint_text(hint("pick a name")),
        );

        let checked = identity::check_name(&self.signup.name);
        self.name_problem(ui, &checked);

        ui.add_space(14.0);
        if self.signup.checking.is_some() {
            disabled_big_button(ui, "CHECKING NAME…");
        } else if let Ok(name) = checked {
            if big_button(ui, "START PLAYING", ACCENT) {
                self.check_name(Identity { id: pending.id.clone(), name });
            }
        } else {
            disabled_big_button(ui, "START PLAYING");
        }

        ui.add_space(10.0);
        if ui
            .link(egui::RichText::new("I already have a code").size(12.0).color(ACCENT))
            .clicked()
        {
            self.signup.restoring = true;
        }
    }

    fn signup_restore(&mut self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new("ENTER YOUR CODE").size(11.0).color(MUTED).strong());
        ui.add_space(6.0);
        match self.signup.paste.take() {
            Some(Ok(text)) => {
                // Tidy a recognisable code into its usual form; anything else goes
                // in as it came, so the player can see what was on the clipboard.
                self.signup.code = Identity::from_recovery(&text)
                    .map(|found| found.recovery_code())
                    .unwrap_or_else(|| text.trim().to_string());
                self.signup.paste_failed = None;
            }
            Some(Err(why)) => self.signup.paste_failed = Some(why),
            None => {}
        }

        // The field and its button side by side, centred as one row.
        let field_width = (ui.available_width() - 8.0 - 100.0).min(320.0);
        ui.allocate_ui_with_layout(
            Vec2::new(field_width + 8.0 + 100.0, 44.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.signup.code)
                        .desired_width(field_width)
                        .font(egui::TextStyle::Monospace)
                        .hint_text(hint("XXXX-XXXX-XXXX-XXXX")),
                );
                if action_button(ui, "Paste", ACCENT).clicked() {
                    self.signup.paste.request();
                }
            },
        );

        let parsed = Identity::from_recovery(&self.signup.code);
        if let Some(why) = &self.signup.paste_failed {
            ui.label(
                egui::RichText::new(format!(
                    "Couldn't read the clipboard ({why}). Click the box and press Ctrl+V or Cmd+V."
                ))
                .size(11.0)
                .color(AMBER),
            );
        } else if parsed.is_none() && !self.signup.code.trim().is_empty() {
            ui.label(
                egui::RichText::new("That is not a valid code").size(11.0).color(RED),
            );
        }

        ui.add_space(12.0);
        ui.label(egui::RichText::new("DISPLAY NAME").size(11.0).color(MUTED).strong());
        ui.add(
            egui::TextEdit::singleline(&mut self.signup.name)
                .desired_width(ui.available_width().min(280.0))
                .hint_text(hint("pick a name")),
        );
        let checked = identity::check_name(&self.signup.name);
        self.name_problem(ui, &checked);

        ui.add_space(14.0);
        match (parsed, checked) {
            _ if self.signup.checking.is_some() => {
                disabled_big_button(ui, "CHECKING NAME…");
            }
            (Some(found), Ok(name)) => {
                if big_button(ui, "RESTORE ACCOUNT", ACCENT) {
                    self.check_name(Identity { id: found.id, name });
                }
            }
            _ => {
                disabled_big_button(ui, "RESTORE ACCOUNT");
            }
        }

        ui.add_space(10.0);
        if ui
            .link(egui::RichText::new("Start a new account instead").size(12.0).color(ACCENT))
            .clicked()
        {
            self.signup.restoring = false;
        }
    }

    fn handle_keys(&mut self, ctx: &egui::Context) {
        // Enter in the name field is typing, not a request to start a round.
        if self.needs_signup() {
            return;
        }
        let (enter, escape, space) = ctx.input(|i| {
            (
                i.key_pressed(egui::Key::Enter),
                i.key_pressed(egui::Key::Escape),
                i.key_pressed(egui::Key::Space),
            )
        });

        match self.game.phase {
            Phase::Playing => {
                if enter {
                    self.game.submit_path();
                } else if escape {
                    self.game.clear_path();
                }
            }
            Phase::Ready | Phase::Over => {
                if enter || space {
                    self.live.play_now(&mut self.game, self.now);
                }
            }
        }
    }

    // --- heads-up display --------------------------------------------------

    fn hud(&mut self, ui: &mut egui::Ui) {
        // A heads-up display is one line of facts; wrapped, it grows downward.
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
        ui.horizontal(|ui| {
            // The one way out of a round, first in the bar at the top left.
            if self.game.phase == Phase::Playing {
                ui.vertical(|ui| {
                    ui.add_space(8.0);
                    if action_button(ui, "LEAVE", RED).clicked() {
                        self.confirm_leave = true;
                    }
                });
                ui.add_space(20.0);
            }
            ui.vertical(|ui| {
                ui.label(egui::RichText::new("WORD LEGEND").size(13.0).color(ACCENT).strong());
                ui.label(
                    egui::RichText::new(format!("{}", self.game.score))
                        .size(34.0)
                        .color(TEXT)
                        .strong(),
                );
            });

            ui.add_space(24.0);

            let league = self.game.ranking.league;
            ui.vertical(|ui| {
                ui.add_space(6.0);
                rank_label(ui, league, 16.0);
                let (label, color) = self.link_label();
                ui.label(egui::RichText::new(label).size(11.0).color(color).strong());
            });

            ui.add_space(24.0);

            ui.vertical(|ui| {
                let name = self.identity.as_ref().map(|i| i.name.as_str()).unwrap_or("");
                ui.add_space(6.0);
                ui.label(egui::RichText::new(name).size(15.0).color(TEXT).strong());
            });

            ui.add_space(24.0);

            ui.vertical(|ui| {
                ui.add_space(4.0);
                self.timer_bar(ui);
            });
        });
    }

    /// The heads-up display for a phone: score and standing on one row, the clock
    /// across the full width beneath. Side by side, as on a desktop, it ran off
    /// the edge of the screen and took the clock with it.
    fn hud_narrow(&mut self, ui: &mut egui::Ui) {
        // Three equal columns. Laid out right-to-left instead, the name took the
        // width and left the league a sliver to wrap into, one letter a line.
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
        let league = self.game.ranking.league;
        let playing = self.game.phase == Phase::Playing;
        let mut leave = false;
        let score = thousands(self.game.score as usize);
        let found = format!("{} found", self.game.found.len());
        let name = self.identity.as_ref().map(|i| i.name.clone()).unwrap_or_default();
        let (link, link_color) = self.link_label();
        ui.columns(3, |cols| {
            // During a round, LEAVE takes the top left and the name gives way; the
            // name is on the home screen.
            let (score_col, rank_col) = if playing { (1, 2) } else { (0, 1) };
            if playing {
                cols[0].add_space(2.0);
                leave = action_button(&mut cols[0], "LEAVE", RED).clicked();
            } else {
                cols[2].label(egui::RichText::new(&name).size(13.0).color(TEXT).strong());
            }

            cols[score_col].label(egui::RichText::new(&score).size(26.0).color(TEXT).strong());
            // The found-words panel does not fit on a phone; the count does.
            cols[score_col].label(egui::RichText::new(&found).size(11.0).color(MUTED));

            cols[rank_col].add_space(4.0);
            rank_label(&mut cols[rank_col], league, 14.0);
            cols[rank_col].label(egui::RichText::new(&link).size(11.0).color(link_color).strong());
        });
        if leave {
            self.confirm_leave = true;
        }
        ui.add_space(4.0);
        self.timer_bar(ui);
    }

    /// Whether this board is the one everyone is on, in a word.
    fn link_label(&self) -> (String, Color32) {
        match self.live.link() {
            Link::Online if self.live.is_live(self.now) => ("LIVE".to_string(), GREEN),
            Link::Online => ("SOLO · NO ROUNDS".to_string(), AMBER),
            Link::Connecting if self.live.is_live(self.now) => ("CONNECTING…".to_string(), MUTED),
            Link::Solo => ("SOLO".to_string(), MUTED),
            _ => ("OFFLINE · SOLO".to_string(), AMBER),
        }
    }

    /// Seconds until the next shared round starts, if there is one to wait for.
    fn next_shared_round_in(&self) -> Option<f32> {
        if !self.live.is_live(self.now) {
            return None;
        }
        let clock = self.live.clock()?;
        Some(if clock.playing { clock.left + RESULTS_SECONDS } else { clock.left })
    }

    fn timer_bar(&self, ui: &mut egui::Ui) {
        let left = self.game.time_left;
        let panicking = self.game.phase == Phase::Playing && left <= PANIC_TIME;
        let color = if panicking {
            // Pulse between red and amber so the last seconds are unmissable.
            let t = (self.elapsed * 6.0).sin() * 0.5 + 0.5;
            lerp_color(RED, AMBER, t)
        } else {
            ACCENT
        };

        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(format!("{}:{:02}", left as u32 / 60, left as u32 % 60))
                    .size(26.0)
                    .color(color)
                    .strong(),
            );
            ui.add_space(10.0);

            // Room is left for the BEST column; never wider than what is there.
            let width = (ui.available_width() - 80.0).max(40.0);
            let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 12.0), Sense::hover());
            let painter = ui.painter();
            painter.rect_filled(rect, 6.0, TILE);

            let fraction = (left / ROUND_SECONDS).clamp(0.0, 1.0);
            if fraction > 0.0 {
                let filled = Rect::from_min_size(
                    rect.min,
                    Vec2::new(rect.width() * fraction, rect.height()),
                );
                painter.rect_filled(filled, 6.0, color);
            }

            ui.add_space(10.0);
            ui.vertical(|ui| {
                ui.label(egui::RichText::new("BEST").size(10.0).color(MUTED));
                ui.label(
                    egui::RichText::new(format!("{}", self.game.best_score()))
                        .size(15.0)
                        .color(GOLD)
                        .strong(),
                );
            });
        });
    }

    // --- board -------------------------------------------------------------

    fn board_area(&mut self, ui: &mut egui::Ui) {
        const PILL_H: f32 = 44.0;
        /// The theme, the letter bonus and its key, and the hint.
        const BELOW_H: f32 = 22.0 + 20.0 + 18.0 + 20.0;
        const GAP: f32 = 12.0;

        let avail = ui.available_size();
        let board_size = (avail.y - PILL_H - BELOW_H - GAP * 2.0)
            .min(avail.x)
            .min(540.0)
            .max(240.0);
        let slack = avail.y - (PILL_H + BELOW_H + GAP * 2.0 + board_size);

        ui.vertical_centered(|ui| {
            ui.add_space((slack * 0.5).max(0.0));
            self.word_pill(ui);
            ui.add_space(GAP);
            self.board(ui, board_size);
            ui.add_space(GAP);

            // What kind of board this is.
            let kind = match &self.game.theme {
                Some(theme) => format!("Theme: {theme}"),
                None if !self.game.superwords().is_empty() => match self.game.superword_theme() {
                    Some(theme) => format!("Superword board · Theme: {theme}"),
                    None => "Superword board: one word fills every tile".to_string(),
                },
                None => String::new(),
            };
            ui.add_sized([ui.available_width(), 22.0], egui::Label::new(egui::RichText::new(kind).size(15.0).color(GOLD).strong()));

            // The letter bonus so far, and what earns it.
            ui.label(
                egui::RichText::new(format!("Letter bonus +{}", thousands(self.game.letter_bonus() as usize)))
                    .size(14.0)
                    .color(TEXT)
                    .strong(),
            );
            ui.label(bonus_key());

            let hint = if is_narrow(ui.ctx()) {
                "Drag across touching letters"
            } else {
                "Drag across touching letters — or click them one by one, then Enter"
            };
            ui.label(egui::RichText::new(hint).size(12.0).color(MUTED));
        });
    }

    /// Are you sure? Leaving banks what has been scored, now.
    fn overlay_confirm_leave(&mut self, ctx: &egui::Context) {
        self.page(ctx, false, |app, ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(if is_narrow(ui.ctx()) { 120.0 } else { 10.0 });
                ui.label(egui::RichText::new("Leave this round?").size(26.0).color(TEXT).strong());
                ui.add_space(12.0);
                let score = app.game.score;
                let (line, detail) = if score > 0 {
                    (
                        format!("You'll leave with {} points.", thousands(score as usize)),
                        "They count toward your average and go on the leaderboard now.",
                    )
                } else {
                    ("You haven't found any words yet.".to_string(), "Nothing will be recorded.")
                };
                ui.label(egui::RichText::new(line).size(16.0).color(GOLD).strong());
                ui.label(egui::RichText::new(detail).size(13.0).color(MUTED));
                ui.add_space(24.0);
                if big_button(ui, "LEAVE ROUND", RED) {
                    let me = app.identity.clone();
                    app.live.leave(&mut app.game, me.as_ref(), app.now);
                    app.confirm_leave = false;
                }
                ui.add_space(10.0);
                if big_button(ui, "KEEP PLAYING", ACCENT) {
                    app.confirm_leave = false;
                }
            });
        });
    }

    /// The traced word, coloured by whether it would score.
    fn word_pill(&self, ui: &mut egui::Ui) {
        let (text, color) = match (self.game.path.is_empty(), self.game.feedback) {
            (false, _) => {
                let word = self.game.current_word().to_uppercase();
                let color = if self.game.current_word_is_valid() { GREEN } else { TEXT };
                (word, color)
            }
            (true, Feedback::Accepted) => (
                format!("{}  +{}", self.game.feedback_word.to_uppercase(), self.game.feedback_points),
                GREEN,
            ),
            (true, Feedback::Duplicate) => (format!("{} — already found", self.game.feedback_word.to_uppercase()), AMBER),
            (true, Feedback::TooShort) => ("Too short — 3 letters minimum".to_string(), AMBER),
            (true, Feedback::NotAWord) => (format!("{} — not a word", self.game.feedback_word.to_uppercase()), RED),
            (true, Feedback::None) => (String::new(), MUTED),
        };

        let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 44.0), Sense::hover());
        if text.is_empty() {
            return;
        }

        let painter = ui.painter();
        let font = FontId::proportional(24.0);
        let galley = painter.layout_no_wrap(text.clone(), font.clone(), color);
        let pill = Rect::from_center_size(
            rect.center(),
            Vec2::new(galley.size().x + 36.0, 42.0),
        );
        painter.rect_filled(pill, 21.0, PANEL);
        painter.rect_stroke(pill, 21.0, Stroke::new(1.5_f32, color.gamma_multiply(0.6)), egui::StrokeKind::Inside);
        painter.text(rect.center(), Align2::CENTER_CENTER, text, font, color);
    }

    fn board(&mut self, ui: &mut egui::Ui, board_size: f32) {
        let (response, painter) =
            ui.allocate_painter(Vec2::splat(board_size), Sense::click_and_drag());
        // On a phone the board runs nearly edge to edge, with tighter gaps. Drags
        // are judged on circles sized to the tile, so the gap does not make
        // diagonals any harder.
        let gap = if is_narrow(ui.ctx()) { PHONE_GAP } else { DESKTOP_GAP };
        let geom = BoardGeometry::with_gap(response.rect.min, board_size, gap);

        self.handle_board_input(&response, &geom);

        self.draw_trail(&painter, &geom);

        for row in 0..SIZE {
            for col in 0..SIZE {
                let at = Position { row, col };
                self.draw_tile(&painter, geom.rect(at), at);
            }
        }

        self.draw_score_popup(&painter, response.rect);
    }

    fn handle_board_input(&mut self, response: &egui::Response, geom: &BoardGeometry) {
        if self.game.phase != Phase::Playing {
            return;
        }

        let pointer = response.interact_pointer_pos();

        if response.drag_started() {
            self.game.is_dragging = true;
            self.game.clear_path();
            // A drag only registers once the pointer has moved a few pixels, and a
            // quick flick can be a whole tile away by then. Start from where it
            // came down, and trace the travel since, or the first letter is lost.
            let origin = ui_press_origin(response).or(pointer);
            if let Some(at) = origin.and_then(|p| geom.tile_at(p)) {
                self.game.begin_path(at);
            }
            if let (Some(from), Some(to)) = (origin, pointer) {
                for at in geom.tiles_along(from, to) {
                    self.game.extend_path(at);
                }
            }
            self.drag_from = pointer;
        } else if response.dragged() && self.game.is_dragging {
            if let Some(to) = pointer {
                // Follow the cursor's actual travel since the last frame. Sampling
                // only the current position lets a quick flick jump clean over a
                // tile, which then silently fails the adjacency check.
                let from = self.drag_from.unwrap_or(to);
                for at in geom.tiles_along(from, to) {
                    self.game.extend_path(at);
                }
                self.drag_from = Some(to);
            }
        } else if response.clicked() {
            if let Some(at) = pointer.and_then(|p| geom.tile_at(p)) {
                // Click-to-chain: tap letters in turn, tap the last one again to submit.
                match self.game.path.last() {
                    Some(&last) if last == at => self.game.submit_path(),
                    Some(_) => self.game.extend_path(at),
                    None => self.game.begin_path(at),
                }
            }
        }

        if response.drag_stopped() && self.game.is_dragging {
            self.game.is_dragging = false;
            self.drag_from = None;
            // A one-tile drag is a mis-click, not an attempt at a word.
            if self.game.path.len() > 1 {
                self.game.submit_path();
            } else {
                self.game.clear_path();
            }
        }
    }

    fn draw_tile(&self, painter: &egui::Painter, rect: Rect, pos: Position) {
        let selected = self.game.path_contains(pos);
        let flashing = self.game.feedback_timer > 0.0 && self.game.feedback_cells.contains(&pos);

        let (fill, edge) = if flashing {
            let accent = match self.game.feedback {
                Feedback::Accepted => GREEN,
                Feedback::NotAWord => RED,
                _ => AMBER,
            };
            (accent.gamma_multiply(0.35), accent)
        } else if selected {
            (ACCENT.gamma_multiply(0.35), ACCENT)
        } else {
            (TILE, TILE_EDGE)
        };

        // Accepted words pop outward briefly.
        let rect = if flashing && self.game.feedback == Feedback::Accepted {
            rect.expand(self.game.feedback_timer * 5.0)
        } else {
            rect
        };

        let radius = rect.width() * 0.18;
        painter.rect_filled(rect, radius, fill);
        painter.rect_stroke(rect, radius, Stroke::new(2.0_f32, edge), egui::StrokeKind::Inside);

        let letters = self.game.grid[pos.row][pos.col].letters;
        // "Qu" needs to be a touch smaller to sit inside the same tile.
        let size = rect.width() * if letters.len() > 1 { 0.38 } else { 0.5 };
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            capitalize(letters),
            FontId::proportional(size),
            if selected || flashing { Color32::WHITE } else { TEXT },
        );

        use_stars(painter, rect, self.game.tile_uses[pos.row][pos.col]);
    }

    fn draw_trail(&self, painter: &egui::Painter, geom: &BoardGeometry) {
        if self.game.path.len() < 2 {
            return;
        }

        let color = if self.game.current_word_is_valid() { GREEN } else { ACCENT };
        let centers: Vec<Pos2> = self.game.path.iter().map(|p| geom.center(*p)).collect();

        for pair in centers.windows(2) {
            painter.line_segment([pair[0], pair[1]], Stroke::new(12.0_f32, color.gamma_multiply(0.7)));
        }
    }

    /// Floating "+400" that rises and fades after a word lands.
    fn draw_score_popup(&self, painter: &egui::Painter, board: Rect) {
        if self.game.feedback != Feedback::Accepted || self.game.feedback_timer <= 0.0 {
            return;
        }
        let progress = 1.0 - (self.game.feedback_timer / 0.9);
        let pos = board.center_top() + Vec2::new(0.0, board.height() * 0.35 - progress * 60.0);
        painter.text(
            pos,
            Align2::CENTER_CENTER,
            format!("+{}", self.game.feedback_points),
            FontId::proportional(40.0),
            GREEN.gamma_multiply(1.0 - progress),
        );
    }

    // --- found words -------------------------------------------------------

    fn found_panel(&mut self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new("FOUND").size(12.0).color(MUTED).strong());
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(format!("{}", self.game.found.len()))
                    .size(28.0)
                    .color(TEXT)
                    .strong(),
            );
            ui.label(
                egui::RichText::new(format!("/ {} on board", self.game.findable_count()))
                    .size(12.0)
                    .color(MUTED),
            );
        });
        ui.add_space(10.0);
        ui.separator();
        ui.add_space(6.0);

        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            if self.game.found.is_empty() {
                ui.label(egui::RichText::new("Nothing yet — go find a word.").size(12.0).color(MUTED));
            }
            // Newest first, so the last find is always in view.
            for word in self.game.found.iter().rev() {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(word.word.to_uppercase())
                            .size(15.0)
                            .color(if word.word.len() >= 6 { GOLD } else { TEXT }),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(egui::RichText::new(format!("{}", word.points)).size(13.0).color(MUTED));
                    });
                });
            }
        });
    }

    // --- overlays ----------------------------------------------------------

    /// The home screen: the logo, a welcome, and PLAY. On a phone it is the whole
    /// screen rather than a card over the board.
    fn overlay_ready(&mut self, ctx: &egui::Context) {
        self.page(ctx, true, |app, ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(8.0);
                logo(ui);
                ui.add_space(22.0);

                ui.label(egui::RichText::new("Welcome,").size(16.0).color(MUTED));
                let name = app.identity.as_ref().map(|i| i.name.clone()).unwrap_or_default();
                ui.label(egui::RichText::new(name).size(30.0).color(TEXT).strong());
                let rank = &app.game.ranking;
                ui.horizontal(|ui| {
                    // Centre the pair as one line.
                    let league = egui::RichText::new(LEAGUES[rank.league].name)
                        .size(15.0)
                        .color(league_color(rank.league))
                        .strong();
                    let average = egui::RichText::new(format!("  ·  {} avg", thousands(rank.average() as usize)))
                        .size(15.0)
                        .color(MUTED);
                    let width = ui.fonts(|f| {
                        let font = FontId::proportional(15.0);
                        f.layout_no_wrap(LEAGUES[rank.league].name.to_string(), font.clone(), TEXT).size().x
                            + f.layout_no_wrap(format!("  ·  {} avg", thousands(rank.average() as usize)), font, TEXT).size().x
                    });
                    ui.add_space(((ui.available_width() - width) / 2.0).max(0.0));
                    ui.label(league);
                    ui.label(average);
                });
                let to_go = match rank.next_threshold() {
                    Some(next) => format!(
                        "{} more average to reach {}",
                        thousands(next.saturating_sub(rank.average()) as usize),
                        LEAGUES[rank.league + 1].name
                    ),
                    None => "Top of the ladder".to_string(),
                };
                ui.label(egui::RichText::new(to_go).size(12.0).color(MUTED));

                ui.add_space(26.0);
                let (_, note, color) = app.join_prompt();
                let (prompt, prompt_color) = if app.play_swipe.show_hint {
                    ("Swipe across the letters, P to Y, like a word in a round", AMBER)
                } else {
                    ("Swipe PLAY to play", MUTED)
                };
                ui.label(egui::RichText::new(prompt).size(13.0).color(prompt_color).strong());
                ui.add_space(6.0);
                if play_band(ui, &mut app.play_swipe) {
                    app.live.play_now(&mut app.game, app.now);
                }
                ui.add_space(8.0);
                ui.label(egui::RichText::new(note).size(13.0).color(color));
                if app.live.queued(&app.game, app.now) {
                    ui.add_space(6.0);
                    if action_button(ui, "Leave the queue", TILE_EDGE).clicked() {
                        app.live.leave_queue();
                    }
                }

                ui.add_space(26.0);
                ui.horizontal(|ui| {
                    let buttons = 2.0;
                    let width = 72.0 * buttons + 16.0;
                    ui.add_space(((ui.available_width() - width) / 2.0).max(0.0));
                    if icon_button(ui, "Stats", draw_chart_icon).clicked() {
                        app.show_stats = true;
                    }
                    ui.add_space(16.0);
                    let copied = app.code_copied_at.is_some_and(|t| app.now - t < 3.0);
                    let label = if copied { "Copied" } else { "My code" };
                    if icon_button(ui, label, if copied { draw_tick_icon } else { draw_copy_icon }).clicked() {
                        if let Some(code) = app.identity.as_ref().map(|me| me.recovery_code()) {
                            ui.ctx().copy_text(code);
                            app.code_copied_at = Some(app.now);
                        }
                    }
                });
                ui.add_space(8.0);
            });
        });
    }

    /// What the start button says, and the line under it, given the shared clock.
    fn join_prompt(&self) -> (String, String, Color32) {
        let clock = self.live.clock().filter(|_| self.live.is_live(self.now));
        let waiting = self.live.joined();

        match (clock, self.live.link()) {
            (Some(c), _) if c.playing && c.left >= MIN_JOIN_SECONDS && !waiting => (
                "JOIN ROUND".to_string(),
                format!("Everyone is on this board · {} left · or press Enter", clock_text(c.left)),
                GREEN,
            ),
            (Some(_), _) => {
                let next = self.next_shared_round_in().unwrap_or(0.0);
                if waiting {
                    let text = format!("You're queued for the next round · starts in {}", clock_text(next));
                    ("QUEUED".to_string(), text, GREEN)
                } else {
                    ("JOIN NEXT ROUND".to_string(), format!("Next round starts in {}", clock_text(next)), AMBER)
                }
            }
            (None, Link::Connecting) if self.live.is_live(self.now) => (
                "START ROUND".to_string(),
                "Connecting to the live round…".to_string(),
                MUTED,
            ),
            (None, Link::Solo) => ("START ROUND".to_string(), "or press Enter".to_string(), MUTED),
            (None, _) => (
                "START ROUND".to_string(),
                "Can't reach the server · playing solo boards".to_string(),
                AMBER,
            ),
        }
    }

    fn overlay_results(&mut self, ctx: &egui::Context) {
        if is_narrow(ctx) {
            return self.page(ctx, false, |app, ui| app.results_page(ui));
        }
        self.overlay(ctx, |app, ui| {
            ui.label(egui::RichText::new("TIME'S UP").size(15.0).color(MUTED).strong());
            ui.label(egui::RichText::new(format!("{}", app.game.score)).size(46.0).color(GOLD).strong());
            ui.label(
                egui::RichText::new(LEAGUES[app.game.ranking.league].name.to_uppercase())
                    .size(20.0)
                    .color(league_color(app.game.ranking.league))
                    .strong(),
            );

            if app.game.is_new_best() {
                ui.label(egui::RichText::new("\u{2605} NEW PERSONAL BEST").size(14.0).color(GOLD).strong());
            }

            ui.add_space(14.0);

            // The columns below need this much room; with less -- a phone, a small
            // tablet -- the card stacks them instead.
            const COLUMNS_WIDTH: f32 = 196.0 + 16.0 + 250.0 + 16.0 + 300.0;
            let narrow = ui.available_width() < COLUMNS_WIDTH;

            // The board you just played, the numbers that came out of it, and where
            // that leaves you in the league: side by side on a desktop, so the card
            // still fits; one above another on a phone, where it scrolls.
            if narrow {
                app.used_board(ui, 180.0);
                ui.add_space(12.0);
                app.stacked(ui, |app, ui| app.round_stats(ui));
                ui.add_space(12.0);
                app.stacked(ui, |app, ui| app.form_panel(ui));
            } else {
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(196.0);
                        app.used_board(ui, 180.0);
                    });
                    ui.add_space(16.0);
                    ui.vertical(|ui| {
                        ui.set_width(250.0);
                        app.round_stats(ui);
                    });
                    ui.add_space(16.0);
                    ui.vertical(|ui| {
                        ui.set_width(300.0);
                        app.form_panel(ui);
                    });
                });
            }

            ui.add_space(12.0);
            app.rank_banner(ui);
            app.unplayed_notice(ui);

            match app.game.round {
                Some(round) if narrow => {
                    app.stacked(ui, |app, ui| app.leaderboard(ui, round));
                    ui.add_space(12.0);
                    app.stacked(ui, |app, ui| app.word_area(ui, 130.0));
                    ui.add_space(18.0);
                    app.results_footer(ui);
                }
                Some(round) => {
                    ui.horizontal_top(|ui| {
                        ui.vertical(|ui| {
                            ui.set_width(300.0);
                            app.leaderboard(ui, round);
                        });
                        ui.add_space(16.0);
                        ui.vertical(|ui| {
                            ui.set_width((ui.available_width()).max(200.0));
                            app.word_area(ui, 130.0);
                        });
                    });
                    ui.add_space(18.0);
                    app.results_footer(ui);
                }
                None => {
                    app.word_area(ui, 130.0);
                    ui.add_space(18.0);
                    app.results_footer(ui);
                }
            }
        });
    }

    /// The scorecard on a phone: one screen, no scrolling but the word list's own.
    /// The board and its numbers side by side at the top, the tabs under them, the
    /// list filling what is left, and the countdown pinned to the bottom.
    fn results_page(&mut self, ui: &mut egui::Ui) {
        const FOOTER: f32 = 56.0;
        let width = ui.available_width();

        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("TIME'S UP").size(12.0).color(MUTED).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let rank = &self.game.ranking;
                ui.label(egui::RichText::new(format!("{} avg", thousands(rank.average() as usize))).size(12.0).color(MUTED));
                ui.label(
                    egui::RichText::new(LEAGUES[rank.league].name.to_uppercase())
                        .size(12.0)
                        .color(league_color(rank.league))
                        .strong(),
                );
            });
        });
        ui.add_space(6.0);

        let board = (width * 0.46).min(210.0);
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(board);
                self.used_board(ui, board);
            });
            ui.add_space(10.0);
            ui.vertical(|ui| {
                ui.set_width(ui.available_width());
                self.compact_stats(ui);
            });
        });

        ui.add_space(4.0);
        self.rank_banner(ui);
        self.unplayed_notice(ui);

        let tabs = self.tab_list();
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            for (tab, label) in &tabs {
                let selected = self.current_tab() == *tab;
                if tab_button(ui, label, selected) {
                    self.results_tab = *tab;
                }
            }
        });
        ui.separator();

        let list_height = (ui.available_height() - FOOTER).max(80.0);
        if self.letter_focus.is_some() {
            self.letter_words(ui, list_height);
        } else {
            match (self.current_tab(), self.game.round) {
            (ResultsTab::Leaderboard, Some(round)) => {
                egui::ScrollArea::vertical()
                    .id_salt("results_players")
                    .max_height(list_height)
                    .min_scrolled_height(list_height)
                    .auto_shrink([false, false])
                    .show(ui, |ui| self.leaderboard_rows(ui, round));
            }
            _ => self.word_list_columns(ui, list_height),
        }
        }

        ui.add_space(6.0);
        self.results_footer(ui);
    }

    /// Seconds until the next round begins: the scorecard's own window on a shared
    /// round; after a solo one, the next shared round if there is a server, or the
    /// solo scorecard's window.
    fn next_round_in(&self) -> f32 {
        if self.game.round.is_some() {
            return self.game.results_left.max(0.0);
        }
        match self.next_shared_round_in() {
            Some(left) => left.max(self.game.results_left.max(0.0)),
            None => self.game.results_left.max(0.0),
        }
    }

    /// The foot of every scorecard: a countdown to the next round, which starts on
    /// its own, and a way home for a player who is done. Going home loses nothing:
    /// the round was banked and handed in when it ended.
    fn results_footer(&mut self, ui: &mut egui::Ui) {
        let left = self.next_round_in();
        ui.horizontal(|ui| {
            let label = egui::RichText::new(format!("Next round in {}s", left.ceil() as u32))
                .size(15.0)
                .color(if left <= 10.0 { AMBER } else { MUTED })
                .strong();
            let width = ui.fonts(|f| f.layout_no_wrap(format!("Next round in {}s", left.ceil() as u32), FontId::proportional(15.0), TEXT).size().x);
            let button_width = 130.0;
            ui.add_space(((ui.available_width() - width - button_width - 16.0) / 2.0).max(0.0));
            ui.label(label);
            ui.add_space(16.0);
            if action_button(ui, "Back to home", ACCENT).clicked() {
                self.live.close_results(&mut self.game);
            }
        });
    }

    /// The round's numbers, narrow enough to sit beside the board on a phone.
    fn compact_stats(&self, ui: &mut egui::Ui) {
        let g = &self.game;
        ui.label(egui::RichText::new(thousands(g.score as usize)).size(26.0).color(GOLD).strong());
        ui.label(
            egui::RichText::new(format!("of {} possible", thousands(g.board_par() as usize))).size(11.0).color(MUTED),
        );
        ui.add_space(6.0);
        let rows = [
            ("Words found", format!("{} / {}", g.found_count(), g.findable_count())),
            ("Avg points/word", format!("{:.0}", g.average_points())),
            ("Time per word", format!("{:.1}s", g.seconds_per_word())),
            ("Avg word length", format!("{:.1}", g.average_word_length())),
            ("Letter bonus", format!("+{}", thousands(g.letter_bonus() as usize))),
            ("Board type", g.board_type()),
        ];
        for (label, value) in rows {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(label).size(11.0).color(MUTED));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(egui::RichText::new(value).size(12.0).color(TEXT).strong());
                });
            });
        }
    }

    /// The tabs this scorecard offers: the leaderboard, with how many played, on a
    /// shared round; the word lists always.
    fn tab_list(&self) -> Vec<(ResultsTab, String)> {
        let mut tabs = Vec::new();
        if let Some(round) = self.game.round {
            let players = self.live.leaderboard_for(round).map(|t| t.entries.len()).unwrap_or(0);
            tabs.push((ResultsTab::Leaderboard, format!("Players ({players})")));
        }
        tabs.push((ResultsTab::Words, "Words".to_string()));
        tabs
    }

    /// The selected tab, or the word lists where there is no leaderboard.
    fn current_tab(&self) -> ResultsTab {
        match self.results_tab {
            ResultsTab::Leaderboard if self.game.round.is_none() => ResultsTab::Words,
            tab => tab,
        }
    }

    /// The scorecard's word area: a tapped letter's words when there is one,
    /// otherwise the three lists.
    fn word_area(&mut self, ui: &mut egui::Ui, height: f32) {
        if self.letter_focus.is_some() {
            self.letter_words(ui, height);
        } else {
            self.word_list_columns(ui, height);
        }
    }

    /// Every word through the tapped letter, found ones in green, in columns that
    /// scroll together. Tap a word to draw its path on the board.
    fn letter_words(&mut self, ui: &mut egui::Ui, height: f32) {
        const HEADER: f32 = 54.0;
        let Some(focus) = self.letter_focus.as_ref() else { return };
        let letter = capitalize(self.game.grid[focus.at.row][focus.at.col].letters);
        let found = focus.words.iter().filter(|(w, _)| self.game.has_found(w)).count();

        let mut close = false;
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(format!("Words through {letter}  ({found}/{})", focus.words.len()))
                    .size(12.0)
                    .color(TEXT)
                    .strong(),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                close = action_button(ui, "Back to lists", TILE_EDGE).clicked();
            });
        });
        ui.separator();
        if close {
            self.letter_focus = None;
            return;
        }

        let body = (height - HEADER).max(40.0);
        let mut picked = None;
        egui::ScrollArea::vertical()
            .id_salt("letter_words")
            .max_height(body)
            .min_scrolled_height(body)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let focus = self.letter_focus.as_ref().expect("checked above");
                if focus.words.is_empty() {
                    ui.label(egui::RichText::new("No word on this board uses that letter.").size(12.0).color(MUTED));
                    return;
                }
                ui.label(egui::RichText::new("Tap a word to see how it runs.").size(11.0).color(MUTED));
                let columns = ((ui.available_width() / 120.0).floor() as usize).clamp(1, 4);
                let per_column = focus.words.len().div_ceil(columns);
                ui.columns(columns, |cols| {
                    for (i, (word, path)) in focus.words.iter().enumerate() {
                        let col = &mut cols[i / per_column];
                        col.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                        let found = self.game.has_found(word);
                        let shown = self.shown_word.as_ref().is_some_and(|(w, _)| w == word);
                        let text = egui::RichText::new(format!("{word}  {}", self.game.word_score(word)))
                            .size(12.0)
                            .color(if found { GREEN } else { TEXT });
                        if col.selectable_label(shown, text).clicked() {
                            picked = Some(if shown { None } else { Some((word.clone(), path.clone())) });
                        }
                    }
                });
            });
        if let Some(choice) = picked {
            self.shown_word = choice;
        }
    }

    /// Common, Obscure and the board's own list as three columns side by side,
    /// each headed with found of total and scrolling on its own, the way WordHero
    /// laid them out. Found words are picked out in green, each with its points.
    fn word_list_columns(&mut self, ui: &mut egui::Ui, height: f32) {
        /// The column header and the rule under it.
        const HEADER: f32 = 26.0;
        let (highlight, highlighted) = self.game.highlight_tab();
        let lists: [(String, Vec<&String>); 3] = [
            ("Common".to_string(), self.game.words.common.iter().collect()),
            ("Obscure".to_string(), self.game.words.obscure.iter().collect()),
            (highlight, highlighted),
        ];
        let body = (height - HEADER).max(40.0);
        let mut picked: Option<String> = None;

        ui.columns(3, |cols| {
            for (i, (col, (name, words))) in cols.iter_mut().zip(lists).enumerate() {
                col.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                let found = words.iter().filter(|w| self.game.has_found(w)).count();
                col.label(egui::RichText::new(format!("{name} ({found}/{})", words.len())).size(12.0).color(TEXT).strong());
                col.separator();
                egui::ScrollArea::vertical()
                    .id_salt(("word_column", i))
                    .max_height(body)
                    .min_scrolled_height(body)
                    .auto_shrink([false, false])
                    .show(col, |ui| {
                        if words.is_empty() {
                            ui.label(egui::RichText::new("None on this board").size(11.0).color(MUTED));
                            return;
                        }
                        let width = ui.available_width();
                        for word in words {
                            let found = self.game.has_found(word);
                            let shown = self.shown_word.as_ref().is_some_and(|(w, _)| w == word);
                            let color = if shown { GOLD } else if found { GREEN } else { TEXT };
                            ui.horizontal(|ui| {
                                // The word gives way to its points when space is short.
                                ui.allocate_ui(Vec2::new((width - 30.0).max(20.0), 16.0), |ui| {
                                    let label = ui.add(
                                        egui::Label::new(egui::RichText::new(word.as_str()).size(12.0).color(color))
                                            .truncate()
                                            .sense(Sense::click()),
                                    );
                                    if label.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                                        picked = Some(word.clone());
                                    }
                                });
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    ui.label(
                                        egui::RichText::new(self.game.word_score(word).to_string())
                                            .size(11.0)
                                            .color(MUTED),
                                    );
                                });
                            });
                        }
                    });
            }
        });

        // Tapping a word draws its path on the board; tapping it again clears it.
        if let Some(word) = picked {
            if self.shown_word.as_ref().is_some_and(|(w, _)| *w == word) {
                self.shown_word = None;
            } else if let Some(path) = crate::game::find_path(&self.game.grid, &word) {
                self.shown_word = Some((word, path));
            }
        }
    }

    /// Everyone who played this board, best first.
    fn leaderboard(&self, ui: &mut egui::Ui, round: u64) {
        let mine = self.identity.as_ref().map(|i| i.name.to_lowercase()).unwrap_or_default();
        let table = self.live.leaderboard_for(round);
        let place = table.and_then(|t| t.entries.iter().position(|e| e.name.to_lowercase() == mine));

        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("LEADERBOARD").size(13.0).color(TEXT).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let text = match (table, place) {
                    (Some(t), Some(i)) => format!("you're #{} of {}", i + 1, t.entries.len()),
                    (Some(t), None) => format!("{} played", t.entries.len()),
                    (None, _) => String::new(),
                };
                ui.label(egui::RichText::new(text).size(11.0).color(GOLD).strong());
            });
        });
        ui.add_space(6.0);

        egui::ScrollArea::vertical()
            .id_salt("leaderboard")
            .max_height(96.0)
            .auto_shrink([false, true])
            .show(ui, |ui| self.leaderboard_rows(ui, round));
    }

    /// The leaderboard's rows, or why there are none yet.
    fn leaderboard_rows(&self, ui: &mut egui::Ui, round: u64) {
        let mine = self.identity.as_ref().map(|i| i.name.to_lowercase()).unwrap_or_default();
        let table = self.live.leaderboard_for(round);
        let place = table.and_then(|t| t.entries.iter().position(|e| e.name.to_lowercase() == mine));

        let Some(table) = table.filter(|t| !t.entries.is_empty()) else {
            let waiting = match self.live.name {
                NameStatus::Claimed => "Collecting scores…",
                _ => "Your name isn't registered yet, so this round can't be ranked.",
            };
            ui.label(egui::RichText::new(waiting).size(12.0).color(MUTED));
            return;
        };
        for (i, entry) in table.entries.iter().enumerate() {
            let me = Some(i) == place;
            let color = if me { GOLD } else { TEXT };
            ui.horizontal(|ui| {
                ui.add_sized(
                    [26.0, 16.0],
                    egui::Label::new(egui::RichText::new(format!("{}.", i + 1)).size(12.0).color(MUTED)),
                );
                ui.label(egui::RichText::new(&entry.name).size(13.0).color(color).strong());
                if let Some(league) = entry.league.filter(|l| *l < LEAGUES.len()) {
                    ui.label(egui::RichText::new(LEAGUES[league].name).size(11.0).color(league_color(league)).strong());
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // A row still waiting on its final score shows as it stood.
                    let detail = if entry.finished { format!("{} words", entry.words) } else { "finishing…".to_string() };
                    ui.label(egui::RichText::new(detail).size(11.0).color(MUTED));
                    ui.label(egui::RichText::new(thousands(entry.score as usize)).size(13.0).color(color).strong());
                });
            });
        }
    }

    /// The played board, with each tile bordered by how hard it worked.
    /// The played board. Tap a letter to see every word that runs through it.
    fn used_board(&mut self, ui: &mut egui::Ui, size: f32) {
        let (response, painter) = ui.allocate_painter(Vec2::splat(size), Sense::click());
        let geom = BoardGeometry::new(response.rect.min, size);

        if response.clicked() {
            if let Some(at) = response.interact_pointer_pos().and_then(|p| geom.tile_at(p)) {
                // The same letter again puts the lists back.
                if self.letter_focus.as_ref().is_some_and(|f| f.at == at) {
                    self.letter_focus = None;
                } else {
                    self.letter_focus = Some(LetterFocus { at, words: self.game.words_through(at) });
                }
            }
        }
        let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
        let _ = response;


        for row in 0..SIZE {
            for col in 0..SIZE {
                let at = Position { row, col };
                let rect = geom.rect(at);
                let uses = self.game.tile_uses[row][col];
                let edge = tile_use_color(uses);

                painter.rect_filled(rect, rect.width() * 0.18, TILE);
                painter.rect_stroke(
                    rect,
                    rect.width() * 0.18,
                    Stroke::new(2.0_f32, edge),
                    egui::StrokeKind::Inside,
                );
                painter.text(
                    rect.center(),
                    Align2::CENTER_CENTER,
                    capitalize(self.game.grid[row][col].letters),
                    FontId::proportional(rect.width() * 0.45),
                    if uses == 0 { MUTED } else { TEXT },
                );
                use_stars(&painter, rect, uses);
                if self.letter_focus.as_ref().is_some_and(|f| f.at == at) {
                    painter.rect_stroke(rect.expand(2.0), rect.width() * 0.2, Stroke::new(3.0_f32, GOLD), egui::StrokeKind::Outside);
                }
            }
        }

        // The path of the word picked from the letter's list, drawn over the tiles
        // so it cannot hide behind them, and translucent so the letters still read
        // through it. A dot marks where the word starts.
        if let Some((_, path)) = self.shown_word.as_ref() {
            let centres: Vec<Pos2> = path.iter().map(|p| geom.center(*p)).collect();
            let color = ACCENT.gamma_multiply(0.6);
            for pair in centres.windows(2) {
                painter.line_segment([pair[0], pair[1]], Stroke::new(size * 0.035, color));
            }
            if let Some(start) = centres.first() {
                painter.circle_filled(*start, size * 0.04, color);
            }
        }

        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            for (label, color) in [("unused", BLUE), ("used", AMBER), ("reused", GREEN)] {
                ui.label(egui::RichText::new(format!("\u{25a0} {label}")).size(10.0).color(color));
            }
        });
    }

    fn round_stats(&self, ui: &mut egui::Ui) {
        let g = &self.game;
        let rows: [(&str, String); 7] = [
            ("Score", format!("{} / {}", g.score, g.board_par())),
            (
                "Words found",
                format!(
                    "{} / {}  ({:.0}%)",
                    g.found_count(),
                    g.findable_count(),
                    g.found_fraction() * 100.0
                ),
            ),
            ("Avg points per word", format!("{:.0}", g.average_points())),
            ("Time per word", format!("{:.1}s", g.seconds_per_word())),
            ("Avg word length", format!("{:.1}", g.average_word_length())),
            ("Letter bonus", format!("+{}", thousands(g.letter_bonus() as usize))),
            ("Board type", g.board_type()),
        ];

        for (label, value) in rows {
            ui.horizontal(|ui| {
                ui.add_sized(
                    [150.0, 18.0],
                    egui::Label::new(egui::RichText::new(label).size(12.0).color(MUTED)),
                );
                ui.label(egui::RichText::new(value).size(14.0).color(TEXT).strong());
            });
        }
    }

    /// Your last ten rounds, the average they make, and how far that is from the
    /// next league.
    fn form_panel(&self, ui: &mut egui::Ui) {
        let rank = &self.game.ranking;

        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("RANK").size(11.0).color(MUTED).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    egui::RichText::new(format!("last {} rounds", FORM_GAMES))
                        .size(10.0)
                        .color(MUTED),
                );
            });
        });

        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(thousands(rank.average() as usize))
                    .size(28.0)
                    .color(TEXT)
                    .strong(),
            );
            ui.label(egui::RichText::new("avg").size(12.0).color(MUTED));
        });

        self.history_chart(ui, 56.0, FORM_GAMES, false);

        ui.add_space(6.0);

        match rank.next_threshold() {
            Some(next) => {
                let (bar, _) =
                    ui.allocate_exact_size(Vec2::new(ui.available_width(), 8.0), Sense::hover());
                let painter = ui.painter();
                painter.rect_filled(bar, 4.0, TILE);
                let filled = Rect::from_min_size(
                    bar.min,
                    Vec2::new(bar.width() * rank.progress(), bar.height()),
                );
                painter.rect_filled(filled, 4.0, league_color(rank.league + 1));
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(format!(
                        "{} to reach {}",
                        thousands(next.saturating_sub(rank.average()) as usize),
                        LEAGUES[rank.league + 1].name
                    ))
                    .size(11.0)
                    .color(MUTED),
                );
            }
            None => {
                ui.label(
                    egui::RichText::new("Top of the ladder").size(11.0).color(GOLD).strong(),
                );
            }
        }

        if rank.games_on_record() < MIN_GAMES_TO_MOVE {
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(format!(
                    "{} more rounds before the ladder moves",
                    MIN_GAMES_TO_MOVE - rank.games_on_record()
                ))
                .size(11.0)
                .color(AMBER),
            );
        }
    }

    /// Game scores and the rank average as two lines, newest on the right: each
    /// game against where it left the average, so a run of good or bad games shows
    /// as the average bending. `axes` adds gridlines and labels for the stats page;
    /// without them it is a compact trend for the rank panel. Hovering snaps to the
    /// nearest game and reads both values off.
    fn history_chart(&self, ui: &mut egui::Ui, height: f32, limit: usize, axes: bool) {
        let history = &self.game.ranking.history;
        let skip = history.len().saturating_sub(limit);
        let mut games: Vec<GameRecord> = history.iter().skip(skip).copied().collect();
        // The average line is worked out from the history's own scores -- each game
        // with up to nine before it -- so the first game's average is its score.
        for (game, average) in games.iter_mut().zip(self.game.ranking.history_averages().into_iter().skip(skip)) {
            game.average = average;
        }

        // Two series, so a legend, always; identity never rests on colour alone.
        ui.horizontal(|ui| {
            legend_key(ui, SERIES_SCORES, "Game scores");
            ui.add_space(12.0);
            legend_key(ui, SERIES_AVERAGE, "Average");
        });

        let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover());
        let (left, bottom) = if axes { (38.0, 18.0) } else { (6.0, 6.0) };
        let plot = Rect::from_min_max(rect.min + Vec2::new(left, 6.0), Pos2::new(rect.max.x - 8.0, rect.max.y - bottom));
        let painter = ui.painter();
        let grid = Stroke::new(1.0_f32, TILE_EDGE);

        if games.is_empty() {
            painter.line_segment([plot.left_bottom(), plot.right_bottom()], grid);
            painter.text(plot.center(), Align2::CENTER_CENTER, "Your rounds will chart here", FontId::proportional(11.0), MUTED);
            return;
        }

        let peak = games.iter().map(|g| g.score.max(g.average)).max().unwrap_or(1).max(1);
        let (top, step) = nice_scale(peak);
        // Zero baseline: a line's height is an honest share of the scale.
        let y = |v: u32| plot.max.y - v as f32 / top as f32 * plot.height();

        if axes {
            let mut value = 0;
            while value <= top {
                let at = y(value);
                painter.line_segment([Pos2::new(plot.min.x, at), Pos2::new(plot.max.x, at)], grid);
                painter.text(Pos2::new(plot.min.x - 6.0, at), Align2::RIGHT_CENTER, short_number(value), FontId::proportional(10.0), MUTED);
                value += step;
            }
        } else {
            painter.line_segment([plot.left_bottom(), plot.right_bottom()], grid);
        }

        // Fixed slots, newest at the right edge: the line grows leftward.
        let slots = limit.max(2);
        let dx = plot.width() / (slots - 1) as f32;
        let first = slots - games.len();
        let x = |i: usize| plot.min.x + (first + i) as f32 * dx;

        if axes {
            let label = |at: f32, text: &str| {
                painter.text(Pos2::new(at, plot.max.y + 4.0), Align2::CENTER_TOP, text, FontId::proportional(10.0), MUTED);
            };
            label(plot.max.x - 14.0, "Newest");
            let mut ago = 10;
            while ago < games.len() {
                let at = x(games.len() - 1 - ago);
                if at - plot.min.x > 34.0 {
                    label(at, &format!("-{ago}"));
                }
                ago += 10;
            }
            if games.len() > 1 {
                label(x(0) + 12.0, "Oldest");
            }
        }

        let hovered = response
            .hover_pos()
            .filter(|p| plot.expand(10.0).contains(*p))
            .map(|p| (((p.x - plot.min.x) / dx).round() as isize - first as isize).clamp(0, games.len() as isize - 1) as usize);
        if let Some(i) = hovered {
            painter.line_segment([Pos2::new(x(i), plot.min.y), Pos2::new(x(i), plot.max.y)], grid);
        }

        for (color, value) in [(SERIES_SCORES, (|g: &GameRecord| g.score) as fn(&GameRecord) -> u32), (SERIES_AVERAGE, |g: &GameRecord| g.average)] {
            let points: Vec<Pos2> = games.iter().enumerate().map(|(i, g)| Pos2::new(x(i), y(value(g)))).collect();
            if points.len() > 1 {
                painter.add(egui::Shape::line(points.clone(), Stroke::new(2.0_f32, color)));
            }
            // An end dot on the newest game, and on the one under the pointer, each
            // ringed in the panel colour so it reads where the lines cross.
            let marked = std::iter::once(points.len() - 1).chain(hovered);
            for i in marked {
                painter.circle(points[i], 4.0, color, Stroke::new(2.0_f32, PANEL));
            }
        }

        if let Some(i) = hovered {
            let game = games[i];
            let ago = games.len() - 1 - i;
            let when = match ago {
                0 => "Last game".to_string(),
                1 => "1 game ago".to_string(),
                n => format!("{n} games ago"),
            };
            response.on_hover_ui_at_pointer(|ui| {
                ui.label(egui::RichText::new(when).size(11.0).color(MUTED));
                ui.label(egui::RichText::new(format!("Score {}", thousands(game.score as usize))).size(13.0).color(TEXT).strong());
                ui.label(egui::RichText::new(format!("Average {}", thousands(game.average as usize))).size(12.0).color(TEXT));
                ui.label(egui::RichText::new(format!("{} words", game.words)).size(11.0).color(MUTED));
            });
        }
    }

    /// The stats page: the rank chart over the last fifty games, and the best game,
    /// all-time averages and totals under it.
    fn overlay_stats(&mut self, ctx: &egui::Context) {
        self.page(ctx, true, |app, ui| {
            let narrow = is_narrow(ui.ctx());
            let (league, average, best_score) =
                (app.game.ranking.league, app.game.ranking.average(), app.game.ranking.best_score);
            let life = app.game.ranking.lifetime.clone();

            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("STATS").size(22.0).color(TEXT).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if action_button(ui, "Back", ACCENT).clicked() {
                        app.show_stats = false;
                    }
                });
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("League:").size(14.0).color(MUTED));
                ui.label(egui::RichText::new(LEAGUES[league].name).size(14.0).color(league_color(league)).strong());
            });
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Rank average:").size(14.0).color(MUTED));
                ui.label(egui::RichText::new(thousands(average as usize)).size(14.0).color(TEXT).strong());
            });
            ui.add_space(10.0);

            app.stacked(ui, |app, ui| app.history_chart(ui, if narrow { 190.0 } else { 230.0 }, HISTORY_GAMES, true));
            ui.add_space(14.0);

            app.stacked(ui, |_, ui| league_ladder(ui, league, average));
            ui.add_space(14.0);

            let per_game = |total: u64| if life.games == 0 { "—".to_string() } else { thousands((total / life.games) as usize) };
            let best_word = match &life.best_word {
                Some((word, points)) => format!("{word} ({points} points)"),
                None => "—".to_string(),
            };
            let blocks: [(&str, Vec<(&str, String)>); 3] = [
                ("Best Game", vec![
                    ("Score", thousands(best_score as usize)),
                    ("Word", best_word),
                    ("Most words", life.most_words.to_string()),
                ]),
                ("All-time Averages", vec![
                    ("Score/game", per_game(life.score)),
                    ("Words/game", per_game(life.words)),
                    ("Games played", thousands(life.games as usize)),
                ]),
                ("Totals", vec![
                    ("Score", thousands(life.score as usize)),
                    ("Words", thousands(life.words as usize)),
                ]),
            ];

            if narrow {
                for (title, rows) in &blocks {
                    app.stacked(ui, |_, ui| stat_block(ui, title, rows));
                    ui.add_space(10.0);
                }
            } else {
                ui.columns(3, |cols| {
                    for (col, (title, rows)) in cols.iter_mut().zip(&blocks) {
                        stat_block(col, title, rows);
                    }
                });
            }

            ui.add_space(16.0);
            ui.vertical_centered(|ui| {
                if big_button(ui, "BACK", ACCENT) {
                    app.show_stats = false;
                }
            });
        });
    }

    /// A screen of its own: every dialog goes through here. On a phone it fills
    /// the screen; elsewhere it is the usual centred card. `scroll` lets a long page scroll; the phone scorecard
    /// passes false, since it is laid out to fit and scrolls only its word list.
    fn page(&mut self, ctx: &egui::Context, scroll: bool, contents: impl FnOnce(&mut Self, &mut egui::Ui)) {
        if !is_narrow(ctx) {
            return self.overlay(ctx, |app, ui| {
                ui.vertical(|ui| contents(app, ui));
            });
        }
        let screen = ctx.screen_rect();
        const MARGIN: f32 = 12.0;
        egui::Area::new(egui::Id::new("page"))
            .order(egui::Order::Foreground)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                ui.painter().rect_filled(screen, 0.0, BG);
                let inner = screen.shrink(MARGIN);
                ui.allocate_new_ui(egui::UiBuilder::new().max_rect(inner), |ui| {
                    ui.set_width(inner.width());
                    ui.set_max_height(inner.height());
                    if scroll {
                        egui::ScrollArea::vertical()
                            .id_salt("page_scroll")
                            .max_height(inner.height())
                            .auto_shrink([false, true])
                            .show(ui, |ui| contents(self, ui));
                    } else {
                        contents(self, ui);
                    }
                });
            });
    }

    /// A full-width, left-aligned section inside a centred card, for the phone
    /// layout's stacked sections.
    fn stacked(&mut self, ui: &mut egui::Ui, contents: impl FnOnce(&mut Self, &mut egui::Ui)) {
        let width = ui.available_width();
        ui.allocate_ui_with_layout(Vec2::new(width, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
            ui.set_width(width);
            contents(self, ui);
        });
    }

    /// Said when a round had nothing found in it: it is on the leaderboard as a
    /// zero, but the rank average is left alone.
    fn unplayed_notice(&self, ui: &mut egui::Ui) {
        if self.game.counts_toward_rank() {
            return;
        }
        ui.label(
            egui::RichText::new("You didn't find any words this round.")
                .size(14.0)
                .color(AMBER)
                .strong(),
        );
        ui.label(
            egui::RichText::new("It won't count toward your rank, but you're on the leaderboard with 0.")
                .size(12.0)
                .color(MUTED),
        );
        ui.add_space(8.0);
    }

    /// Shown only on a round that actually moved you up or down.
    fn rank_banner(&self, ui: &mut egui::Ui) {
        let Some(change) = self.game.rank_change else { return };
        let color = match change.movement {
            Movement::Promoted => GREEN,
            Movement::Relegated => RED,
            Movement::Held => return,
        };
        let size = if is_narrow(ui.ctx()) { 18.0 } else { 22.0 };
        ui.label(egui::RichText::new(change.headline()).size(size).color(color).strong());
        ui.add_space(8.0);
    }

    /// Dim everything and centre a card on top of it.
    fn overlay(&mut self, ctx: &egui::Context, contents: impl FnOnce(&mut Self, &mut egui::Ui)) {
        let screen = ctx.screen_rect();
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Middle,
            egui::Id::new("overlay_dim"),
        ));
        painter.rect_filled(screen, 0.0, BG.gamma_multiply(0.88));

        // Pinned near the top rather than centred: an Area anchored to the centre
        // only gets the space below its own origin, which capped the scroll area
        // at ~400px and hid the button under a fold.
        let narrow = screen.width() < NARROW;
        let margin: f32 = if narrow { 10.0 } else { 24.0 };
        let inner: i8 = if narrow { 16 } else { 30 };
        let width = 900.0_f32.min(screen.width() - margin * 2.0);

        egui::Area::new(egui::Id::new("overlay"))
            .order(egui::Order::Foreground)
            .anchor(Align2::CENTER_TOP, Vec2::new(0.0, margin))
            .show(ctx, |ui| {
                ui.set_width(width);
                egui::Frame::default()
                    .fill(PANEL)
                    .stroke(Stroke::new(1.0_f32, TILE_EDGE))
                    .corner_radius(18.0)
                    .inner_margin(egui::Margin::same(inner))
                    .show(ui, |ui| {
                        // Laid out directly when there is room: a ScrollArea here
                        // collapses to about 400px regardless of the max height it
                        // is given, which hid the button under a fold. On a window
                        // too short for the card -- a phone in landscape, say --
                        // scrolling beats an unreachable button.
                        // A phone's cards are stacked and long, so they always scroll.
                        if screen.height() < SHORT_WINDOW || narrow {
                            egui::ScrollArea::vertical()
                                .id_salt("overlay_scroll")
                                .max_height(screen.height() - margin * 2.0 - inner as f32 * 2.0)
                                .auto_shrink([false, true])
                                .show(ui, |ui| {
                                    ui.vertical_centered(|ui| contents(self, ui));
                                });
                        } else {
                            ui.vertical_centered(|ui| contents(self, ui));
                        }
                    });
            });
    }
}

/// Where the tiles sit, and which one a point is on.
///
/// The gaps between tiles deliberately belong to *no* tile. An earlier version
/// assigned them to the nearest neighbour so that fast drags could not skip a
/// letter, but that made the point where four tiles meet resolve to one of them:
/// dragging diagonally clipped a corner and picked up a letter nobody aimed at,
/// and if that letter was already in the word, the revisit rewound the path and
/// destroyed it mid-drag. Skipping is prevented by tracing the cursor's travel
/// instead -- see `tiles_along`.
pub struct BoardGeometry {
    origin: Pos2,
    tile: f32,
    gap: f32,
}

/// Gap between tiles, as a fraction of the board's width.
const DESKTOP_GAP: f32 = 0.035;
const PHONE_GAP: f32 = 0.02;

impl BoardGeometry {
    pub fn new(origin: Pos2, board_size: f32) -> Self {
        Self::with_gap(origin, board_size, DESKTOP_GAP)
    }

    pub fn with_gap(origin: Pos2, board_size: f32, gap_fraction: f32) -> Self {
        let gap = board_size * gap_fraction;
        let tile = (board_size - gap * (SIZE as f32 + 1.0)) / SIZE as f32;
        Self { origin, tile, gap }
    }

    pub fn rect(&self, at: Position) -> Rect {
        let step = self.tile + self.gap;
        Rect::from_min_size(
            self.origin
                + Vec2::new(
                    self.gap + at.col as f32 * step,
                    self.gap + at.row as f32 * step,
                ),
            Vec2::splat(self.tile),
        )
    }

    pub fn center(&self, at: Position) -> Pos2 {
        self.rect(at).center()
    }

    /// The tile under a point, or `None` in the gaps and outside the board.
    pub fn tile_at(&self, pos: Pos2) -> Option<Position> {
        let local = pos - self.origin;
        let step = self.tile + self.gap;
        let col = ((local.x - self.gap) / step).floor();
        let row = ((local.y - self.gap) / step).floor();
        if row < 0.0 || col < 0.0 || row >= SIZE as f32 || col >= SIZE as f32 {
            return None;
        }
        let at = Position { row: row as usize, col: col as usize };
        // The floor above only narrows it to a candidate; the point still has to
        // land on the tile itself rather than in the gap after it.
        self.rect(at).contains(pos).then_some(at)
    }

    /// The tile whose centre circle a point is inside, for a drag already under
    /// way.
    ///
    /// A drag is judged on circles, not the tiles' squares. A diagonal passes
    /// right by the corners of the two tiles beside it, and on squares a finger
    /// only a few pixels off the true diagonal clipped one and picked up a letter
    /// nobody aimed at. Circles leave those corners empty: a diagonal can wander
    /// about a third of a tile off line before it touches a neighbour, while a
    /// straight drag still catches every tile it crosses near the middle.
    pub fn tile_near(&self, pos: Pos2) -> Option<Position> {
        let at = self.nearest(pos)?;
        (self.center(at).distance(pos) <= self.tile * DRAG_HIT_RADIUS).then_some(at)
    }

    /// The tile whose square cell (tile plus half the gap around it) a point is in.
    fn nearest(&self, pos: Pos2) -> Option<Position> {
        let local = pos - self.origin - Vec2::splat(self.gap * 0.5);
        let step = self.tile + self.gap;
        let col = (local.x / step).floor();
        let row = (local.y / step).floor();
        if row < 0.0 || col < 0.0 || row >= SIZE as f32 || col >= SIZE as f32 {
            return None;
        }
        Some(Position { row: row as usize, col: col as usize })
    }

    /// Every tile the segment `from` -> `to` passes through the centre circle of,
    /// in order, without repeats.
    pub fn tiles_along(&self, from: Pos2, to: Pos2) -> Vec<Position> {
        // A tenth of a tile, so even a segment that only grazes a circle is sampled
        // inside it.
        let stride = (self.tile * 0.1).max(1.0);
        let steps = ((from.distance(to) / stride).ceil() as usize).clamp(1, 512);

        let mut out: Vec<Position> = Vec::new();
        for i in 1..=steps {
            let t = i as f32 / steps as f32;
            let point = from + (to - from) * t;
            if let Some(at) = self.tile_near(point) {
                if out.last() != Some(&at) {
                    out.push(at);
                }
            }
        }
        out
    }
}

/// Radius of a tile's drag target, as a fraction of the tile's width.
const DRAG_HIT_RADIUS: f32 = 0.40;

/// Blue for a tile no word used, amber for one used once, green for one reused.
fn tile_use_color(uses: u32) -> Color32 {
    match uses {
        0 => BLUE,
        1 => AMBER,
        _ => GREEN,
    }
}

fn league_color(league: usize) -> Color32 {
    match league {
        0 => Color32::from_rgb(0xc0, 0x84, 0x57), // bronze
        1 => Color32::from_rgb(0xc3, 0xcb, 0xd8), // silver
        2 => GOLD,
        3 => Color32::from_rgb(0x8f, 0xe0, 0xd0), // platinum
        4 => Color32::from_rgb(0x7f, 0xd4, 0xff), // diamond
        _ => Color32::from_rgb(0xff, 0x8a, 0xd8), // hero
    }
}

/// The usual two-overlapping-sheets copy icon, drawn rather than taken from a
/// font so it looks the same everywhere. Turns into a tick once copied.
fn copy_icon(ui: &mut egui::Ui, rect: Rect, copied: bool) -> egui::Response {
    let response = ui
        .interact(rect, egui::Id::new(COPY_CODE_ID), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(if copied { "Copied" } else { "Copy code" });

    let painter = ui.painter();
    let background = if response.hovered() { TILE_EDGE } else { TILE };
    painter.rect_filled(rect, 8.0, background);

    let c = rect.center();
    if copied {
        let tick = vec![c + Vec2::new(-8.0, 0.0), c + Vec2::new(-2.5, 5.5), c + Vec2::new(8.0, -6.0)];
        painter.add(egui::Shape::line(tick, Stroke::new(2.5_f32, GREEN)));
    } else {
        let color = if response.hovered() { TEXT } else { GOLD };
        let stroke = Stroke::new(1.8_f32, color);
        let back = Rect::from_min_size(c + Vec2::new(-8.0, -8.0), Vec2::splat(11.0));
        let front = back.translate(Vec2::splat(5.0));
        painter.rect_stroke(back, 2.0, stroke, egui::StrokeKind::Middle);
        // The front sheet hides the back one where they overlap.
        painter.rect_filled(front, 2.0, background);
        painter.rect_stroke(front, 2.0, stroke, egui::StrokeKind::Middle);
    }
    response
}

/// Stars in a tile's bottom-right corner for the words it has been used in: a
/// yellow star once it has been used, and a green one beside it once it has been
/// used twice -- so it is plain which letters are still waiting to be used.
fn use_stars(painter: &egui::Painter, tile: Rect, uses: u32) {
    if uses == 0 {
        return;
    }
    let size = (tile.width() * 0.2).max(7.0);
    let inset = tile.width() * 0.07;
    let corner = tile.right_bottom() - Vec2::splat(inset);
    painter.text(corner, Align2::RIGHT_BOTTOM, "\u{2605}", FontId::proportional(size), GOLD);
    if uses >= 2 {
        painter.text(corner - Vec2::new(size * 0.95, 0.0), Align2::RIGHT_BOTTOM, "\u{2605}", FontId::proportional(size), GREEN);
    }
}

/// WORD over LEGEND in black and gold letter tiles.
fn logo(ui: &mut egui::Ui) {
    const GAP: f32 = 5.0;
    let size = ((ui.available_width() - GAP * 5.0) / 6.0).min(48.0);
    for word in ["WORD", "LEGEND"] {
        let width = word.len() as f32 * size + (word.len() - 1) as f32 * GAP;
        let (row, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), size), Sense::hover());
        let painter = ui.painter();
        let start = row.center().x - width / 2.0;
        for (i, letter) in word.chars().enumerate() {
            let tile = Rect::from_min_size(Pos2::new(start + i as f32 * (size + GAP), row.min.y), Vec2::splat(size));
            painter.rect_filled(tile, size * 0.18, LOGO_TILE);
            painter.rect_stroke(tile, size * 0.18, Stroke::new(2.0_f32, GOLD), egui::StrokeKind::Inside);
            painter.text(tile.center(), Align2::CENTER_CENTER, letter, FontId::proportional(size * 0.62), GOLD);
        }
        ui.add_space(GAP);
    }
}

/// Where the pointer came down for the drag or press this response is part of.
fn ui_press_origin(response: &egui::Response) -> Option<Pos2> {
    response.ctx.input(|i| i.pointer.press_origin())
}

/// PLAY spelled in tiles across a band, played like the board: swipe P-L-A-Y in
/// order to start. The same circles decide what a swipe touches, the same trail
/// follows the finger, and tracing back over a tile rewinds to it. A tap or any
/// other swipe starts nothing and turns on the hint above.
fn play_band(ui: &mut egui::Ui, swipe: &mut PlaySwipe) -> bool {
    const GAP: f32 = 8.0;
    const WORD: [char; 4] = ['P', 'L', 'A', 'Y'];
    let size = ((ui.available_width() - 40.0 - GAP * 3.0) / 4.0).min(66.0);
    let (band, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), size + 28.0), Sense::click_and_drag());

    let width = size * 4.0 + GAP * 3.0;
    let start = band.center().x - width / 2.0;
    let tiles: Vec<Rect> = (0..4)
        .map(|i| Rect::from_min_size(Pos2::new(start + i as f32 * (size + GAP), band.center().y - size / 2.0), Vec2::splat(size)))
        .collect();
    let near = |p: Pos2| tiles.iter().position(|t| t.center().distance(p) <= size * DRAG_HIT_RADIUS);
    let along = |from: Pos2, to: Pos2| {
        let steps = ((from.distance(to) / (size * 0.1)).ceil() as usize).clamp(1, 512);
        let mut hit: Vec<usize> = Vec::new();
        for i in 0..=steps {
            if let Some(t) = near(from + (to - from) * (i as f32 / steps as f32)) {
                if hit.last() != Some(&t) {
                    hit.push(t);
                }
            }
        }
        hit
    };
    let extend = |swipe: &mut PlaySwipe, t: usize| {
        if let Some(i) = swipe.trail.iter().position(|x| *x == t) {
            swipe.trail.truncate(i + 1); // back over a tile: rewind to it
        } else if swipe.trail.last().is_none_or(|last| last.abs_diff(t) == 1) {
            swipe.trail.push(t);
        }
    };

    let pointer = response.interact_pointer_pos();
    let mut played = false;
    if response.drag_started() {
        swipe.trail.clear();
        let origin = ui_press_origin(&response).or(pointer);
        if let (Some(from), Some(to)) = (origin, pointer) {
            for t in along(from, to) {
                extend(swipe, t);
            }
        }
        swipe.from = pointer;
    } else if response.dragged() {
        if let (Some(from), Some(to)) = (swipe.from, pointer) {
            for t in along(from, to) {
                extend(swipe, t);
            }
            swipe.from = Some(to);
        }
    } else if response.clicked() {
        swipe.show_hint = true;
    }
    if response.drag_stopped() {
        played = swipe.trail == [0, 1, 2, 3];
        if !played {
            swipe.show_hint = true;
        }
        swipe.trail.clear();
        swipe.from = None;
    }

    let painter = ui.painter();
    painter.rect_filled(band, 0.0, PANEL);
    painter.line_segment([band.left_top(), band.right_top()], Stroke::new(2.0_f32, GOLD));
    painter.line_segment([band.left_bottom(), band.right_bottom()], Stroke::new(2.0_f32, GOLD));

    let complete = swipe.trail == [0, 1, 2, 3];
    let trail_color = if complete { GREEN } else { ACCENT };
    for pair in swipe.trail.windows(2) {
        painter.line_segment([tiles[pair[0]].center(), tiles[pair[1]].center()], Stroke::new(12.0_f32, trail_color.gamma_multiply(0.7)));
    }
    for (i, tile) in tiles.iter().enumerate() {
        let lit = swipe.trail.contains(&i);
        let (fill, edge, ink) = if lit {
            (trail_color.gamma_multiply(0.35), trail_color, Color32::WHITE)
        } else {
            (TILE, TILE_EDGE, TEXT)
        };
        painter.rect_filled(*tile, size * 0.18, fill);
        painter.rect_stroke(*tile, size * 0.18, Stroke::new(2.0_f32, edge), egui::StrokeKind::Inside);
        painter.text(tile.center(), Align2::CENTER_CENTER, WORD[i], FontId::proportional(size * 0.5), ink);
    }
    played
}

/// A square button with a drawn icon over a short label.
fn icon_button(ui: &mut egui::Ui, label: &str, icon: fn(&egui::Painter, Rect, Color32)) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(72.0, 60.0), Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    let painter = ui.painter();
    let color = if response.hovered() { TEXT } else { MUTED };
    painter.rect_filled(rect, 10.0, if response.hovered() { TILE_EDGE } else { TILE });
    let icon_rect = Rect::from_center_size(Pos2::new(rect.center().x, rect.min.y + 22.0), Vec2::splat(22.0));
    icon(painter, icon_rect, color);
    painter.text(Pos2::new(rect.center().x, rect.max.y - 10.0), Align2::CENTER_CENTER, label, FontId::proportional(11.0), color);
    response
}

fn draw_chart_icon(painter: &egui::Painter, r: Rect, color: Color32) {
    let stroke = Stroke::new(2.0_f32, color);
    painter.line_segment([r.left_bottom(), r.right_bottom()], stroke);
    painter.line_segment([r.left_bottom(), r.left_top()], stroke);
    let pts = vec![
        r.left_bottom() + Vec2::new(4.0, -5.0),
        r.left_bottom() + Vec2::new(9.0, -12.0),
        r.left_bottom() + Vec2::new(14.0, -8.0),
        r.left_bottom() + Vec2::new(20.0, -18.0),
    ];
    painter.add(egui::Shape::line(pts, stroke));
}

fn draw_copy_icon(painter: &egui::Painter, r: Rect, color: Color32) {
    let stroke = Stroke::new(1.8_f32, color);
    let back = Rect::from_min_size(r.min + Vec2::new(2.0, 2.0), Vec2::splat(13.0));
    let front = back.translate(Vec2::splat(6.0));
    painter.rect_stroke(back, 2.0, stroke, egui::StrokeKind::Middle);
    painter.rect_filled(front, 2.0, TILE);
    painter.rect_stroke(front, 2.0, stroke, egui::StrokeKind::Middle);
}

fn draw_tick_icon(painter: &egui::Painter, r: Rect, _color: Color32) {
    let c = r.center();
    let tick = vec![c + Vec2::new(-8.0, 0.0), c + Vec2::new(-2.5, 5.5), c + Vec2::new(8.0, -6.0)];
    painter.add(egui::Shape::line(tick, Stroke::new(2.5_f32, GREEN)));
}

/// A legend entry: a short line in the series colour, then its name in muted ink.
fn legend_key(ui: &mut egui::Ui, color: Color32, label: &str) {
    let (key, _) = ui.allocate_exact_size(Vec2::new(16.0, 10.0), Sense::hover());
    ui.painter().line_segment([key.left_center(), key.right_center()], Stroke::new(2.0_f32, color));
    ui.painter().circle(key.center(), 3.0, color, Stroke::NONE);
    ui.label(egui::RichText::new(label).size(11.0).color(MUTED));
}

/// Every league and the average it takes, with where the player stands: the
/// league they are in, how far off each one above is, and the rule for moving.
fn league_ladder(ui: &mut egui::Ui, league: usize, average: u32) {
    ui.label(egui::RichText::new("Leagues").size(14.0).color(TEXT).strong());
    ui.label(
        egui::RichText::new(format!(
            "Your league follows your average over your last {FORM_GAMES} games. Reach a league's average to move up \
             (after {MIN_GAMES_TO_MOVE} games); fall below your league's and you drop back one."
        ))
        .size(11.0)
        .color(MUTED),
    );
    ui.add_space(4.0);
    egui::Grid::new("league_ladder").num_columns(3).spacing([14.0, 4.0]).show(ui, |ui| {
        for (i, def) in LEAGUES.iter().enumerate() {
            ui.label(egui::RichText::new(def.name).size(13.0).color(league_color(i)).strong());
            ui.label(egui::RichText::new(format!("{} avg", thousands(def.threshold as usize))).size(12.0).color(TEXT));
            let (status, color) = match i {
                _ if i == league => ("You are here".to_string(), GOLD),
                _ if i < league => ("Passed".to_string(), MUTED),
                // Above the line already, but still short of the games to move.
                _ if average >= def.threshold => ("Average reached".to_string(), GREEN),
                _ => (format!("{} to go", thousands(def.threshold.saturating_sub(average) as usize)), MUTED),
            };
            ui.label(egui::RichText::new(status).size(12.0).color(color));
            ui.end_row();
        }
    });
}

/// "Rank: Gold", the league in its own colour.
fn rank_label(ui: &mut egui::Ui, league: usize, size: f32) {
    let mut job = egui::text::LayoutJob::default();
    job.append("Rank: ", 0.0, egui::TextFormat { font_id: FontId::proportional(size), color: MUTED, ..Default::default() });
    job.append(
        LEAGUES[league].name,
        0.0,
        egui::TextFormat { font_id: FontId::proportional(size), color: league_color(league), ..Default::default() },
    );
    ui.label(job);
}

/// What earns the letter bonus, with the stars that mark it.
fn bonus_key() -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    let font = FontId::proportional(11.0);
    let mut add = |text: &str, color: Color32| {
        job.append(text, 0.0, egui::TextFormat { font_id: font.clone(), color, ..Default::default() });
    };
    add("\u{2605}", GOLD);
    add(" +25 each letter used   ", MUTED);
    add("\u{2605}", GREEN);
    add(" +100 used again   all letters +500", MUTED);
    job.halign = egui::Align::Center;
    job
}

/// A titled group of label/value rows, for the stats page.
fn stat_block(ui: &mut egui::Ui, title: &str, rows: &[(&str, String)]) {
    ui.label(egui::RichText::new(title).size(14.0).color(TEXT).strong());
    for (label, value) in rows {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(format!("{label}:")).size(12.0).color(MUTED));
            ui.label(egui::RichText::new(value).size(12.0).color(TEXT).strong());
        });
    }
}

/// A chart scale topping out at or above `peak` in about four even, round steps.
fn nice_scale(peak: u32) -> (u32, u32) {
    let rough = (peak as f64 / 4.0).max(1.0);
    let magnitude = 10f64.powf(rough.log10().floor());
    let step = [1.0, 2.0, 2.5, 5.0, 10.0]
        .iter()
        .map(|m| m * magnitude)
        .find(|s| *s >= rough)
        .unwrap_or(10.0 * magnitude)
        .max(1.0) as u32;
    let top = peak.div_ceil(step) * step;
    (top, step)
}

/// `0`, `900`, `5k`, `2.5k`: axis labels that stay short.
fn short_number(n: u32) -> String {
    match n {
        0..=999 => n.to_string(),
        _ if n % 1000 == 0 => format!("{}k", n / 1000),
        _ => format!("{:.1}k", n as f32 / 1000.0),
    }
}

/// A button that looks like one and is easy to hit: filled, bold, and at least a
/// fingertip tall on a phone (44px, the usual touch-target minimum), a little
/// smaller on a desktop where a mouse is precise.
fn action_button(ui: &mut egui::Ui, label: &str, fill: Color32) -> egui::Response {
    let narrow = is_narrow(ui.ctx());
    let (height, size) = if narrow { (44.0, 15.0) } else { (36.0, 14.0) };
    // Sized to its label plus padding and laid out at exactly that size, so the
    // label sits in the middle: a button merely given a minimum width draws its
    // label at the left edge.
    let text_width = ui.fonts(|f| {
        f.layout_no_wrap(label.to_string(), FontId::proportional(size), TEXT).size().x
    });
    let width = (text_width + 40.0).max(96.0);
    ui.add_sized(
        [width, height],
        egui::Button::new(egui::RichText::new(label).size(size).color(Color32::WHITE).strong())
            .fill(fill)
            .stroke(Stroke::new(1.0_f32, fill.gamma_multiply(1.3)))
            .corner_radius(height / 2.0),
    )
    .on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A tab in a row of tabs: filled when selected, touch-sized on a phone.
fn tab_button(ui: &mut egui::Ui, label: &str, selected: bool) -> bool {
    let height = if is_narrow(ui.ctx()) { 40.0 } else { 30.0 };
    ui.add(
        egui::Button::new(egui::RichText::new(label).size(13.0).color(if selected { Color32::WHITE } else { TEXT }).strong())
            .fill(if selected { ACCENT } else { TILE })
            .corner_radius(8.0)
            .min_size(Vec2::new(0.0, height)),
    )
    .on_hover_cursor(egui::CursorIcon::PointingHand)
    .clicked()
}

fn big_button(ui: &mut egui::Ui, label: &str, color: Color32) -> bool {
    ui.add_sized(
        [240.0, 48.0],
        egui::Button::new(egui::RichText::new(label).size(17.0).color(Color32::WHITE).strong())
            .fill(color)
            .corner_radius(24.0),
    )
    .clicked()
}

/// The same pill as `big_button`, faded, for an action that is not available
/// yet. A default egui button here reads as plain grey text, not a button.
fn disabled_big_button(ui: &mut egui::Ui, label: &str) {
    ui.add_enabled_ui(false, |ui| big_button(ui, label, ACCENT));
}

/// Placeholder text for a field. It needs its colour set here: the app overrides
/// every text colour to `TEXT`, and a plain hint would inherit that and look like
/// something already typed in.
fn hint(text: &str) -> egui::RichText {
    egui::RichText::new(text).color(MUTED)
}

fn style(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = BG;
    visuals.window_fill = PANEL;
    visuals.override_text_color = Some(TEXT);
    ctx.set_visuals(visuals);
}

/// `133.4` -> `2:13`
fn clock_text(seconds: f32) -> String {
    let s = seconds.max(0.0).ceil() as u32;
    format!("{}:{:02}", s / 60, s % 60)
}

/// `370105` -> `370,105`
fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

fn capitalize(letters: &str) -> String {
    let mut chars = letters.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let mix = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t) as u8;
    Color32::from_rgb(mix(a.r(), b.r()), mix(a.g(), b.g()), mix(a.b(), b.b()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every non-ASCII character in the UI must exist in egui's bundled fonts,
    /// or it renders as a tofu box. (`\u{30fb}` did exactly that.)
    #[test]
    fn ui_glyphs_are_renderable() {
        let ctx = egui::Context::default();
        ctx.run(Default::default(), |_| {});

        let font = FontId::proportional(14.0);
        for glyph in ["\u{2022}", "\u{00b7}", "\u{2014}", "\u{2605}"] {
            assert!(
                ctx.fonts(|f| f.has_glyphs(&font, glyph)),
                "missing glyph {glyph:?} (U+{:04X})",
                glyph.chars().next().unwrap() as u32
            );
        }
        // The character that regressed, kept as a canary for the assertion itself.
        assert!(!ctx.fonts(|f| f.has_glyphs(&font, "\u{30fb}")));
    }

    /// Lay out an overlay headlessly at the real window size and report the card's
    /// rect. Two passes: an Area only knows its size after it has been shown once.
    fn overlay_rect_at(
        app: &mut WordLegendApp,
        draw: fn(&mut WordLegendApp, &egui::Context),
        size: Vec2,
    ) -> Rect {
        let ctx = egui::Context::default();
        let screen = Rect::from_min_size(Pos2::ZERO, size);
        for _ in 0..2 {
            let input = egui::RawInput { screen_rect: Some(screen), ..Default::default() };
            let _ = ctx.run(input, |ctx| draw(app, ctx));
        }
        let state = egui::AreaState::load(&ctx, egui::Id::new("overlay")).expect("overlay shown");
        Rect::from_min_size(state.left_top_pos(), state.size.expect("overlay sized"))
    }

    fn overlay_rect(app: &mut WordLegendApp, draw: fn(&mut WordLegendApp, &egui::Context)) -> Rect {
        overlay_rect_at(app, draw, Vec2::new(1000.0, 780.0))
    }

    /// A finished round showing a rank change, which is the tallest scorecard.
    fn app_after_round() -> WordLegendApp {
        let mut app = WordLegendApp::new();
        for _ in 0..crate::league::MIN_GAMES_TO_MOVE {
            app.game.ranking.record(9_000);
        }
        app.game.start_round();
        // A round with a score in it, so it is banked and can move the rank.
        app.game.score = 9_000;
        app.game.time_left = 0.0;
        app.game.tick(0.2);
        assert_eq!(app.game.phase, Phase::Over, "expected the round to have ended");
        app
    }

    // --- phones ----------------------------------------------------------------

    /// A Pixel held upright, a small Android phone, and a small tablet (which
    /// gets the desktop layout), in CSS pixels.
    const PHONES: [Vec2; 3] = [Vec2::new(411.0, 923.0), Vec2::new(360.0, 740.0), Vec2::new(700.0, 1000.0)];

    /// An app that never touches the network, signed in unless told otherwise.
    fn offline_app(signed_in: bool) -> WordLegendApp {
        let mut app = WordLegendApp::new();
        app.live = Live::new(net::Client::recording("https://server"));
        if signed_in {
            app.identity = Some(Identity { id: "0123456789ABCDEF".into(), name: "longest_name_16c".into() });
        }
        app
    }

    /// Run more frames on an existing context; the last frame's shapes.
    fn run_frames_on(app: &mut WordLegendApp, ctx: &egui::Context, size: Vec2, frames: usize) -> Vec<egui::Shape> {
        let screen = Rect::from_min_size(Pos2::ZERO, size);
        let mut shapes = Vec::new();
        for _ in 0..frames {
            let input = egui::RawInput { screen_rect: Some(screen), ..Default::default() };
            shapes = painted(ctx.run(input, |ctx| app.frame(ctx)).shapes);
        }
        shapes
    }

    /// Run whole frames at a screen size; the context and the last frame's shapes.
    fn run_frames(app: &mut WordLegendApp, size: Vec2, frames: usize) -> (egui::Context, Vec<egui::Shape>) {
        let ctx = egui::Context::default();
        let screen = Rect::from_min_size(Pos2::ZERO, size);
        let mut shapes = Vec::new();
        for _ in 0..frames {
            let input = egui::RawInput { screen_rect: Some(screen), ..Default::default() };
            shapes = painted(ctx.run(input, |ctx| app.frame(ctx)).shapes);
        }
        (ctx, shapes)
    }

    /// Anything drawn past either side of the screen: the screenshot's clipped
    /// clock and cut-off columns.
    fn off_screen(shapes: &[egui::Shape], size: Vec2) -> Vec<String> {
        shapes
            .iter()
            .filter_map(|shape| {
                let r = shape.visual_bounding_rect();
                let outside = r.is_finite() && (r.min.x < -0.5 || r.max.x > size.x + 0.5);
                outside.then(|| match shape {
                    egui::Shape::Text(t) => format!("text {:?} at {r:?}", t.galley.job.text),
                    other => format!("{:?} at {r:?}", std::mem::discriminant(other)),
                })
            })
            .collect()
    }

    fn assert_fits(what: &str, shapes: &[egui::Shape], size: Vec2) {
        let spilled = off_screen(shapes, size);
        assert!(spilled.is_empty(), "{what} at {size:?} draws off screen:\n{}", spilled.join("\n"));
    }

    #[test]
    fn every_screen_fits_a_phone() {
        for size in PHONES {
            let mut app = offline_app(false);
            let (_, shapes) = run_frames(&mut app, size, 4);
            assert_fits("signup", &shapes, size);

            let mut app = offline_app(false);
            app.signup.restoring = true;
            let (_, shapes) = run_frames(&mut app, size, 4);
            assert_fits("restore", &shapes, size);

            let mut app = offline_app(true);
            for score in [3_000, 5_000, 4_200] {
                app.game.ranking.record(score);
            }
            let (_, shapes) = run_frames(&mut app, size, 4);
            assert_fits("start card", &shapes, size);

            let mut app = offline_app(true);
            app.game.start_round();
            app.game.score = 123_400;
            let (ctx, shapes) = run_frames(&mut app, size, 4);
            assert_fits("playing", &shapes, size);
            let texts: Vec<String> = shapes
                .iter()
                .filter_map(|s| match s {
                    egui::Shape::Text(t) => Some(t.galley.job.text.clone()),
                    _ => None,
                })
                .collect();
            assert!(texts.iter().any(|t| t == "BEST"), "the clock row lost its BEST column at {size:?}");
            let hud = egui::containers::panel::PanelState::load(&ctx, egui::Id::new("hud")).expect("hud").rect;
            // Room for a touch-sized LEAVE button; the letter-wrapping bug made it 186px.
            assert!(hud.height() <= 130.0, "the heads-up display is {}px tall at {size:?}", hud.height());

            for shared in [false, true] {
                let mut app = offline_app(true);
                for _ in 0..crate::league::MIN_GAMES_TO_MOVE {
                    app.game.ranking.record(9_000);
                }
                app.game.start_round();
                app.game.score = 9_000;
                app.game.time_left = 0.0;
                app.game.tick(0.2);
                if shared {
                    app.game.round = Some(7);
                    let entries = (0..12)
                        .map(|i| net::Entry { name: format!("player_name_{i:02}"), score: 20_000 - i * 900, words: 30, finished: true, league: Some(i as usize % 6) })
                        .collect();
                    app.live.show_leaderboard(net::Leaderboard { round: 7, entries });
                }
                let (_, shapes) = run_frames(&mut app, size, 4);
                assert_fits(if shared { "shared results" } else { "solo results" }, &shapes, size);
            }
        }
    }

    #[test]
    fn a_round_with_nothing_found_is_shown_but_not_banked() {
        let mut app = offline_app(true);
        for score in [4_000, 6_000] {
            app.game.ranking.record(score);
        }
        let before = app.game.ranking.recent.clone();

        app.game.start_round();
        app.game.round = Some(7);
        app.game.time_left = 0.0;
        app.game.tick(0.2);
        assert_eq!(app.game.phase, Phase::Over);
        assert_eq!(app.game.ranking.recent, before, "a round with no words moved the rank average");
        assert!(app.game.rank_change.is_none());

        // Still on the leaderboard, as a zero.
        app.live.show_leaderboard(net::Leaderboard {
            round: 7,
            entries: vec![
                net::Entry { name: "someone".into(), score: 5_000, words: 12, finished: true, league: Some(1) },
                net::Entry { name: "longest_name_16c".into(), score: 0, words: 0, finished: true, league: None },
            ],
        });
        let (_, shapes) = run_frames(&mut app, Vec2::new(1000.0, 780.0), 4);
        let texts: Vec<String> = shapes
            .iter()
            .filter_map(|s| match s {
                egui::Shape::Text(t) => Some(t.galley.job.text.clone()),
                _ => None,
            })
            .collect();
        assert!(texts.iter().any(|t| t == "You didn't find any words this round."), "no notice: {texts:?}");
        assert!(texts.iter().any(|t| t == "you're #2 of 2"), "not placed on the table: {texts:?}");

        // A round with a score in it is banked as before.
        app.game.start_round();
        app.game.score = 2_000;
        app.game.time_left = 0.0;
        app.game.tick(0.2);
        assert_eq!(app.game.ranking.recent.len(), before.len() + 1);
    }

    /// Where a straight drag from one tile centre to another would pass, pushed
    /// sideways by `offset` pixels.
    fn offset_drag(g: &BoardGeometry, from: Position, to: Position, offset: f32) -> Vec<Position> {
        let (a, b) = (g.center(from), g.center(to));
        let normal = (b - a).normalized().rot90() * offset;
        g.tiles_along(a + normal, b + normal)
    }

    #[test]
    fn a_diagonal_drag_can_wander_without_picking_up_a_neighbour() {
        let g = geom();
        let (p, o) = (Position { row: 0, col: 0 }, Position { row: 1, col: 1 });
        // A quarter of a tile off the true diagonal, either side: well beyond the
        // few pixels the square hit-test allowed.
        for offset in [-0.25 * g.tile, 0.25 * g.tile] {
            let crossed = offset_drag(&g, p, o, offset);
            assert_eq!(crossed, vec![p, o], "{offset}px off the diagonal picked up {crossed:?}");
        }
    }

    #[test]
    fn a_straight_drag_off_centre_still_catches_every_tile() {
        let g = geom();
        let row: Vec<Position> = (0..SIZE).map(|col| Position { row: 1, col }).collect();
        for offset in [-0.3 * g.tile, 0.3 * g.tile] {
            let crossed = offset_drag(&g, row[0], row[SIZE - 1], offset);
            assert_eq!(crossed, row, "{offset}px off centre missed a tile");
        }
    }

    /// The league table made these cards much taller; they must still fit the window.
    #[test]
    fn overlay_cards_fit_the_window() {
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 780.0));

        let mut app = WordLegendApp::new();
        let ready = overlay_rect(&mut app, |a, ctx| a.overlay_ready(ctx));
        assert!(screen.contains_rect(ready), "start card overflows: {ready:?}");

        // A season-ending round shows the most content: board, stats, table and all.
        let mut app = app_after_round();
        let over = overlay_rect(&mut app, |a, ctx| a.overlay_results(ctx));
        assert!(screen.contains_rect(over), "results card overflows: {over:?}");
    }

    #[test]
    fn the_signup_card_fits_and_gates_play() {
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 780.0));
        let mut app = WordLegendApp::new();
        // With no stored account, the first-run screen must come up.
        assert!(app.needs_signup(), "a fresh install should ask for an account");

        let card = overlay_rect(&mut app, |a, ctx| a.overlay_signup(ctx));
        assert!(screen.contains_rect(card), "signup card overflows: {card:?}");

        // A code is minted once and held steady, not regenerated every frame.
        let first = app.signup.pending.clone().expect("a code should have been minted");
        overlay_rect(&mut app, |a, ctx| a.overlay_signup(ctx));
        assert_eq!(app.signup.pending.as_ref().unwrap().id, first.id, "the code changed under the player");

        // Naming the account is what finishes signup.
        let me = Identity { id: first.id, name: "wordfan".into() };
        app.identity = Some(me);
        assert!(!app.needs_signup());
    }

    /// The shared scorecard adds a leaderboard; with a long one it must still fit.
    #[test]
    fn the_shared_scorecard_with_a_full_leaderboard_fits() {
        let mut app = app_after_round();
        app.game.round = Some(7);
        let entries = (0..40)
            .map(|i| net::Entry { name: format!("player_{i:02}"), score: 20_000 - i * 400, words: 30, finished: true, league: Some(i as usize % 6) })
            .collect();
        app.live.show_leaderboard(net::Leaderboard { round: 7, entries });

        for size in [Vec2::new(1000.0, 780.0), Vec2::new(1000.0, 600.0)] {
            let screen = Rect::from_min_size(Pos2::ZERO, size);
            let over = overlay_rect_at(&mut app, |a, ctx| a.overlay_results(ctx), size);
            assert!(screen.contains_rect(over), "shared scorecard overflows {size:?}: {over:?}");
        }
    }

    /// Run one signup frame at the real window size with the given input events,
    /// and return what egui asked the platform to put on the clipboard.
    fn signup_frame(app: &mut WordLegendApp, ctx: &egui::Context, events: Vec<egui::Event>) -> Vec<String> {
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 780.0));
        let input = egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
        let output = ctx.run(input, |ctx| app.overlay_signup(ctx));
        output
            .platform_output
            .commands
            .into_iter()
            .filter_map(|c| match c {
                egui::OutputCommand::CopyText(text) => Some(text),
                _ => None,
            })
            .collect()
    }

    /// Every shape a frame painted, with nested lists flattened.
    fn painted(shapes: Vec<egui::epaint::ClippedShape>) -> Vec<egui::Shape> {
        fn flatten(shape: egui::Shape, out: &mut Vec<egui::Shape>) {
            match shape {
                egui::Shape::Vec(inner) => inner.into_iter().for_each(|s| flatten(s, out)),
                other => out.push(other),
            }
        }
        let mut out = Vec::new();
        for clipped in shapes {
            flatten(clipped.shape, &mut out);
        }
        out
    }

    /// The things a screenshot of the signup card caught: a sentence with a hole
    /// in it, a placeholder that looked typed in, and a disabled button that did
    /// not look like a button.
    #[test]
    fn signup_text_and_buttons_look_right() {
        for restoring in [false, true] {
            let mut app = WordLegendApp::new();
            app.signup.restoring = restoring;
            let ctx = egui::Context::default();
            let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 780.0));
            let mut shapes = Vec::new();
            for _ in 0..4 {
                let input = egui::RawInput { screen_rect: Some(screen), ..Default::default() };
                shapes = painted(ctx.run(input, |ctx| app.overlay_signup(ctx)).shapes);
            }

            let texts: Vec<(String, Vec<Color32>)> = shapes
                .iter()
                .filter_map(|s| match s {
                    egui::Shape::Text(t) => Some((
                        t.galley.job.text.clone(),
                        t.galley.job.sections.iter().map(|sec| sec.format.color).collect(),
                    )),
                    _ => None,
                })
                .collect();

            for (text, _) in &texts {
                assert!(!text.contains("   "), "a run of spaces in {text:?}");
            }

            let placeholders: &[&str] =
                if restoring { &["pick a name", "XXXX-XXXX-XXXX-XXXX"] } else { &["pick a name"] };
            for placeholder in placeholders {
                let (_, colors) = texts
                    .iter()
                    .find(|(t, _)| t == placeholder)
                    .unwrap_or_else(|| panic!("placeholder {placeholder:?} not drawn"));
                assert!(colors.iter().all(|c| *c == MUTED), "{placeholder:?} drawn as {colors:?}, not muted");
            }

            // The disabled button keeps the pill's full size.
            let pill = shapes.iter().any(|s| match s {
                egui::Shape::Rect(r) => (r.rect.size() - Vec2::new(240.0, 48.0)).length() < 1.0,
                _ => false,
            });
            assert!(pill, "the disabled button is not the usual pill (restoring: {restoring})");
        }
    }

    /// Lay out the rank panel on its own, as the start card shows it.
    fn form_panel_frame(app: &WordLegendApp, ctx: &egui::Context, events: Vec<egui::Event>) -> Vec<egui::Shape> {
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 200.0));
        let input = egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
        painted(
            ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.set_width(340.0);
                    app.form_panel(ui);
                });
            })
            .shapes,
        )
    }

    fn texts_of(shapes: &[egui::Shape]) -> Vec<String> {
        shapes
            .iter()
            .filter_map(|s| match s {
                egui::Shape::Text(t) => Some(t.galley.job.text.clone()),
                _ => None,
            })
            .collect()
    }

    fn lines_in(shapes: &[egui::Shape], color: Color32) -> Vec<Vec<Pos2>> {
        shapes
            .iter()
            .filter_map(|s| match s {
                egui::Shape::Path(p) if p.stroke.width == 2.0 && p.stroke.color == egui::epaint::ColorMode::Solid(color) => {
                    Some(p.points.clone())
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_rank_chart_draws_game_scores_and_the_average_as_two_lines() {
        let mut app = WordLegendApp::new();
        for (score, words) in [(1_000, 5), (4_000, 12), (2_500, 9), (6_000, 20), (3_000, 10), (5_500, 18), (7_000, 22)] {
            app.game.ranking.record_round(score, words, None);
        }
        let ctx = egui::Context::default();
        let shapes = form_panel_frame(&app, &ctx, vec![]);

        let scores = lines_in(&shapes, SERIES_SCORES);
        let averages = lines_in(&shapes, SERIES_AVERAGE);
        assert_eq!((scores.len(), averages.len()), (1, 1), "expected one line per series");
        assert_eq!(scores[0].len(), 7);
        assert_eq!(averages[0].len(), 7);

        // Newest on the right; the scores swing, the average moves less.
        assert!(scores[0].windows(2).all(|w| w[1].x > w[0].x), "games out of order");
        assert!(scores[0][6].y < scores[0][0].y, "7,000 should plot above 1,000");
        let spread = |line: &[Pos2]| line.iter().map(|p| p.y).fold(f32::MIN, f32::max) - line.iter().map(|p| p.y).fold(f32::MAX, f32::min);
        assert!(spread(&averages[0]) < spread(&scores[0]), "the average should swing less than the games");

        // The first game's average is its own score: the two lines start together.
        assert!((averages[0][0].y - scores[0][0].y).abs() < 0.5, "the first game's average is not its score");

        // Two series, so a legend naming both.
        let texts = texts_of(&shapes);
        assert!(texts.iter().any(|t| t == "Game scores") && texts.iter().any(|t| t == "Average"), "no legend: {texts:?}");
    }

    #[test]
    fn hovering_the_chart_reads_off_a_game() {
        let mut app = WordLegendApp::new();
        for (score, words) in [(2_000, 7), (9_000, 31), (3_000, 11)] {
            app.game.ranking.record_round(score, words, None);
        }
        let ctx = egui::Context::default();
        let shapes = form_panel_frame(&app, &ctx, vec![]);
        let line = lines_in(&shapes, SERIES_SCORES).pop().expect("a line");

        // Aim near, not on, the middle game: the nearest one should be picked.
        let aim = line[1] + Vec2::new(6.0, 12.0);
        let mut texts = Vec::new();
        for _ in 0..4 {
            texts = texts_of(&form_panel_frame(&app, &ctx, vec![egui::Event::PointerMoved(aim)]));
        }
        assert!(texts.iter().any(|t| t == "1 game ago"), "no readout for the hovered game: {texts:?}");
        assert!(texts.iter().any(|t| t == "Score 9,000"), "hovered game's score missing: {texts:?}");
        assert!(texts.iter().any(|t| t == "31 words"), "hovered game's words missing: {texts:?}");
    }

    #[test]
    fn an_empty_record_says_where_the_line_will_go() {
        let app = WordLegendApp::new();
        let ctx = egui::Context::default();
        let shapes = form_panel_frame(&app, &ctx, vec![]);
        assert!(texts_of(&shapes).iter().any(|t| t == "Your rounds will chart here"));
        assert!(lines_in(&shapes, SERIES_SCORES).is_empty());
    }

    #[test]
    fn the_stats_page_shows_the_chart_best_game_averages_and_totals() {
        for size in [Vec2::new(1000.0, 780.0), PHONES[0], PHONES[1]] {
            let mut app = offline_app(true);
            for (score, words, best) in [(3_000, 12, ("otters", 1_400)), (8_400, 30, ("quizzers", 2_200)), (5_000, 20, ("hat", 100))] {
                app.game.ranking.record_round(score, words, Some(best));
                app.game.ranking.best_score = app.game.ranking.best_score.max(score);
            }
            app.show_stats = true;
            // Enough frames for the card's fade-in to finish, so colours are exact.
            let (ctx, top) = run_frames(&mut app, size, 12);
            assert_fits("stats page", &top, size);
            // A phone's page scrolls: read the bottom of it too.
            let screen = Rect::from_min_size(Pos2::ZERO, size);
            let mut bottom = Vec::new();
            for events in [
                vec![egui::Event::PointerMoved(screen.center())],
                vec![egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: Vec2::new(0.0, -3000.0), modifiers: Default::default() }],
            ]
            .into_iter()
            .chain(std::iter::repeat_with(Vec::new).take(30))
            {
                let input = egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
                bottom = painted(ctx.run(input, |ctx| app.frame(ctx)).shapes);
            }
            let shapes: Vec<egui::Shape> = top.iter().cloned().chain(bottom).collect();
            let texts = texts_of(&shapes);
            for expected in ["Best Game", "All-time Averages", "Totals", "quizzers (2200 points)", "8,400", "5,466", "16,400", "62"] {
                assert!(texts.iter().any(|t| t == expected), "stats page at {size:?} is missing {expected:?}: {texts:?}");
            }
            assert_eq!(lines_in(&top, SERIES_SCORES).len(), 1, "no chart on the stats page at {size:?}");
            for label in ["Newest", "Oldest"] {
                assert!(texts.iter().any(|t| t == label), "no {label} axis label at {size:?}");
            }
            // The leagues and the average each takes, with where the player is.
            for expected in ["Leagues", "Bronze", "Silver", "4,000 avg", "Hero", "30,000 avg", "You are here", "Average reached"] {
                assert!(texts.iter().any(|t| t == expected), "the league ladder at {size:?} is missing {expected:?}");
            }
        }
    }

    #[test]
    fn the_home_screen_welcomes_the_player_and_play_joins() {
        for size in [Vec2::new(1000.0, 780.0), PHONES[0], PHONES[1]] {
            let mut app = offline_app(true);
            let ctx = egui::Context::default();
            let screen = Rect::from_min_size(Pos2::ZERO, size);
            let frame = |app: &mut WordLegendApp, events: Vec<egui::Event>| {
                let input = egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
                painted(ctx.run(input, |ctx| app.frame(ctx)).shapes)
            };
            let mut shapes = Vec::new();
            // Enough frames for the card to finish fading in, so colours are exact.
            for _ in 0..12 {
                shapes = frame(&mut app, vec![]);
            }
            assert_fits("home screen", &shapes, size);
            let texts = texts_of(&shapes);
            for expected in ["Welcome,", "longest_name_16c", "Stats", "My code"] {
                assert!(texts.iter().any(|t| t == expected), "home at {size:?} is missing {expected:?}");
            }
            // The logo and the PLAY band are tiles, letter by letter.
            for letter in ["W", "O", "R", "D", "L", "E", "G", "N", "P", "A", "Y"] {
                assert!(texts.iter().any(|t| t == letter), "no {letter} tile at {size:?}");
            }

            // The logo is black and gold.
            let gold_letters = shapes
                .iter()
                .filter(|s| matches!(s, egui::Shape::Text(t) if t.galley.job.text == "W" && t.galley.job.sections[0].format.color == GOLD))
                .count();
            assert_eq!(gold_letters, 1, "the W of the logo is not gold at {size:?}");
            assert!(
                shapes.iter().any(|s| matches!(s, egui::Shape::Rect(r) if r.fill == LOGO_TILE)),
                "no black logo tiles at {size:?}"
            );

            let centre_of = |shapes: &[egui::Shape], letter: &str| {
                shapes
                    .iter()
                    .filter_map(|s| match s {
                        egui::Shape::Text(t) if t.galley.job.text == letter => Some(s.visual_bounding_rect().center()),
                        _ => None,
                    })
                    .last()
                    .unwrap_or_else(|| panic!("no {letter} tile"))
            };
            let (p, l, a, y) = (centre_of(&shapes, "P"), centre_of(&shapes, "L"), centre_of(&shapes, "A"), centre_of(&shapes, "Y"));
            let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };

            // A tap on PLAY starts nothing; it explains the swipe instead.
            frame(&mut app, vec![egui::Event::PointerMoved(y), button(y, true)]);
            frame(&mut app, vec![button(y, false)]);
            let shapes = frame(&mut app, vec![]);
            assert!(!app.live.joined(), "a tap started the game at {size:?}");
            assert!(texts_of(&shapes).iter().any(|t| t.starts_with("Swipe across the letters")), "no hint after a tap");

            // P-L-A and let go: not PLAY, so nothing.
            frame(&mut app, vec![egui::Event::PointerMoved(p), button(p, true)]);
            frame(&mut app, vec![egui::Event::PointerMoved(l)]);
            frame(&mut app, vec![egui::Event::PointerMoved(a)]);
            frame(&mut app, vec![button(a, false)]);
            assert!(!app.live.joined(), "a partial swipe started the game at {size:?}");

            // Y-A-L-P backwards is not PLAY either.
            frame(&mut app, vec![egui::Event::PointerMoved(y), button(y, true)]);
            frame(&mut app, vec![egui::Event::PointerMoved(p)]);
            frame(&mut app, vec![button(p, false)]);
            assert!(!app.live.joined(), "a backwards swipe started the game at {size:?}");

            // P-L-A-Y, lit up along the way, and let go: that plays.
            frame(&mut app, vec![egui::Event::PointerMoved(p), button(p, true)]);
            frame(&mut app, vec![egui::Event::PointerMoved(l)]);
            frame(&mut app, vec![egui::Event::PointerMoved(a)]);
            let lit = frame(&mut app, vec![egui::Event::PointerMoved(y)]);
            assert_eq!(app.play_swipe.trail, vec![0, 1, 2, 3], "the swipe did not trace PLAY at {size:?}");
            assert!(
                lit.iter().any(|s| matches!(s, egui::Shape::LineSegment { stroke, .. } if stroke.width == 12.0)),
                "no trail drawn under the swipe at {size:?}"
            );
            frame(&mut app, vec![button(y, false)]);
            assert!(app.live.joined(), "swiping PLAY at {size:?} did not join");
        }
    }

    #[test]
    fn the_phone_scorecard_fits_one_screen_without_scrolling_the_page() {
        for size in PHONES.iter().take(2).copied() {
            let mut app = offline_app(true);
            for _ in 0..crate::league::MIN_GAMES_TO_MOVE {
                app.game.ranking.record(9_000);
            }
            app.game.start_round();
            app.game.round = Some(7);
            app.game.score = 9_000;
            app.game.time_left = 0.0;
            app.game.tick(0.2);
            let entries = (0..30)
                .map(|i| net::Entry { name: format!("player_name_{i:02}"), score: 20_000 - i * 500, words: 30, finished: i % 2 == 0, league: Some(3) })
                .collect();
            app.live.show_leaderboard(net::Leaderboard { round: 7, entries });

            let (_, shapes) = run_frames(&mut app, size, 4);
            assert_fits("phone scorecard", &shapes, size);
            let texts = texts_of(&shapes);

            // The countdown is pinned on screen, not below a fold.
            let countdown = shapes
                .iter()
                .find(|s| matches!(s, egui::Shape::Text(t) if t.galley.job.text.starts_with("Next round in")))
                .map(|s| s.visual_bounding_rect())
                .expect("no countdown");
            assert!(countdown.max.y <= size.y, "the countdown is off screen at {size:?}: {countdown:?}");

            // Opens on the leaderboard, with the word lists a tab away.
            assert!(texts.iter().any(|t| t == "Players (30)"), "no players tab: {texts:?}");
            assert!(texts.iter().any(|t| t == "Words"), "no words tab");
            assert!(texts.iter().any(|t| t == "player_name_00"), "the leaderboard is not the open tab");
            assert!(texts.iter().any(|t| t == "finishing…"), "a player still finishing is not shown as such");
            assert!(texts.iter().any(|t| t == "9,000"), "the score is not beside the board");
        }
    }

    /// The word lists are three columns side by side, and each scrolls on its own.
    #[test]
    fn each_word_list_is_its_own_scrolling_column() {
        for size in [PHONES[0], PHONES[1], Vec2::new(1000.0, 780.0)] {
            let mut app = offline_app(true);
            app.game.start_round();
            app.game.round = Some(7);
            app.game.score = 9_000;
            app.game.time_left = 0.0;
            app.game.tick(0.2);
            app.results_tab = ResultsTab::Words;
            // Plenty in every list, so each has something to scroll.
            app.game.words.common = (0..80).map(|i| format!("common{i:02}")).collect();
            app.game.words.obscure = (0..80).map(|i| format!("obscure{i:02}")).collect();
            app.game.theme = Some("Animals".into());
            app.game.words.theme = (0..80).map(|i| format!("animal{i:02}")).collect();

            let ctx = egui::Context::default();
            let screen = Rect::from_min_size(Pos2::ZERO, size);
            let frame = |app: &mut WordLegendApp, events: Vec<egui::Event>| {
                let input = egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
                painted(ctx.run(input, |ctx| app.frame(ctx)).shapes)
            };
            let mut shapes = Vec::new();
            for _ in 0..12 {
                shapes = frame(&mut app, vec![]);
            }
            assert_fits("word columns", &shapes, size);

            let find = |shapes: &[egui::Shape], text: &str| {
                shapes.iter().find_map(|s| match s {
                    egui::Shape::Text(t) if t.galley.job.text == text => Some(s.visual_bounding_rect()),
                    _ => None,
                })
            };
            let headers: Vec<Rect> = ["Common (0/80)", "Obscure (0/80)", "Animals (0/80)"]
                .iter()
                .map(|h| find(&shapes, h).unwrap_or_else(|| panic!("no {h} header at {size:?}: {:?}", texts_of(&shapes))))
                .collect();
            assert!(
                headers.windows(2).all(|w| w[1].min.x > w[0].max.x && (w[1].min.y - w[0].min.y).abs() < 2.0),
                "the lists are not side by side at {size:?}: {headers:?}"
            );

            // Scroll over the Common column only.
            let common_first = find(&shapes, "common00").expect("common00 drawn");
            let obscure_first = find(&shapes, "obscure00").expect("obscure00 drawn");
            let over_common = common_first.center() + Vec2::new(0.0, 40.0);
            frame(&mut app, vec![egui::Event::PointerMoved(over_common)]);
            frame(
                &mut app,
                vec![egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: Vec2::new(0.0, -300.0), modifiers: Default::default() }],
            );
            for _ in 0..30 {
                shapes = frame(&mut app, vec![]);
            }
            let common_after = find(&shapes, "common00").map(|r| r.min.y).unwrap_or(f32::MIN);
            let obscure_after = find(&shapes, "obscure00").expect("obscure00 still drawn").min.y;
            assert!(common_after < common_first.min.y - 50.0, "the Common column did not scroll at {size:?}");
            assert!((obscure_after - obscure_first.min.y).abs() < 1.0, "scrolling Common moved Obscure too at {size:?}");
        }
    }

    /// Tapping a letter on the scorecard's board lists the words through it, and
    /// tapping one of those draws its path.
    #[test]
    fn tapping_a_scorecard_letter_lists_its_words_and_shows_their_paths() {
        for size in [PHONES[0], Vec2::new(1000.0, 780.0)] {
            let mut app = offline_app(true);
            app.game.start_round();
            app.game.grid = std::array::from_fn(|r| {
                std::array::from_fn(|c| crate::game::Cell { letters: ["c", "a", "t", "s", "h", "e", "r", "o", "m", "i", "n", "d", "p", "l", "u", "g"][r * SIZE + c] })
            });
            app.game.words = crate::game::solve_board(&app.game.dictionary, &app.game.grid);
            app.game.score = 100;
            app.game.time_left = 0.0;
            app.game.tick(0.2);

            let ctx = egui::Context::default();
            let screen = Rect::from_min_size(Pos2::ZERO, size);
            let frame = |app: &mut WordLegendApp, events: Vec<egui::Event>| {
                let input = egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
                painted(ctx.run(input, |ctx| app.frame(ctx)).shapes)
            };
            let mut shapes = Vec::new();
            for _ in 0..12 {
                shapes = frame(&mut app, vec![]);
            }

            // The scorecard's small board: its tiles are the small TILE squares.
            let mut small: Vec<Rect> = shapes
                .iter()
                .filter_map(|s| match s {
                    egui::Shape::Rect(r) if r.fill == TILE && r.rect.width() < 60.0 && (r.rect.width() - r.rect.height()).abs() < 1.0 => Some(r.rect),
                    _ => None,
                })
                .collect();
            small.sort_by(|a, b| (a.min.y, a.min.x).partial_cmp(&(b.min.y, b.min.x)).unwrap());
            assert_eq!(small.len(), 16, "expected the scorecard board at {size:?}");
            let a_tile = small[1].center(); // the A in CATS

            let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
            frame(&mut app, vec![egui::Event::PointerMoved(a_tile), button(a_tile, true)]);
            frame(&mut app, vec![button(a_tile, false)]);
            for _ in 0..3 {
                shapes = frame(&mut app, vec![]);
            }
            let focus = app.letter_focus.as_ref().expect("tapping a letter did nothing");
            assert_eq!(focus.at, Position { row: 0, col: 1 });
            let texts = texts_of(&shapes);
            assert!(texts.iter().any(|t| t.starts_with("Words through A")), "no heading for the letter at {size:?}");
            assert!(focus.words.iter().any(|(w, _)| w == "cats"), "CATS is not among the words through A");
            // Longest first, so the first entry is the one on screen to tap.
            let (first, _) = focus.words[0].clone();
            let label = format!("{first}  {}", app.game.word_score(&first));
            assert!(texts.iter().any(|t| *t == label), "{label:?} is not listed at {size:?}");
            assert_fits("letter words", &shapes, size);

            // Tap it: its path is drawn over the small board.
            let cats = shapes
                .iter()
                .find_map(|s| match s {
                    egui::Shape::Text(t) if t.galley.job.text == label => Some(s.visual_bounding_rect().center()),
                    _ => None,
                })
                .unwrap();
            let before = shapes.iter().filter(|s| matches!(s, egui::Shape::LineSegment { .. })).count();
            frame(&mut app, vec![egui::Event::PointerMoved(cats), button(cats, true)]);
            frame(&mut app, vec![button(cats, false)]);
            shapes = frame(&mut app, vec![]);
            let shown = app.shown_word.as_ref().map(|(w, _)| w.clone());
            assert_eq!(shown.as_deref(), Some(first.as_str()), "tapping the word did not pick it at {size:?}");
            let after = shapes.iter().filter(|s| matches!(s, egui::Shape::LineSegment { .. })).count();
            assert!(after >= before + first.len() - 1, "{first}'s path was not drawn at {size:?}");

            // Drawn over the letters, not under them: every path segment is painted
            // after the last letter on the small board.
            let last_letter = shapes
                .iter()
                .rposition(|s| matches!(s, egui::Shape::Text(t) if t.galley.job.text == "G" && s.visual_bounding_rect().width() < 30.0))
                .expect("the small board's letters");
            let path_width = small[0].width() * 4.0 * 0.035;
            let segments: Vec<usize> = shapes
                .iter()
                .enumerate()
                .filter(|(_, s)| matches!(s, egui::Shape::LineSegment { stroke, .. } if (stroke.width - path_width).abs() < 1.5))
                .map(|(i, _)| i)
                .collect();
            assert!(!segments.is_empty(), "no path segments found at {size:?}");
            assert!(segments.iter().all(|i| *i > last_letter), "the path is drawn under the letters at {size:?}");

            // Tapping the letter again goes back to the lists.
            frame(&mut app, vec![egui::Event::PointerMoved(a_tile), button(a_tile, true)]);
            frame(&mut app, vec![button(a_tile, false)]);
            assert!(app.letter_focus.is_none(), "the same letter did not close its list at {size:?}");
        }
    }

    /// Any word in the scorecard's lists can be tapped to draw its path.
    #[test]
    fn tapping_a_word_in_the_lists_shows_its_path() {
        for size in [PHONES[0], Vec2::new(1000.0, 780.0)] {
            let mut app = offline_app(true);
            app.game.start_round();
            app.game.grid = std::array::from_fn(|r| {
                std::array::from_fn(|c| crate::game::Cell { letters: ["c", "a", "t", "s", "h", "e", "r", "o", "m", "i", "n", "d", "p", "l", "u", "g"][r * SIZE + c] })
            });
            app.game.words = crate::game::solve_board(&app.game.dictionary, &app.game.grid);
            app.game.score = 100;
            app.game.time_left = 0.0;
            app.game.tick(0.2);
            app.results_tab = ResultsTab::Words;

            let ctx = egui::Context::default();
            let screen = Rect::from_min_size(Pos2::ZERO, size);
            let frame = |app: &mut WordLegendApp, events: Vec<egui::Event>| {
                let input = egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
                painted(ctx.run(input, |ctx| app.frame(ctx)).shapes)
            };
            let mut shapes = Vec::new();
            for _ in 0..12 {
                shapes = frame(&mut app, vec![]);
            }
            let first = app.game.words.common[0].clone();
            let at = shapes
                .iter()
                .find_map(|s| match s {
                    egui::Shape::Text(t) if t.galley.job.text == first => Some(s.visual_bounding_rect().center()),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("{first} is not in the Common column at {size:?}"));
            let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
            frame(&mut app, vec![egui::Event::PointerMoved(at), button(at, true)]);
            frame(&mut app, vec![button(at, false)]);
            let shapes = frame(&mut app, vec![]);

            let (word, path) = app.shown_word.clone().unwrap_or_else(|| panic!("tapping {first} showed nothing at {size:?}"));
            assert_eq!(word, first);
            let spelled: String = path.iter().map(|p| app.game.grid[p.row][p.col].letters).collect();
            assert_eq!(spelled, first, "the path does not spell the word");
            let segments = shapes.iter().filter(|s| matches!(s, egui::Shape::LineSegment { .. })).count();
            assert!(segments >= path.len() - 1, "no path drawn for {first} at {size:?}");

            // Tapping it again clears the path.
            frame(&mut app, vec![egui::Event::PointerMoved(at), button(at, true)]);
            frame(&mut app, vec![button(at, false)]);
            assert!(app.shown_word.is_none(), "tapping the word again did not clear its path at {size:?}");
        }
    }

    fn click(app: &mut WordLegendApp, ctx: &egui::Context, screen: Rect, at: Pos2) {
        let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        for events in [vec![egui::Event::PointerMoved(at), button(true)], vec![button(false)], vec![]] {
            let input = egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
            let _ = ctx.run(input, |ctx| app.frame(ctx));
        }
    }

    fn text_centre(shapes: &[egui::Shape], text: &str) -> Option<Pos2> {
        shapes.iter().rev().find_map(|s| match s {
            egui::Shape::Text(t) if t.galley.job.text == text => Some(s.visual_bounding_rect().center()),
            _ => None,
        })
    }

    #[test]
    fn leaving_a_round_asks_first_then_banks_the_score_and_goes_home() {
        for size in [PHONES[0], Vec2::new(1000.0, 780.0)] {
            let mut app = offline_app(true);
            app.game.start_round();
            app.game.theme = Some("Halloween".into());
            app.game.score = 2_400;
            let games = app.game.ranking.lifetime.games;
            let (ctx, shapes) = run_frames(&mut app, size, 6);
            let screen = Rect::from_min_size(Pos2::ZERO, size);
            let texts = texts_of(&shapes);
            assert!(texts.iter().any(|t| t == "Theme: Halloween"), "no theme under the board at {size:?}");
            assert!(texts.iter().any(|t| t.starts_with("Letter bonus +")), "no letter bonus under the board at {size:?}");
            assert_fits("playing", &shapes, size);

            // The bar says the rank plainly, and has LEAVE in it.
            assert!(texts.iter().any(|t| t == "Rank: Bronze"), "the bar does not say Rank: Bronze at {size:?}");
            assert!(!texts.iter().any(|t| t.ends_with(" avg") && t.starts_with("Rank")), "the bar still shows the average");
            let bar_leave = text_centre(&shapes, "LEAVE").expect("a LEAVE button in the bar");
            assert!(bar_leave.y < 130.0, "LEAVE is not in the bar at the top at {size:?}: {bar_leave:?}");
            assert!(bar_leave.x < size.x / 3.0, "LEAVE is not at the left of the bar at {size:?}: {bar_leave:?}");
            let leaves = texts.iter().filter(|t| t.eq_ignore_ascii_case("leave") || *t == "Leave round").count();
            assert_eq!(leaves, 1, "there should be exactly one way to leave on screen at {size:?}");

            // Its label is centred on the button.
            let button = shapes
                .iter()
                .filter_map(|s| match s {
                    egui::Shape::Rect(r) if r.fill == RED && r.rect.contains(bar_leave) => Some(r.rect),
                    _ => None,
                })
                .next()
                .expect("LEAVE is not a filled button");
            assert!(
                (button.center().x - bar_leave.x).abs() < 2.0 && (button.center().y - bar_leave.y).abs() < 2.0,
                "LEAVE's label is off centre at {size:?}: label {bar_leave:?}, button {button:?}"
            );

            // Leave round: a confirmation first, nothing banked yet.
            click(&mut app, &ctx, screen, bar_leave);
            let shapes = run_frames_on(&mut app, &ctx, size, 4);
            let texts = texts_of(&shapes);
            assert!(texts.iter().any(|t| t == "Leave this round?"), "no confirmation at {size:?}");
            assert!(texts.iter().any(|t| t == "You'll leave with 2,400 points."), "the confirmation does not say what is kept");
            assert_eq!(app.game.phase, Phase::Playing);

            // Keep playing: back to the round.
            click(&mut app, &ctx, screen, text_centre(&shapes, "KEEP PLAYING").unwrap());
            assert!(!app.confirm_leave && app.game.phase == Phase::Playing, "Keep playing did not return to the round");

            // Leave for real: banked, and home.
            let shapes = run_frames_on(&mut app, &ctx, size, 2);
            click(&mut app, &ctx, screen, text_centre(&shapes, "LEAVE").unwrap());
            let shapes = run_frames_on(&mut app, &ctx, size, 4);
            click(&mut app, &ctx, screen, text_centre(&shapes, "LEAVE ROUND").unwrap());
            assert_eq!(app.game.phase, Phase::Ready, "leaving did not go home at {size:?}");
            assert_eq!(app.game.ranking.lifetime.games, games + 1, "leaving did not bank the score");
        }
    }

    /// Buttons are filled and, on a phone, at least a fingertip tall.
    #[test]
    fn buttons_are_touch_sized_on_a_phone() {
        let size = PHONES[1];
        let mut app = offline_app(true);
        app.game.start_round();
        app.game.score = 1_000;
        app.game.time_left = 0.0;
        app.game.tick(0.2);
        app.game.round = Some(7);
        let (_, shapes) = run_frames(&mut app, size, 8);
        let label = text_centre(&shapes, "Back to home").expect("a Back to home button");
        let button = shapes
            .iter()
            .filter_map(|s| match s {
                egui::Shape::Rect(r) if r.fill == ACCENT && r.rect.contains(label) => Some(r.rect),
                _ => None,
            })
            .next()
            .expect("Back to home is not a filled button");
        assert!(button.height() >= 44.0, "Back to home is only {}px tall on a phone", button.height());
        for tab in ["Players (0)", "Words"] {
            let at = text_centre(&shapes, tab).unwrap_or_else(|| panic!("no {tab} tab"));
            let rect = shapes
                .iter()
                .filter_map(|s| match s {
                    egui::Shape::Rect(r) if (r.fill == ACCENT || r.fill == TILE) && r.rect.contains(at) && r.rect.width() < 250.0 => Some(r.rect),
                    _ => None,
                })
                .next()
                .unwrap_or_else(|| panic!("the {tab} tab is not a filled button"));
            assert!(rect.height() >= 40.0, "the {tab} tab is only {}px tall", rect.height());
        }
    }

    #[test]
    fn the_leaderboard_shows_each_players_league() {
        let mut app = offline_app(true);
        app.game.start_round();
        app.game.score = 1_000;
        app.game.time_left = 0.0;
        app.game.tick(0.2);
        app.game.round = Some(7);
        app.live.show_leaderboard(net::Leaderboard {
            round: 7,
            entries: vec![
                net::Entry { name: "ace".into(), score: 30_000, words: 60, finished: true, league: Some(5) },
                net::Entry { name: "newbie".into(), score: 900, words: 4, finished: true, league: Some(0) },
                net::Entry { name: "oldclient".into(), score: 800, words: 3, finished: true, league: None },
            ],
        });
        for size in [PHONES[0], Vec2::new(1000.0, 780.0)] {
            let (_, shapes) = run_frames(&mut app, size, 8);
            let texts = texts_of(&shapes);
            for (name, league) in [("ace", "Hero"), ("newbie", "Bronze")] {
                let row_y = text_centre(&shapes, name).unwrap_or_else(|| panic!("{name} missing at {size:?}")).y;
                let tag = shapes.iter().any(|s| matches!(s, egui::Shape::Text(t) if t.galley.job.text == league && (s.visual_bounding_rect().center().y - row_y).abs() < 6.0));
                assert!(tag, "{name}'s league {league} is not on their row at {size:?}: {texts:?}");
            }
        }
    }

    #[test]
    fn a_scorecard_counts_down_to_the_next_round_and_has_a_way_home() {
        for size in [PHONES[0], Vec2::new(1000.0, 780.0)] {
            for shared in [false, true] {
                let mut app = offline_app(true);
                app.game.start_round();
                app.game.score = 1_000;
                app.game.time_left = 0.0;
                app.game.tick(0.2);
                if shared {
                    app.game.round = Some(7);
                }
                app.live.play_now(&mut app.game, 0.0);
                let (ctx, shapes) = run_frames(&mut app, size, 8);
                let texts = texts_of(&shapes);
                assert!(texts.iter().any(|t| t.starts_with("Next round in ")), "no countdown (shared {shared}) at {size:?}");
                assert!(!texts.iter().any(|t| t == "NEXT ROUND"), "a button to press on is still there at {size:?}");
                assert_fits("scorecard", &shapes, size);

                let screen = Rect::from_min_size(Pos2::ZERO, size);
                click(&mut app, &ctx, screen, text_centre(&shapes, "Back to home").expect("a way home"));
                assert_eq!(app.game.phase, Phase::Ready, "Back to home did not go home (shared {shared}) at {size:?}");
                assert!(!app.live.joined(), "going home should leave the cycle of rounds");
            }
        }
    }

    /// On a phone every dialog is a whole screen, not a card over the board.
    #[test]
    fn every_dialog_is_full_screen_on_a_phone() {
        for size in PHONES.iter().take(2).copied() {
            let screens: [(&str, fn(&mut WordLegendApp)); 5] = [
                ("signup", |app| app.identity = None),
                ("restore", |app| {
                    app.identity = None;
                    app.signup.restoring = true;
                }),
                ("home", |_| {}),
                ("stats", |app| app.show_stats = true),
                ("results", |app| {
                    app.game.start_round();
                    app.game.score = 1_000;
                    app.game.time_left = 0.0;
                    app.game.tick(0.2);
                }),
            ];
            for (name, set_up) in screens {
                let mut app = offline_app(true);
                set_up(&mut app);
                let (ctx, _) = run_frames(&mut app, size, 4);
                let page = egui::AreaState::load(&ctx, egui::Id::new("page"));
                assert!(page.is_some(), "{name} at {size:?} is not a full-screen page");
                assert!(
                    egui::AreaState::load(&ctx, egui::Id::new("overlay")).is_none(),
                    "{name} at {size:?} is still a card"
                );
            }
        }
    }

    #[test]
    fn the_phone_board_is_nearly_edge_to_edge_and_diagonals_stay_easy() {
        let size = PHONES[0];
        let mut app = offline_app(true);
        app.game.start_round();
        let (_, shapes) = run_frames(&mut app, size, 4);
        let tiles: Vec<Rect> = shapes
            .iter()
            .filter_map(|s| match s {
                egui::Shape::Rect(r) if r.fill == TILE => Some(r.rect),
                _ => None,
            })
            .collect();
        let span = tiles.iter().map(|r| r.max.x).fold(f32::MIN, f32::max) - tiles.iter().map(|r| r.min.x).fold(f32::MAX, f32::min);
        assert!(span >= size.x * 0.9, "the board spans only {span}px of {}px", size.x);

        // With the phone's tighter gaps, a diagonal still wanders a quarter tile
        // off line without touching a neighbour.
        let g = BoardGeometry::with_gap(Pos2::ZERO, size.x - 8.0, PHONE_GAP);
        let (p, o) = (Position { row: 1, col: 1 }, Position { row: 2, col: 2 });
        for offset in [-0.25 * g.tile, 0.25 * g.tile] {
            assert_eq!(offset_drag(&g, p, o, offset), vec![p, o], "{offset}px off the diagonal picked up a neighbour");
        }
    }

    #[test]
    fn used_letters_wear_a_yellow_star_and_reused_ones_a_green_one_too() {
        let mut app = offline_app(true);
        app.game.start_round();
        app.game.tile_uses = [[0; SIZE]; SIZE];
        app.game.tile_uses[0][0] = 1;
        app.game.tile_uses[2][3] = 2;
        app.game.tile_uses[3][1] = 5;
        let (_, shapes) = run_frames(&mut app, Vec2::new(1000.0, 780.0), 4);

        let stars: Vec<(Color32, Pos2)> = shapes
            .iter()
            .filter_map(|s| match s {
                egui::Shape::Text(t) if t.galley.job.text == "\u{2605}" => {
                    Some((t.galley.job.sections[0].format.color, s.visual_bounding_rect().center()))
                }
                _ => None,
            })
            .collect();
        let yellow = stars.iter().filter(|(c, _)| *c == GOLD).count();
        let green = stars.iter().filter(|(c, _)| *c == GREEN).count();
        assert_eq!((yellow, green), (3, 2), "one yellow per used tile, one green per reused tile: {stars:?}");

        // Each sits in the bottom-right quarter of its own tile.
        let board_tiles: Vec<Rect> = shapes
            .iter()
            .filter_map(|s| match s {
                egui::Shape::Rect(r) if r.fill == TILE => Some(r.rect),
                _ => None,
            })
            .collect();
        for (_, at) in &stars {
            let tile = board_tiles.iter().find(|t| t.contains(*at)).expect("a star outside every tile");
            assert!(at.x > tile.center().x && at.y > tile.center().y, "star at {at:?} is not bottom-right in {tile:?}");
        }
    }

    /// The sixteen board tiles' centres from a playing frame, row by row.
    fn board_centres(shapes: &[egui::Shape]) -> Vec<Pos2> {
        let mut tiles: Vec<Rect> = shapes
            .iter()
            .filter_map(|s| match s {
                egui::Shape::Rect(r) if r.fill == TILE && r.rect.width() > 40.0 && (r.rect.width() - r.rect.height()).abs() < 1.0 => {
                    Some(r.rect)
                }
                _ => None,
            })
            .collect();
        tiles.sort_by(|a, b| (a.min.y, a.min.x).partial_cmp(&(b.min.y, b.min.x)).unwrap());
        assert_eq!(tiles.len(), 16, "expected the board's sixteen tiles");
        tiles.iter().map(|t| t.center()).collect()
    }

    #[test]
    fn a_quick_swipe_keeps_the_letter_it_started_on() {
        let size = Vec2::new(1000.0, 780.0);
        let mut app = offline_app(true);
        app.game.start_round();
        let ctx = egui::Context::default();
        let screen = Rect::from_min_size(Pos2::ZERO, size);
        let frame = |app: &mut WordLegendApp, events: Vec<egui::Event>| {
            let input = egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
            painted(ctx.run(input, |ctx| app.frame(ctx)).shapes)
        };
        let mut shapes = Vec::new();
        for _ in 0..3 {
            shapes = frame(&mut app, vec![]);
        }
        let c = board_centres(&shapes);
        let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };

        // Touch the top-left tile, and in the very next frame be on the tile
        // diagonally below it: a quick flick.
        frame(&mut app, vec![egui::Event::PointerMoved(c[0]), button(c[0], true)]);
        frame(&mut app, vec![egui::Event::PointerMoved(c[5])]);
        let path = app.game.path.clone();
        assert_eq!(
            path,
            vec![Position { row: 0, col: 0 }, Position { row: 1, col: 1 }],
            "the swipe lost the letter it started on"
        );
    }

    #[test]
    fn the_copy_icon_copies_the_whole_code() {
        let mut app = WordLegendApp::new();
        let ctx = egui::Context::default();
        // The card takes a couple of frames to size and centre itself; aim after.
        for _ in 0..4 {
            signup_frame(&mut app, &ctx, vec![]);
        }
        let code = app.signup.pending.as_ref().unwrap().recovery_code();

        let icon = ctx.read_response(egui::Id::new(COPY_CODE_ID)).expect("copy icon drawn").rect;
        let at = icon.center();
        let press = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        signup_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at)]);
        signup_frame(&mut app, &ctx, vec![press(true)]);
        let copied = signup_frame(&mut app, &ctx, vec![press(false)]);

        assert_eq!(copied, vec![code], "the icon did not copy the code");
        assert!(app.signup.copied_at.is_some(), "the icon should switch to its tick");
    }

    #[test]
    fn the_code_can_be_selected_and_copied_by_hand() {
        let mut app = WordLegendApp::new();
        let ctx = egui::Context::default();
        for _ in 0..4 {
            signup_frame(&mut app, &ctx, vec![]);
        }
        let code = app.signup.pending.as_ref().unwrap().recovery_code();

        // Select the middle two groups, as a drag across them would, then Ctrl+C.
        let id = egui::Id::new(RECOVERY_CODE_ID);
        let mut state = egui::widgets::text_edit::TextEditState::load(&ctx, id).expect("code field drawn");
        state.cursor.set_char_range(Some(egui::text::CCursorRange::two(
            egui::text::CCursor::new(5),
            egui::text::CCursor::new(14),
        )));
        state.store(&ctx, id);
        ctx.memory_mut(|m| m.request_focus(id));

        signup_frame(&mut app, &ctx, vec![]);
        let copied = signup_frame(&mut app, &ctx, vec![egui::Event::Copy]);
        assert_eq!(copied, vec![code[5..14].to_string()], "Ctrl+C did not copy the selection");

        // Typing into it changes nothing: it is there to be read, not edited.
        signup_frame(&mut app, &ctx, vec![egui::Event::Text("x".into()), egui::Event::Key {
            key: egui::Key::Backspace,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }]);
        assert_eq!(app.signup.pending.as_ref().unwrap().recovery_code(), code);
    }

    /// Signed in, the start card also offers the recovery code again.
    #[test]
    fn the_start_card_with_an_account_fits() {
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 780.0));
        let mut app = WordLegendApp::new();
        app.identity = Some(Identity { id: "0123456789ABCDEF".into(), name: "wordfan".into() });
        let ready = overlay_rect(&mut app, |a, ctx| a.overlay_ready(ctx));
        assert!(screen.contains_rect(ready), "start card overflows: {ready:?}");
    }

    /// Pasting a saved note fills in just the code, tidied; a refused clipboard
    /// says how to paste by hand instead, and the card still fits either way.
    #[test]
    fn pasting_a_code_fills_the_restore_field() {
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 780.0));
        let mut app = WordLegendApp::new();
        app.signup.restoring = true;

        app.signup.paste.put(Ok("Word Legend -- 0123-4567-89ab-cdef\n".into()));
        let card = overlay_rect(&mut app, |a, ctx| a.overlay_signup(ctx));
        assert_eq!(app.signup.code, "0123-4567-89AB-CDEF");
        assert!(screen.contains_rect(card), "restore card overflows: {card:?}");

        app.signup.paste.put(Err("clipboard access was refused".into()));
        let card = overlay_rect(&mut app, |a, ctx| a.overlay_signup(ctx));
        assert!(app.signup.paste_failed.is_some());
        assert_eq!(app.signup.code, "0123-4567-89AB-CDEF", "a failed paste must not wipe the field");
        assert!(screen.contains_rect(card), "restore card overflows: {card:?}");
    }

    /// The card must stay on screen when the window is too short to hold it,
    /// otherwise the button that starts the next round cannot be reached.
    #[test]
    fn a_short_window_scrolls_instead_of_overflowing() {
        for height in [560.0, 640.0, 700.0] {
            let size = Vec2::new(1000.0, height);
            let screen = Rect::from_min_size(Pos2::ZERO, size);
            let mut app = app_after_round();
            let over = overlay_rect_at(&mut app, |a, ctx| a.overlay_results(ctx), size);
            assert!(
                screen.contains_rect(over),
                "results card overflows a {height}px window: {over:?}"
            );
        }
    }

    fn geom() -> BoardGeometry {
        BoardGeometry::new(Pos2::ZERO, 460.0)
    }

    #[test]
    fn every_tile_centre_hits_its_own_tile() {
        let g = geom();
        for row in 0..SIZE {
            for col in 0..SIZE {
                let at = Position { row, col };
                assert_eq!(g.tile_at(g.center(at)), Some(at));
            }
        }
    }

    #[test]
    fn the_gaps_belong_to_no_tile() {
        let g = geom();
        let a = g.rect(Position { row: 0, col: 0 });
        let b = g.rect(Position { row: 0, col: 1 });
        // Halfway between two tiles, horizontally.
        let between = Pos2::new((a.max.x + b.min.x) * 0.5, a.center().y);
        assert_eq!(g.tile_at(between), None);

        // The point where four tiles meet: the one that used to resolve to a
        // neighbour and inject a letter into a diagonal drag.
        let corner = Pos2::new((a.max.x + b.min.x) * 0.5, (a.max.y + g.gap * 0.5).max(a.max.y));
        assert_eq!(g.tile_at(Pos2::new(corner.x, a.max.y + g.gap * 0.5)), None);
    }

    #[test]
    fn a_diagonal_drag_does_not_clip_its_neighbours() {
        let g = geom();
        let (p, o) = (Position { row: 0, col: 0 }, Position { row: 1, col: 1 });
        let (t, s) = (Position { row: 0, col: 1 }, Position { row: 1, col: 0 });

        // The tile the drag starts on is included, which is harmless: re-offering
        // the tile already at the head of the path is a no-op.
        //
        // What matters is that neither neighbour appears. P -> O passes through
        // the point where all four tiles meet.
        let crossed = g.tiles_along(g.center(p), g.center(o));
        assert_eq!(crossed, vec![p, o], "diagonal picked up {crossed:?}");
        assert!(!crossed.contains(&t) && !crossed.contains(&s));

        // The return diagonal must not pick up P or O either: in a P-O-T-S trace
        // both are already in the word, so a stray hit would rewind it.
        let crossed = g.tiles_along(g.center(t), g.center(s));
        assert_eq!(crossed, vec![t, s], "diagonal picked up {crossed:?}");
        assert!(!crossed.contains(&p) && !crossed.contains(&o));
    }

    /// The exact path from the bug report: P top-left, diagonal to O, up to T,
    /// diagonal again to S, spelling POTS around a 2x2 square.
    #[test]
    fn tracing_a_square_keeps_every_letter() {
        let g = geom();
        let mut game = Game::new();
        game.phase = Phase::Playing;

        let corners = [
            Position { row: 0, col: 0 }, // P
            Position { row: 1, col: 1 }, // O
            Position { row: 0, col: 1 }, // T
            Position { row: 1, col: 0 }, // S
        ];

        game.begin_path(corners[0]);
        for pair in corners.windows(2) {
            for at in g.tiles_along(g.center(pair[0]), g.center(pair[1])) {
                game.extend_path(at);
            }
        }

        assert_eq!(game.path, corners, "the square trace lost or rewound letters");
    }

    #[test]
    fn a_fast_flick_across_a_row_skips_nothing() {
        let g = geom();
        let row: Vec<Position> = (0..SIZE).map(|col| Position { row: 2, col }).collect();
        // One frame, all the way across: every tile in between must still register.
        let crossed = g.tiles_along(g.center(row[0]), g.center(row[SIZE - 1]));
        assert_eq!(crossed, row, "a flick skipped a letter: {crossed:?}");
    }

    #[test]
    fn thousands_groups_digits() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1_000), "1,000");
        assert_eq!(thousands(370_105), "370,105");
    }
}
