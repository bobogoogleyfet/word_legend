# Word Legend

A two-minute word game. Trace adjacent letters on a 4×4 board to spell words,
scoring by length, and climb a promotion-and-relegation ladder across six
leagues.

Runs natively on the desktop and in the browser via WebAssembly.

**Play it:** https://bobogoogleyfet.github.io/word_legend/
*(available once the repository is public and Pages is enabled — see
[Deploying](#deploying-to-github-pages))*

## Playing

Drag across touching letters to spell a word, then release. Letters connect in
any of the eight directions, diagonals included, and each tile can be used only
once per word. Dragging back over an earlier tile rewinds the trail.

You can also tap letters one at a time and press <kbd>Enter</kbd> to submit, or
<kbd>Esc</kbd> to clear. The word turns green while it is one that would score.

Words must be at least three letters. The `Qu` tile counts as two.

## Scoring

Length dominates volume — a single seven-letter find beats four three-letter
ones:

| Letters | 3   | 4   | 5   | 6     | 7     | 8     | 9+                 |
| ------- | --- | --- | --- | ----- | ----- | ----- | ------------------ |
| Points  | 100 | 400 | 800 | 1,400 | 1,800 | 2,200 | +400 per extra letter |

## Leagues

A season runs five rounds against a table of five rivals. **The top two are
promoted and the bottom two go down.** Ties go to you. Bronze cannot relegate
and Hero cannot promote, so winning at the top defends the title instead.

| League   | Rival strength |
| -------- | -------------- |
| Bronze   | 8% of par      |
| Silver   | 11.5%          |
| Gold     | 15%            |
| Platinum | 19%            |
| Diamond  | 24%            |
| Hero     | 30%            |

Rival scores are a share of the board's *par* — everything on offer if you found
every common word on it — rather than fixed targets. Par swings from roughly
28,000 to 75,000 depending on the draw, so fixed numbers would let a sparse board
hand rivals the season and a rich one hand it to you. Scaling to par means a good
board lifts the whole table.

Your league, season progress and best score persist between sessions: to
`~/.word_legend_save` on the desktop, and to `localStorage` in the browser.

## Running it

### Desktop

```sh
cargo run --release
```

### Browser

Needs the wasm target and a matching `wasm-bindgen` CLI:

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.100   # must match Cargo.lock

./build-web.sh
python3 -m http.server -d dist 8080
```

Then open <http://localhost:8080>.

## How it works

**Boards are generated, not sprinkled.** Letters come from the 16 standard
Boggle dice, which give a far better mix than sampling letter frequencies
independently. Each candidate board is then solved in full — a depth-first walk
over the 16 tiles, pruned against a prefix trie — and rerolled until it holds at
least 60 common words with at least four long ones. Generation takes well under
a millisecond.

**The dictionary has two tiers.** Any of ~364,000 words scores when you trace
it, so a legitimate find is never rejected. But boards and the end-of-round
"you missed" list are drawn from a curated ~63,000-word common tier, because the
full list is padded with obscurities — without the split, the longest words on a
board came out as things like `usninic` and `isolead`, which makes a scorecard
read as nonsense. `common.txt` is derived from SCOWL and intersected with
`dictionary.txt`, so it can never cite a word the game would refuse.

## Layout

| File                | Contents                                                |
| ------------------- | ------------------------------------------------------- |
| `src/dictionary.rs` | Two-tier word list and the prefix trie                   |
| `src/game.rs`       | Round logic, scoring, board generation and the solver    |
| `src/league.rs`     | Seasons, standings, promotion and relegation             |
| `src/storage.rs`    | Saves: a dotfile on desktop, `localStorage` on the web   |
| `src/ui.rs`         | egui front end                                           |
| `src/lib.rs`        | Web entry point                                          |
| `src/main.rs`       | Desktop entry point                                      |

## Development

```sh
cargo test
```

The suite covers scoring, adjacency, the solver's tier filtering, board richness
and the league rules. Two tests are worth knowing about:

- `ui::tests::overlay_cards_fit_the_window` lays the overlays out headlessly at
  the real window size and asserts the card fits, since the results card is tall
  enough to overflow a short window.
- `ui::tests::ui_glyphs_are_renderable` checks that every non-ASCII character in
  the UI exists in egui's bundled fonts. One previously rendered as a tofu box.

Storage is stubbed out under `cfg(test)`: several tests play a round to
completion, which persists, so an unstubbed build would overwrite your real
league ladder every time the suite ran.

## Deploying to GitHub Pages

`.github/workflows/pages.yml` builds and publishes on every push to `main`. It
runs the tests first, so a broken build will not deploy.

Two things are needed before the site will come up:

1. **The repository must be public**, unless you are on a paid plan — Pages on a
   private repository requires GitHub Pro, Team or Enterprise.
2. **Settings → Pages → Source** must be set to **GitHub Actions**.

The first run installs `wasm-bindgen-cli` and takes several minutes; the Rust
cache makes later runs much quicker.

### A note on size

Both word lists are compiled into the WebAssembly binary, which makes it ~7.8 MB,
or **~2.5 MB gzipped** — what actually crosses the wire, since Pages compresses
automatically. That is a slow first load on mobile, which is why the page shows a
loading bar rather than a blank screen. Dropping the full acceptance list and
shipping only the common tier would cut it to roughly 0.9 MB gzipped, at the cost
of rejecting obscure-but-real words.
