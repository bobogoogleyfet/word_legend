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

/// Seconds left at which the clock starts pulsing red.
const PANIC_TIME: f32 = 15.0;

pub struct WordLegendApp {
    game: Game,
    /// Drives the tile "pop" when a word lands, independent of game state.
    elapsed: f32,
}

impl WordLegendApp {
    pub fn new() -> Self {
        Self {
            game: Game::new(),
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
        let gap = board_size * 0.035;
        let tile_size = (board_size - gap * 5.0) / SIZE as f32;

        let (response, painter) =
            ui.allocate_painter(Vec2::splat(board_size), Sense::click_and_drag());
        let origin = response.rect.min;

        let tile_rect = |row: usize, col: usize| {
            Rect::from_min_size(
                origin
                    + Vec2::new(
                        gap + col as f32 * (tile_size + gap),
                        gap + row as f32 * (tile_size + gap),
                    ),
                Vec2::splat(tile_size),
            )
        };

        // Generous hit boxes: the gaps between tiles belong to the nearest tile so
        // a fast drag never skips a letter.
        let hit = |pos: Pos2| -> Option<Position> {
            let local = pos - origin;
            let step = tile_size + gap;
            let col = ((local.x - gap * 0.5) / step).floor();
            let row = ((local.y - gap * 0.5) / step).floor();
            if row < 0.0 || col < 0.0 || row >= SIZE as f32 || col >= SIZE as f32 {
                return None;
            }
            Some(Position { row: row as usize, col: col as usize })
        };

        self.handle_board_input(&response, hit);

        self.draw_trail(&painter, &tile_rect);

        for row in 0..SIZE {
            for col in 0..SIZE {
                self.draw_tile(&painter, tile_rect(row, col), Position { row, col });
            }
        }

        self.draw_score_popup(&painter, response.rect);
    }

    fn handle_board_input(
        &mut self,
        response: &egui::Response,
        hit: impl Fn(Pos2) -> Option<Position>,
    ) {
        if self.game.phase != Phase::Playing {
            return;
        }

        if let Some(pos) = response.interact_pointer_pos().and_then(&hit) {
            if response.drag_started() {
                self.game.is_dragging = true;
                self.game.begin_path(pos);
            } else if response.dragged() && self.game.is_dragging {
                self.game.extend_path(pos);
            } else if response.clicked() {
                // Click-to-chain: tap letters in turn, tap the last one again to submit.
                match self.game.path.last() {
                    Some(&last) if last == pos => self.game.submit_path(),
                    Some(_) => self.game.extend_path(pos),
                    None => self.game.begin_path(pos),
                }
            }
        }

        if response.drag_stopped() {
            if self.game.is_dragging {
                self.game.is_dragging = false;
                // A one-tile drag is a mis-click, not an attempt at a word.
                if self.game.path.len() > 1 {
                    self.game.submit_path();
                } else {
                    self.game.clear_path();
                }
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

    fn draw_trail(&self, painter: &egui::Painter, tile_rect: &impl Fn(usize, usize) -> Rect) {
        if self.game.path.len() < 2 {
            return;
        }

        let color = if self.game.current_word_is_valid() { GREEN } else { ACCENT };
        let centers: Vec<Pos2> = self
            .game
            .path
            .iter()
            .map(|p| tile_rect(p.row, p.col).center())
            .collect();

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
                egui::RichText::new(format!("/ {} on board", self.game.solutions.len()))
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
            app.standings_table(ui);
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
            ui.label(egui::RichText::new(format!("{}", app.game.score)).size(64.0).color(GOLD).strong());
            ui.label(egui::RichText::new(app.game.rank()).size(24.0).color(ACCENT).strong());

            if app.game.is_new_best() {
                ui.label(egui::RichText::new("★ NEW PERSONAL BEST").size(14.0).color(GOLD).strong());
            }

            ui.add_space(18.0);
            ui.horizontal(|ui| {
                stat(ui, "WORDS", &format!("{}", app.game.found.len()));
                ui.add_space(28.0);
                stat(
                    ui,
                    "ON BOARD",
                    &format!("{}", app.game.solutions.len()),
                );
                ui.add_space(28.0);
                stat(
                    ui,
                    "LONGEST",
                    &app.game
                        .longest_found()
                        .map(|w| w.word.to_uppercase())
                        .unwrap_or_else(|| "—".to_string()),
                );
            });

            ui.add_space(18.0);
            app.season_banner(ui);
            app.standings_table(ui);

            ui.add_space(16.0);
            ui.label(egui::RichText::new("YOU MISSED").size(12.0).color(MUTED).strong());
            ui.add_space(4.0);

            egui::ScrollArea::vertical().max_height(84.0).show(ui, |ui| {
                let missed = app.game.missed_words(40);
                if missed.is_empty() {
                    ui.label(egui::RichText::new("Nothing. You cleared the board.").size(14.0).color(GREEN));
                } else {
                    ui.horizontal_wrapped(|ui| {
                        for word in missed {
                            ui.label(
                                egui::RichText::new(word.to_uppercase())
                                    .size(14.0)
                                    .color(if word.len() >= 6 { GOLD } else { MUTED }),
                            );
                        }
                    });
                }
            });

            ui.add_space(18.0);
            let label = if app.game.season_outcome.is_some() { "NEW SEASON" } else { "NEXT ROUND" };
            if big_button(ui, label, ACCENT) {
                app.game.start_round();
            }
        });
    }

    /// The six-strong league table, with your row picked out.
    fn standings_table(&self, ui: &mut egui::Ui) {
        let season = &self.game.season;
        let played = season.round > 0;

        ui.horizontal(|ui| {
            ui.add_space(60.0);
            ui.label(egui::RichText::new("LEAGUE TABLE").size(11.0).color(MUTED).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(60.0);
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
                ui.add_space(60.0);
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
                    ui.add_space(60.0);
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

        egui::Area::new(egui::Id::new("overlay"))
            .order(egui::Order::Foreground)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.set_width(600.0);
                egui::Frame::default()
                    .fill(PANEL)
                    .stroke(Stroke::new(1.0_f32, TILE_EDGE))
                    .corner_radius(18.0)
                    .inner_margin(egui::Margin::same(30))
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .max_height(screen.height() - 60.0)
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                ui.vertical_centered(|ui| contents(self, ui));
                            });
                    });
            });
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

fn stat(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.vertical(|ui| {
        ui.label(egui::RichText::new(label).size(10.0).color(MUTED).strong());
        ui.label(egui::RichText::new(value).size(20.0).color(TEXT).strong());
    });
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
    fn overlay_rect(app: &mut WordLegendApp, draw: fn(&mut WordLegendApp, &egui::Context)) -> Rect {
        let ctx = egui::Context::default();
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 780.0));
        for _ in 0..2 {
            let input = egui::RawInput { screen_rect: Some(screen), ..Default::default() };
            let _ = ctx.run(input, |ctx| draw(app, ctx));
        }
        let state = egui::AreaState::load(&ctx, egui::Id::new("overlay")).expect("overlay shown");
        Rect::from_min_size(state.left_top_pos(), state.size.expect("overlay sized"))
    }

    /// The league table made these cards much taller; they must still fit the window.
    #[test]
    fn overlay_cards_fit_the_window() {
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 780.0));

        let mut app = WordLegendApp::new();
        let ready = overlay_rect(&mut app, |a, ctx| a.overlay_ready(ctx));
        assert!(screen.contains_rect(ready), "start card overflows: {ready:?}");

        // A season-ending round shows the most content: banner, table and all.
        let mut app = WordLegendApp::new();
        for _ in 0..4 {
            app.game.season.record_round(9_000, 30_000);
        }
        app.game.start_round();
        app.game.time_left = 0.0;
        app.game.tick(0.2);
        assert!(app.game.season_outcome.is_some(), "expected a finished season");

        let over = overlay_rect(&mut app, |a, ctx| a.overlay_results(ctx));
        assert!(screen.contains_rect(over), "results card overflows: {over:?}");
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
