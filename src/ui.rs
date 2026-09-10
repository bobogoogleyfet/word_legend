//! egui front end: the board, the clock, and the round-end scorecard.

use crate::game::{Feedback, Game, Phase, Position, ROUND_SECONDS, SIZE};
use crate::league::{Movement, LEAGUES, SEASON_ROUNDS};
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

/// Below this window height the results card cannot fit and has to scroll.
const SHORT_WINDOW: f32 = 720.0;

/// Seconds left at which the clock starts pulsing red.
const PANIC_TIME: f32 = 15.0;

pub struct WordLegendApp {
    game: Game,
    /// Which of the Common / Obscure / Theme tabs the scorecard is showing.
    results_tab: usize,
    /// Where the cursor was last frame, so a drag can be traced as a segment
    /// rather than sampled as isolated points.
    drag_from: Option<Pos2>,
    /// Drives the tile "pop" when a word lands, independent of game state.
    elapsed: f32,
}

impl WordLegendApp {
    pub fn new() -> Self {
        Self {
            game: Game::new(),
            results_tab: 0,
            drag_from: None,
            elapsed: 0.0,
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
        self.game.tick(dt);

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

        match self.game.phase {
            Phase::Ready => self.overlay_ready(ctx),
            Phase::Over => self.overlay_results(ctx),
            Phase::Playing => {}
        }

        // The clock and the word animations both need a live frame rate.
        if self.game.phase == Phase::Playing || self.game.feedback_timer > 0.0 {
            ctx.request_repaint();
        }
    }
}

impl WordLegendApp {
    fn handle_keys(&mut self, ctx: &egui::Context) {
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
                    self.game.start_round();
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

            let season = &self.game.season;
            ui.vertical(|ui| {
                ui.label(
                    egui::RichText::new(LEAGUES[season.league].name.to_uppercase())
                        .size(15.0)
                        .color(league_color(season.league))
                        .strong(),
                );
                ui.label(
                    egui::RichText::new(format!(
                        "Round {} of {}",
                        season.round_number(),
                        SEASON_ROUNDS
                    ))
                    .size(11.0)
                    .color(MUTED),
                );
            });

            ui.add_space(28.0);

            ui.vertical(|ui| {
                ui.add_space(4.0);
                self.timer_bar(ui);
            });
        });
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
                    egui::RichText::new(format!("{}", self.game.best_score))
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
                egui::RichText::new(format!(
                    "{} League · Round {} of {}",
                    LEAGUES[app.game.season.league].name,
                    app.game.season.round_number(),
                    SEASON_ROUNDS
                ))
                .size(16.0)
                .color(league_color(app.game.season.league))
                .strong(),
            );
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new("Two minutes a round. Top two promote, bottom two drop.")
                    .size(13.0)
                    .color(MUTED),
            );

            ui.add_space(18.0);
            app.standings_table(ui, 60.0);
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
            if big_button(ui, "START ROUND", ACCENT) {
                app.game.start_round();
            }
            ui.add_space(8.0);
            ui.label(egui::RichText::new("or press Enter").size(12.0).color(MUTED));
        });
    }

    fn overlay_results(&mut self, ctx: &egui::Context) {
        self.overlay(ctx, |app, ui| {
            ui.label(egui::RichText::new("TIME'S UP").size(15.0).color(MUTED).strong());
            ui.label(egui::RichText::new(format!("{}", app.game.score)).size(46.0).color(GOLD).strong());
            ui.label(egui::RichText::new(app.game.rank()).size(20.0).color(ACCENT).strong());

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
                    app.standings_table(ui, 0.0);
                });
            });

            ui.add_space(12.0);
            app.season_banner(ui);
            app.word_tabs(ui);

            ui.add_space(18.0);
            let label = if app.game.season_outcome.is_some() { "NEW SEASON" } else { "NEXT ROUND" };
            if big_button(ui, label, ACCENT) {
                app.game.start_round();
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

    /// The six-strong league table, with your row picked out.
    fn standings_table(&self, ui: &mut egui::Ui, pad: f32) {
        let season = &self.game.season;
        let played = season.round > 0;

        ui.horizontal(|ui| {
            ui.add_space(pad);
            ui.label(egui::RichText::new("LEAGUE TABLE").size(11.0).color(MUTED).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(pad);
                ui.label(egui::RichText::new("TOTAL").size(11.0).color(MUTED).strong());
                if played {
                    ui.add_space(24.0);
                    ui.label(egui::RichText::new("LAST").size(11.0).color(MUTED).strong());
                }
            });
        });
        ui.add_space(4.0);

        let table_size = season.rivals.len() + 1;
        for row in season.standings() {
            // Show the promotion and relegation cut lines the way a real table does.
            let zone = if row.place <= 2 {
                GREEN
            } else if row.place > table_size - 2 {
                RED
            } else {
                MUTED
            };
            let name_color = if row.is_you { TEXT } else { MUTED };

            ui.horizontal(|ui| {
                ui.add_space(pad);
                ui.label(egui::RichText::new(format!("{}", row.place)).size(14.0).color(zone).strong());
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new(&row.name)
                        .size(14.0)
                        .color(name_color)
                        .strong()
                        .background_color(if row.is_you { ACCENT.gamma_multiply(0.30) } else { Color32::TRANSPARENT }),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(pad);
                    ui.label(egui::RichText::new(thousands(row.total as usize)).size(14.0).color(name_color).strong());
                    if played {
                        ui.add_space(24.0);
                        ui.label(egui::RichText::new(thousands(row.last as usize)).size(12.0).color(MUTED));
                    }
                });
            });
        }
    }

    /// Promotion or relegation, shown only on the round that ends a season.
    fn season_banner(&self, ui: &mut egui::Ui) {
        let Some(outcome) = self.game.season_outcome else { return };
        let color = match outcome.movement {
            Movement::Promoted => GREEN,
            Movement::Relegated => RED,
            Movement::Held if outcome.defended() => GOLD,
            Movement::Held => MUTED,
        };

        ui.label(
            egui::RichText::new(format!("SEASON OVER · {} PLACE", ordinal(outcome.place)))
                .size(11.0)
                .color(MUTED)
                .strong(),
        );
        ui.label(egui::RichText::new(outcome.headline()).size(24.0).color(color).strong());
        ui.add_space(10.0);
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

fn ordinal(n: usize) -> String {
    let suffix = match (n % 10, n % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{n}{suffix}")
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

fn style(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = BG;
    visuals.window_fill = PANEL;
    visuals.override_text_color = Some(TEXT);
    ctx.set_visuals(visuals);
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

    /// A scorecard mid-season, which is the tallest thing the game draws.
    fn app_at_season_end() -> WordLegendApp {
        let mut app = WordLegendApp::new();
        for _ in 0..4 {
            app.game.season.record_round(9_000, 30_000);
        }
        app.game.start_round();
        app.game.time_left = 0.0;
        app.game.tick(0.2);
        assert!(app.game.season_outcome.is_some(), "expected a finished season");
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
        let mut app = app_at_season_end();
        let over = overlay_rect(&mut app, |a, ctx| a.overlay_results(ctx));
        assert!(screen.contains_rect(over), "results card overflows: {over:?}");
    }

    /// The card must stay on screen when the window is too short to hold it,
    /// otherwise the button that starts the next round cannot be reached.
    #[test]
    fn a_short_window_scrolls_instead_of_overflowing() {
        for height in [560.0, 640.0, 700.0] {
            let size = Vec2::new(1000.0, height);
            let screen = Rect::from_min_size(Pos2::ZERO, size);
            let mut app = app_at_season_end();
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
    fn ordinals_read_correctly() {
        let got: Vec<String> = [1, 2, 3, 4, 6, 11, 12, 13, 21, 22].iter().map(|n| ordinal(*n)).collect();
        assert_eq!(
            got,
            ["1st", "2nd", "3rd", "4th", "6th", "11th", "12th", "13th", "21st", "22nd"]
        );
    }

    #[test]
    fn thousands_groups_digits() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1_000), "1,000");
        assert_eq!(thousands(370_105), "370,105");
    }
}
