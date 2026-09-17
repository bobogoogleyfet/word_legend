//! egui front end: the board, the clock, and the round-end scorecard.

use crate::clipboard::Paste;
use crate::game::{Feedback, Game, Phase, Position, RESULTS_SECONDS, ROUND_SECONDS, SIZE};
use crate::identity::{self, Identity};
use crate::league::{Movement, FORM_GAMES, LEAGUES, MIN_GAMES_TO_MOVE};
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
    /// Which of the Common / Obscure / Theme tabs the scorecard is showing.
    results_tab: usize,
    /// Where the cursor was last frame, so a drag can be traced as a segment
    /// rather than sampled as isolated points.
    drag_from: Option<Pos2>,
    /// Drives the tile "pop" when a word lands, independent of game state.
    elapsed: f32,
    /// When the recovery code was last copied from the start screen, to confirm it.
    code_copied_at: Option<f64>,
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
            results_tab: 0,
            drag_from: None,
            elapsed: 0.0,
            code_copied_at: None,
        }
    }
}

impl eframe::App for WordLegendApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
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

        egui::TopBottomPanel::top("hud")
            .frame(egui::Frame::default().fill(PANEL).inner_margin(egui::Margin::symmetric(20, 14)))
            .show(ctx, |ui| self.hud(ui));

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

        egui::CentralPanel::default()
            .frame(egui::Frame::default().fill(BG).inner_margin(egui::Margin::same(16)))
            .show(ctx, |ui| self.board_area(ui));

        if self.needs_signup() {
            self.overlay_signup(ctx);
            ctx.request_repaint();
            return;
        }

        match self.game.phase {
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
        self.overlay(ctx, |app, ui| {
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
    }

    fn signup_new(&mut self, ui: &mut egui::Ui, pending: &Identity) {
        ui.label(egui::RichText::new("YOUR RECOVERY CODE").size(11.0).color(MUTED).strong());
        ui.add_space(6.0);

        // The code is the account. Make it impossible to miss. It is a read-only
        // text field rather than painted text, so it can be selected and copied
        // with Ctrl+C like any other text, and the icon beside it copies it whole.
        let code = pending.recovery_code();
        let (rect, _) = ui.allocate_exact_size(Vec2::new(420.0, 56.0), Sense::hover());
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
                .font(FontId::monospace(26.0))
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
                .desired_width(280.0)
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
        ui.allocate_ui_with_layout(
            Vec2::new(320.0 + 8.0 + 70.0, 30.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.signup.code)
                        .desired_width(320.0)
                        .font(egui::TextStyle::Monospace)
                        .hint_text(hint("XXXX-XXXX-XXXX-XXXX")),
                );
                if ui.add_sized([70.0, 24.0], egui::Button::new("Paste")).clicked() {
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
                .desired_width(280.0)
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
        ui.horizontal(|ui| {
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

            let rank = &self.game.ranking;
            ui.vertical(|ui| {
                ui.label(
                    egui::RichText::new(LEAGUES[rank.league].name.to_uppercase())
                        .size(15.0)
                        .color(league_color(rank.league))
                        .strong(),
                );
                ui.label(
                    egui::RichText::new(format!("Rank {} avg", thousands(rank.average() as usize)))
                        .size(11.0)
                        .color(MUTED),
                );
            });

            ui.add_space(24.0);

            ui.vertical(|ui| {
                let name = self.identity.as_ref().map(|i| i.name.as_str()).unwrap_or("");
                ui.label(egui::RichText::new(name).size(15.0).color(TEXT).strong());
                let (label, color) = self.link_label();
                ui.label(egui::RichText::new(label).size(11.0).color(color).strong());
            });

            ui.add_space(24.0);

            ui.vertical(|ui| {
                ui.add_space(4.0);
                self.timer_bar(ui);
            });
        });
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

            let width = (ui.available_width() - 130.0).max(120.0);
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
        const HINT_H: f32 = 20.0;
        const GAP: f32 = 14.0;

        let avail = ui.available_size();
        let board_size = (avail.y - PILL_H - HINT_H - GAP * 2.0)
            .min(avail.x)
            .min(540.0)
            .max(240.0);
        let slack = avail.y - (PILL_H + HINT_H + GAP * 2.0 + board_size);

        ui.vertical_centered(|ui| {
            ui.add_space((slack * 0.5).max(0.0));
            self.word_pill(ui);
            ui.add_space(GAP);
            self.board(ui, board_size);
            ui.add_space(GAP);
            ui.label(
                egui::RichText::new("Drag across touching letters — or click them one by one, then Enter")
                    .size(12.0)
                    .color(MUTED),
            );
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
        let geom = BoardGeometry::new(response.rect.min, board_size);

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
            if let Some(at) = pointer.and_then(|p| geom.tile_at(p)) {
                self.game.begin_path(at);
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

    fn overlay_ready(&mut self, ctx: &egui::Context) {
        self.overlay(ctx, |app, ui| {
            ui.label(egui::RichText::new("WORD LEGEND").size(46.0).color(TEXT).strong());
            ui.label(
                egui::RichText::new(format!("{} League", LEAGUES[app.game.ranking.league].name))
                    .size(16.0)
                    .color(league_color(app.game.ranking.league))
                    .strong(),
            );
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(format!(
                    "Three minutes a round. Your rank is your average over the last {FORM_GAMES}."
                ))
                .size(13.0)
                .color(MUTED),
            );

            ui.add_space(18.0);
            ui.horizontal(|ui| {
                ui.add_space(60.0);
                ui.vertical(|ui| {
                    ui.set_width(340.0);
                    app.form_panel(ui);
                });
            });
            ui.add_space(16.0);

            for line in [
                "Drag across touching letters — any direction, diagonals included",
                "Longer words are worth far more: a 7 beats four 3s",
            ] {
                ui.label(egui::RichText::new(format!("•  {line}")).size(13.0).color(MUTED));
            }

            ui.add_space(12.0);
            ui.label(
                egui::RichText::new(format!(
                    "Boards built from {} common words · {} more accepted if you find them",
                    thousands(app.game.dictionary.common_count()),
                    thousands(app.game.dictionary.word_count() - app.game.dictionary.common_count())
                ))
                .size(12.0)
                .color(MUTED),
            );

            ui.add_space(18.0);
            let (button, note, color) = app.join_prompt();
            if big_button(ui, &button, ACCENT) {
                app.live.play_now(&mut app.game, app.now);
            }
            ui.add_space(8.0);
            ui.label(egui::RichText::new(note).size(12.0).color(color));

            // The code is only shown once at signup; this is the way back to it.
            if let Some(code) = app.identity.as_ref().map(|me| me.recovery_code()) {
                ui.add_space(10.0);
                let copied = app.code_copied_at.is_some_and(|t| app.now - t < 3.0);
                let (label, color) =
                    if copied { ("Recovery code copied", GREEN) } else { ("Copy my recovery code", ACCENT) };
                if ui.link(egui::RichText::new(label).size(12.0).color(color)).clicked() {
                    ui.ctx().copy_text(code);
                    app.code_copied_at = Some(app.now);
                }
            }
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
                let button = if waiting { "YOU'RE IN" } else { "JOIN NEXT ROUND" };
                (button.to_string(), format!("Next round starts in {}", clock_text(next)), AMBER)
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

            // The board you just played, the numbers that came out of it, and where
            // that leaves you in the league -- side by side, so the card still fits.
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

            ui.add_space(12.0);
            app.rank_banner(ui);

            match app.game.round {
                Some(round) => {
                    ui.horizontal_top(|ui| {
                        ui.vertical(|ui| {
                            ui.set_width(300.0);
                            app.leaderboard(ui, round);
                        });
                        ui.add_space(16.0);
                        ui.vertical(|ui| {
                            ui.set_width((ui.available_width()).max(200.0));
                            app.word_tabs(ui);
                        });
                    });
                    ui.add_space(18.0);
                    // A shared scorecard is everyone's, so it runs out rather than
                    // being skipped.
                    let left = app.game.results_left.max(0.0);
                    ui.label(
                        egui::RichText::new(format!("Next round in {}s", left.ceil() as u32))
                            .size(15.0)
                            .color(if left <= 10.0 { AMBER } else { MUTED })
                            .strong(),
                    );
                }
                None => {
                    app.word_tabs(ui);
                    ui.add_space(18.0);
                    if big_button(ui, "NEXT ROUND", ACCENT) {
                        app.live.play_now(&mut app.game, app.now);
                    }
                    ui.add_space(6.0);
                    let left = app.next_shared_round_in().unwrap_or(app.game.results_left.max(0.0));
                    ui.label(
                        egui::RichText::new(format!("Next round in {}s", left.ceil() as u32))
                            .size(12.0)
                            .color(if left <= 10.0 { AMBER } else { MUTED }),
                    );
                }
            }
        });
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
            .show(ui, |ui| {
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
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                egui::RichText::new(format!("{} words", entry.words)).size(11.0).color(MUTED),
                            );
                            ui.label(
                                egui::RichText::new(thousands(entry.score as usize)).size(13.0).color(color).strong(),
                            );
                        });
                    });
                }
            });
    }

    /// The played board, with each tile bordered by how hard it worked.
    fn used_board(&self, ui: &mut egui::Ui, size: f32) {
        let (response, painter) = ui.allocate_painter(Vec2::splat(size), Sense::hover());
        let geom = BoardGeometry::new(response.rect.min, size);

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
            }
        }

        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            for (label, color) in [("unused", BLUE), ("used once", AMBER), ("reused", GREEN)] {
                ui.label(egui::RichText::new(format!("\u{25a0} {label}")).size(10.0).color(color));
            }
        });
    }

    fn round_stats(&self, ui: &mut egui::Ui) {
        let g = &self.game;
        let rows: [(&str, String); 6] = [
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

    /// Common / Obscure / Theme, the way WordHero split its word list.
    fn word_tabs(&mut self, ui: &mut egui::Ui) {
        let counts = [
            self.game.words.common.len(),
            self.game.words.obscure.len(),
            self.game.theme_words().len(),
        ];

        // The third tab is named after the board's own category when it has one.
        let theme_tab = self.game.theme.clone().unwrap_or_else(|| "Theme".to_string());
        let names = ["Common".to_string(), "Obscure".to_string(), theme_tab];

        ui.horizontal(|ui| {
            for (i, name) in names.iter().enumerate() {
                let selected = self.results_tab == i;
                let text = egui::RichText::new(format!("{name} ({})", counts[i]))
                    .size(13.0)
                    .color(if selected { TEXT } else { MUTED })
                    .strong();
                if ui.selectable_label(selected, text).clicked() {
                    self.results_tab = i;
                }
            }
        });

        let words: Vec<&String> = match self.results_tab {
            0 => self.game.words.common.iter().collect(),
            1 => self.game.words.obscure.iter().collect(),
            _ => self.game.theme_words(),
        };

        ui.add_space(6.0);
        egui::ScrollArea::vertical()
            .id_salt("word_tabs")
            .max_height(96.0)
            .show(ui, |ui| {
                if words.is_empty() {
                    ui.label(
                        egui::RichText::new("Nothing in this list for this board.")
                            .size(12.0)
                            .color(MUTED),
                    );
                    return;
                }
                ui.horizontal_wrapped(|ui| {
                    for word in words.iter().take(300) {
                        // Found words are picked out; the rest are what you left behind.
                        let found = self.game.has_found(word);
                        ui.label(
                            egui::RichText::new(word.to_uppercase())
                                .size(13.0)
                                .color(if found { GREEN } else { MUTED })
                                .strong(),
                        );
                    }
                });
            });
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

        // Recent rounds as bars, newest on the right.
        let peak = rank.recent.iter().copied().max().unwrap_or(1).max(1);
        let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 34.0), Sense::hover());
        let painter = ui.painter();
        let slot = rect.width() / FORM_GAMES as f32;
        for (i, score) in rank.recent.iter().enumerate() {
            let height = (*score as f32 / peak as f32) * rect.height();
            let bar = Rect::from_min_size(
                Pos2::new(rect.min.x + i as f32 * slot, rect.max.y - height.max(2.0)),
                Vec2::new((slot - 3.0).max(2.0), height.max(2.0)),
            );
            painter.rect_filled(bar, 2.0, league_color(rank.league).gamma_multiply(0.8));
        }

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

    /// Shown only on a round that actually moved you up or down.
    fn rank_banner(&self, ui: &mut egui::Ui) {
        let Some(change) = self.game.rank_change else { return };
        let color = match change.movement {
            Movement::Promoted => GREEN,
            Movement::Relegated => RED,
            Movement::Held => return,
        };
        ui.label(egui::RichText::new(change.headline()).size(22.0).color(color).strong());
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
        const MARGIN: f32 = 24.0;
        let width = 900.0_f32.min(screen.width() - MARGIN * 2.0);

        egui::Area::new(egui::Id::new("overlay"))
            .order(egui::Order::Foreground)
            .anchor(Align2::CENTER_TOP, Vec2::new(0.0, MARGIN))
            .show(ctx, |ui| {
                ui.set_width(width);
                egui::Frame::default()
                    .fill(PANEL)
                    .stroke(Stroke::new(1.0_f32, TILE_EDGE))
                    .corner_radius(18.0)
                    .inner_margin(egui::Margin::same(30))
                    .show(ui, |ui| {
                        // Laid out directly when there is room: a ScrollArea here
                        // collapses to about 400px regardless of the max height it
                        // is given, which hid the button under a fold. On a window
                        // too short for the card -- a phone in landscape, say --
                        // scrolling beats an unreachable button.
                        if screen.height() < SHORT_WINDOW {
                            egui::ScrollArea::vertical()
                                .id_salt("overlay_scroll")
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

impl BoardGeometry {
    pub fn new(origin: Pos2, board_size: f32) -> Self {
        let gap = board_size * 0.035;
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

    /// Every tile the segment `from` -> `to` passes over, in order, without repeats.
    pub fn tiles_along(&self, from: Pos2, to: Pos2) -> Vec<Position> {
        // A quarter of a tile is fine enough that the segment cannot cross a whole
        // tile between samples, and coarse enough to stay cheap.
        let stride = (self.tile * 0.25).max(1.0);
        let steps = ((from.distance(to) / stride).ceil() as usize).clamp(1, 256);

        let mut out: Vec<Position> = Vec::new();
        for i in 1..=steps {
            let t = i as f32 / steps as f32;
            let point = from + (to - from) * t;
            if let Some(at) = self.tile_at(point) {
                if out.last() != Some(&at) {
                    out.push(at);
                }
            }
        }
        out
    }
}

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
        app.game.time_left = 0.0;
        app.game.tick(0.2);
        assert_eq!(app.game.phase, Phase::Over, "expected the round to have ended");
        app
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
            .map(|i| net::Entry { name: format!("player_{i:02}"), score: 20_000 - i * 400, words: 30 })
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
