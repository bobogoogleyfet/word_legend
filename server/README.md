# Word Legend server

A single Cloudflare Worker plus one KV namespace. It runs the shared clock,
owns display names, checks submitted scores and serves the leaderboard.

## Credentials

**Nothing secret is committed, and nothing secret belongs in this directory.**

| What | Where it lives |
| --- | --- |
| Cloudflare login (deploying) | your own `wrangler login` session |
| `CLOUDFLARE_API_TOKEN` (CI) | a GitHub Actions secret |
| `SEED_SECRET` | `npx wrangler secret put SEED_SECRET`, stored by Cloudflare |
| Local dev values | `.dev.vars`, which is gitignored |

`wrangler.toml` is committed and holds no credentials. The KV namespace id in it
is a name, not a key: it grants nothing without an authenticated account.

Copy `.dev.vars.example` to `.dev.vars` for local work. If you ever paste a token
into a file here, check `git status` before committing.

## Setting it up

```sh
cd server
npm install

npx wrangler login                       # opens a browser
npx wrangler kv namespace create ROUNDS  # prints an id
# put that id into wrangler.toml under [[kv_namespaces]]

# Build the rounds the server serves and validates against, then upload them.
cd .. && cargo run --release --bin export_rounds -- rounds 1000
cd server && ./upload-rounds.sh

npx wrangler deploy
```

Deploying prints the Worker URL. That URL is what the game talks to.

## How a score is checked

The Worker never takes a score from the browser. A submission is a list of words;
the Worker looks each one up in that round's precomputed word set and adds up the
points itself. Inventing a score means inventing words that are genuinely on the
board, which is just playing the game.

Rounds are precomputed by the Rust generator rather than by the Worker, because
validating properly means owning the lexicon, and 1.7MB of it will not fit in a
Worker. `export_rounds` vets each board against the full quality bar -- word
count, a long word or two, and every tile usable in at least two words -- and
writes packs of 100 for upload.

## What is deliberately not defended

Anyone who can read the game can also run its solver, so a determined player
could submit a perfect round. Stopping that needs submissions paced across the
round and rate limiting, which is not built. The check here defeats inventing a
score outright, which is the prank worth defeating among people who know
each other.

## Endpoints

| Route | Purpose |
| --- | --- |
| `GET /round` | current round, phase, seconds left, and the board |
| `POST /claim` | `{id, name}` — take a display name |
| `POST /score` | `{id, round, words[]}` — submit a round, scored server-side |
| `GET /leaderboard?round=` | names and scores for a round |

The board's word list is never sent to clients: they work it out themselves, and
handing it over would turn each round into a copying exercise.

## Tests

```sh
cargo run --release --bin export_rounds -- rounds 100   # from the repo root
cd server && npm test
```

Covers name uniqueness, that a forged score is ignored, that invented words score
nothing, that a word cannot be counted twice, and that account ids never appear
on a leaderboard.

## Free tier

Workers gives 100,000 requests/day and KV gives 100,000 reads and 1,000 writes.
A round costs one write per player who submits, so the write limit is the one to
watch: roughly 1,000 submissions a day. Uploading 10 packs costs 10 writes.
