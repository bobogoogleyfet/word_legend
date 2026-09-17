# Word Legend

A multiplayer word game. Everyone plays the same 4×4 board on the same clock:
trace touching letters to spell words, then see where you placed on the round's
leaderboard. Your league follows your average over your last ten games.

Runs in the browser via WebAssembly — on a phone or a desktop — and natively on
the desktop.

**Play it:** https://bobogoogleyfet.github.io/word_legend/

## Playing

### Getting in

There is no email and no password. The first time you open the game it shows a
**recovery code** and asks for a display name. The code *is* the account: copy it
(the icon beside it, or select it and Ctrl+C) and keep it somewhere safe. On
another device, choose **I already have a code** and paste it in.

From the home screen, **swipe P-L-A-Y** across the tiles, just as you would spell
a word, to join the round. A tap explains the swipe instead; on a desktop, Enter
works too. If a round is nearly over or everyone is on the scorecard, you are
**queued** for the next one, with a countdown and a way out of the queue.

### A round

Rounds last **three minutes**, then everyone gets a **30-second scorecard** before
the next begins on its own.

- Drag across touching letters — in any of the eight directions — and let go to
  submit. Each tile can be used once per word; dragging back over a tile rewinds
  to it. On a desktop you can also click letters one at a time and press Enter.
- Words need at least three letters. The `Qu` tile counts as two letters.
- A tile gets a **yellow star** once a found word has used it, and a **green
  star** once a second has. Unused letters stand out at a glance.
- The theme, if the board has one, is shown under the board, along with the
  letter bonus so far.
- **LEAVE** (in the bar at the top) ends your round early, after a confirmation.
  Anything you scored is banked into your average and put on the leaderboard
  straight away, and you go home rather than into the next round.
- Refreshing the page does not lose a round: it picks up where you were, with
  whatever time the round has left. If the round ended while the page was closed,
  your words are banked and handed in.

### The scorecard

Your board with its stars, the round's numbers, the **Players** leaderboard (with
each player's league) and the **Words** you could have found — Common, Obscure and
the board's theme, side by side, each scrolling on its own.

- **Tap a word** to draw its path on the board.
- **Tap a letter** to list every word that runs through it.
- **Back to home** leaves the cycle of rounds; nothing is lost by it.

A round in which you found nothing is shown on the leaderboard with 0 but does
not count toward your average.

## Scoring

Length dominates volume — one seven-letter word beats four three-letter ones:

| Letters | 3   | 4   | 5   | 6     | 7     | 8     | 9+                    |
| ------- | --- | --- | --- | ----- | ----- | ----- | --------------------- |
| Points  | 100 | 400 | 800 | 1,400 | 1,800 | 2,200 | +400 per extra letter |

On top of that:

| Bonus | Points |
| --- | --- |
| An **obscure** word (valid, but not an everyday one) | +10% of its points |
| A **superword** — one word filling all sixteen tiles | 20,000 flat |
| Each letter used at least once (yellow star) | +25 |
| Each letter used at least twice (green star) | +100 more |
| Every letter on the board used | +500 |

The letter bonus is worth up to 2,500 a round.

## Boards

Boards come in a fixed rotation rather than at random, so nothing goes missing
for thirty rounds and then turns up twice in a row:

- **Every fifth round is a superword board**, built around a sixteen-letter word
  laid out across every tile — CHARACTERIZATION, MISUNDERSTANDING,
  WHATCHAMACALLITS. Find it for 20,000.
- **The other four are themed.** A themed board holds **at least twenty words
  from its theme**, plurals included — Animals, Food, Nature, Body, Music,
  Science, Math, History, Verbs and Adjectives rotate evenly, never back to back.
- **Each month favours its season.** One round in five is the month's theme:

  | Month | Theme | Month | Theme |
  | --- | --- | --- | --- |
  | January | Winter | July | Fourth of July |
  | February | Valentine's Day | August | School |
  | March | St. Patrick's Day | September | Fall |
  | April | Easter | October | Halloween |
  | May | Spring | November | Thanksgiving |
  | June | Summer | December | Christmas |

Every board also clears a quality bar: at least 60 everyday words, a few long
ones, and every tile usable in at least two words, so no letter is dead.

## Leagues

Your rank is the **average of your last ten banked games**, not your last one: a
single bad board will not cost you a league, and a single lucky one will not win
one.

| League   | Average needed |
| -------- | -------------- |
| Bronze   | —              |
| Silver   | 4,000          |
| Gold     | 8,000          |
| Platinum | 13,000         |
| Diamond  | 20,000         |
| Hero     | 30,000         |

- **Promotion** waits until you have banked five games, then moves you up one
  league whenever your average reaches the next.
- **Relegation** does not wait: if your average falls below your league's floor,
  you drop back one league.
- The gaps widen toward the top. Hero takes a 26,000 average from everything but
  superwords.

The **Stats** page (from the home screen) charts your last fifty games — each
game's score and the average it left you with — and shows every league with the
average it takes, your best game (score, best word, most words), all-time
averages and totals.

Your league, history and stats are saved on the device: `localStorage` in the
browser, dotfiles in your home directory on the desktop.

## Running it

### In a browser

Needs the wasm target and a matching `wasm-bindgen` CLI:

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.100   # must match Cargo.lock

./build-web.sh
python3 -m http.server -d dist 8080
```

Then open <http://localhost:8080>. `build-web.sh` stamps the page with a hash of
the build, so a reload after a deploy fetches the new build rather than a cached
one.

### On the desktop

```sh
cargo run --release
```

### The server

Multiplayer runs on a Cloudflare Worker — see [`server/README.md`](server/README.md).
Without it, or when it cannot be reached, the game still works: it plays **solo**
boards on its own clock (keeping the superword every fifth round) and rejoins the
shared rounds when the server answers again.

## How it works

**One clock for everyone.** Round *N* runs from a fixed epoch plus *N* × 210
seconds, so the server never has to store where the cycle is, and every client
agrees on it. Clients read the clock occasionally and carry it forward between
reads.

**Boards are built ahead of time.** The Rust generator builds a thousand rounds
at a time — plus a thousand per month for the seasonal themes — and they are
uploaded as packs for the server to serve and to check scores against. A themed
board with twenty theme words cannot be found by rolling dice, so it is searched
for by simulated annealing, starting from a few planted words. The exporter uses
every core; the full set takes about half an hour.

**Scores are checked, not trusted.** A client sends the words it found and the
path it traced for each. The server scores the words against the round's word
list and awards the letter bonus only for paths that really spell their word
through touching tiles.

**The leaderboard is complete at the bell.** Players report progress while they
play, so everyone in the round is already on the table when it ends. Each round's
table lives in its own Durable Object, which keeps simultaneous submissions from
overwriting each other.

**The dictionary has two tiers.** The lexicon is ENABLE — 172,823 words, public
domain — so every word the game accepts is a real, playable one. About 63,000
everyday words form the **Common** tier that boards are built from and judged
against; the rest are **Obscure**: accepted, and worth 10% more, but never held
against you on a scorecard.

## Layout

| File | Contents |
| --- | --- |
| `src/game.rs` | A round: tracing, scoring, the letter bonus, the solver, saving a round in play |
| `src/rotation.rs` | The board schedule — superwords, themes, seasons — and building themed and superword boards |
| `src/live.rs` | Following the shared round: the clock, joining, leaving, resuming, handing in, the leaderboard |
| `src/net.rs` | Talking to the server, without ever blocking a frame |
| `src/league.rs` | The rank average, leagues, history and lifetime stats |
| `src/ui.rs` | The egui front end: home, board, scorecard, stats, signup |
| `src/identity.rs` | Accounts: a random id shown as a recovery code, and a display name |
| `src/dictionary.rs` | The two-tier word list and its prefix trie |
| `src/themes.rs` | Theme word lists, from `themes.txt` |
| `src/export.rs`, `src/bin/export_rounds.rs` | Building round packs for the server |
| `src/clipboard.rs` | Reading the clipboard for the recovery code's Paste button |
| `src/storage.rs` | Saves: a dotfile on desktop, `localStorage` on the web |
| `themes.txt`, `superwords.txt` | The theme lists and the curated superwords |
| `server/` | The Cloudflare Worker |
| `vendor/egui_glow` | egui's renderer with its shaders at high precision — see below |

## Development

```sh
cargo test                  # the game
cd server && npm test       # the Worker (needs round packs; see server/README.md)
```

The UI tests render every screen headlessly — at desktop, tablet and phone sizes —
and check that nothing is drawn off screen, that buttons are touch-sized on a
phone, and that interactions (swiping PLAY, leaving a round, tapping a letter,
scrolling a word column) do what they should. Storage is stubbed out under
`cfg(test)`, so the suite never touches your real saves.

`vendor/egui_glow` is egui's OpenGL renderer with one change: its shaders use high
precision. Many phone GPUs treat medium precision as 16-bit, which drew slivers of
neighbouring letters beside every glyph. Drop the vendored copy once egui_glow no
longer uses medium precision.

## Deploying to GitHub Pages

`.github/workflows/pages.yml` runs the tests, builds the web bundle and publishes
it on every push to `main`. The repository must be public (unless on a paid plan),
and **Settings → Pages → Source** must be **GitHub Actions**.

The bundle is ~7 MB, **~2.3 MB gzipped** — what actually crosses the wire, since
Pages compresses it — because both word lists are compiled in. The page shows a
loading bar while it downloads.
