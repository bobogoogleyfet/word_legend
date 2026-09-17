# Word Legend server

A Cloudflare Worker with a KV namespace and two Durable Object classes. It runs
the shared clock, owns display names, checks submitted scores, banks players'
scores and keeps each round's leaderboard.

## Where data is stored

| Where | What | Written |
| --- | --- | --- |
| KV namespace `ROUNDS` — `pack:N`, `pack:mMM:N` | the round packs: boards and their valid words | on upload |
| KV `ROUNDS` — `name:<name>` | which account owns a display name | when a name is claimed or changed |
| KV `ROUNDS` — `player:<id>` | an account's display name | when a name is claimed or changed |
| Durable Object `Player`, one per account | the last ten banked scores | each scored round |
| Durable Object `Leaderboard`, one per round | that round's table: names, scores, leagues | progress and final scores |

In the Cloudflare dashboard: **Storage & Databases → KV** for the namespace, and
**Workers & Pages → word-legend → Durable Objects** for the objects. No files,
recovery codes or personal details are stored: an account is a random id and a
chosen name.

## Credentials

**Nothing secret is committed, and nothing secret belongs in this directory.**

| What | Where it lives |
| --- | --- |
| Cloudflare login (deploying) | your own `wrangler login` session |
| `CLOUDFLARE_API_TOKEN` (CI) | a GitHub Actions secret |
| Local dev values | `.dev.vars`, which is gitignored |

`wrangler.toml` is committed and holds no credentials. The KV namespace id in it
is a name, not a key: it grants nothing without an authenticated account.

`SEED_SECRET`, mentioned in `wrangler.toml` and `.dev.vars.example`, is not used
by the Worker at present: rounds are precomputed and uploaded, not derived from a
secret seed.

## Deploying

**Run these from this `server/` directory.** Run from the repository root,
`wrangler` does not find this project's config and offers to deploy the whole
repository as a static site instead — say no.

```sh
cd server
npm install
npx wrangler login                   # once; opens a browser

npx wrangler deploy                  # the Worker
./upload-rounds.sh                   # the round packs, after building them (below)
```

Deploy the Worker before uploading new packs: a new Worker still reads older
packs, but an older Worker cannot score newer ones correctly.

### First-time setup

```sh
npx wrangler kv namespace create ROUNDS   # prints an id: put it in wrangler.toml
```

The `Leaderboard` and `Player` Durable Objects are declared in `wrangler.toml`
and created by the first deploy.

## Rounds

Rounds are precomputed by the Rust generator, which owns the lexicon — 1.7 MB of
words will not fit in a Worker. From the repository root:

```sh
cargo run --release --bin export_rounds -- rounds 1000 --all-months
```

This writes the default rounds to `rounds/pack-N.json` and each month's rounds —
favouring that month's seasonal theme — to `rounds/month-MM/pack-N.json`: 130
packs, about 32 MB, in roughly half an hour on sixteen cores. Leave off
`--all-months` for just the default rounds, in a couple of minutes.

A pack holds a hundred rounds, each with its grid, its theme (or `null` for a
superword board), and every valid word on it split into `words` (common) and
`obscure`. `upload-rounds.sh` uploads the defaults as `pack:N` and each month as
`pack:mMM:N`.

The Worker serves the pack for the month a round starts in (UTC), and falls back
to the default pack when that month has not been uploaded. Rounds cycle through
the thousand slots in a pack set; round *N* is slot *N* mod 1000.

## How a score is checked

The Worker never takes a score from the browser. A submission is the words found
and, for each, the path of tiles it was traced along. The Worker:

1. Looks each word up in that round's word list — an invented word scores nothing
   — and scores it: its length's points, 10% more if it is obscure, or 20,000 for
   a superword filling all sixteen tiles. A word counts once.
2. Checks each path spells its word through distinct, touching tiles, and adds the
   letter bonus from the valid ones: 25 per tile used, 100 more per tile used
   twice, 500 for using all sixteen. The client works the bonus out the same way;
   the two must agree.

This scoring must match the client's `score_word` and `letter_bonus` in
`src/game.rs` exactly, or a scorecard and the leaderboard will disagree.

## The leaderboard

Each round's table is its own `Leaderboard` Durable Object, named `round:N`.
Players' rounds end on the same tick, so scores arrive together; kept in KV, they
overwrote each other and cached reads hid new ones. In a Durable Object each
player's row is its own key and reads are consistent.

A player is put on the table as soon as they join a round, by progress reports
(`/progress`) while they play. Progress is scored like a final score but marked
not final, never banked, taken only while the round is being played, and never
overwrites a final score. So when a round ends, everyone who played is already
there; their final score (`/score`) replaces the row. An alarm clears a round's
table a week after its first score.

A round with nothing found is filed with 0 but not added to the player's average.

## What is deliberately not defended

Anyone who can read the game can also run its solver, so a determined player
could submit a perfect round. Stopping that needs submissions paced across the
round and rate limiting, which is not built. The check here defeats inventing a
score outright, which is the prank worth defeating among people who know each
other. A player's league on the leaderboard is as their game reports it.

## Endpoints

| Route | Purpose |
| --- | --- |
| `GET /round` | the current round, its phase (`play` or `results`), seconds left, and its board |
| `POST /claim` | `{id, name}` — take a display name |
| `POST /progress` | `{id, round, words[], paths[], league}` — progress while playing; nothing banked |
| `POST /score` | `{id, round, words[], paths[], league}` — the final score, scored and banked in the player's Durable Object |
| `GET /leaderboard?round=` | names, scores, word counts, leagues and whether each is final |

The board's word list is never sent to clients: they work it out themselves, and
handing it over would turn each round into a copying exercise.

## Tests

```sh
cargo run --release --bin export_rounds -- rounds 100   # from the repo root, if there are no packs
cd server && npm test
```

The tests run under plain Node, with a stand-in for `cloudflare:workers` and an
in-memory namespace for the Durable Objects, and the clock pinned mid-round. They
cover names, forged and invented scores, the obscure and superword bonuses, the
letter bonus against valid and invalid paths, simultaneous submissions,
progress and its throttling, the monthly packs, the leaderboard's leagues, and
that a scored round costs no KV write.

## Cost, and staying free

On the **Workers Free plan** the limits are hard: once a daily allowance is used
up, further requests fail until the day resets, and nothing is ever billed. The
Paid plan has no spending cap, only usage notifications, so stay on Free unless
you mean to pay. Check the plan under **Workers & Pages → Plans**.

The game is built to stay well inside the free allowances:

- **Clients ask rarely.** The clock is read about every two minutes and carried
  forward locally; a new board is fetched once when a round starts, and only by
  players waiting to play. Progress is reported on joining and then at most once a
  minute. The table is read four times over a scorecard.
- **The Worker saves work.** Packs are cached in memory for ten minutes rather than
  read from KV on every request, and repeated table reads within a second and a
  half are answered from memory.
- **Writes are few.** A scored round writes to the player's Durable Object, not
  KV; KV is written only when a name is claimed or changed. The leaderboard skips
  progress that changes nothing or arrives within eight seconds of the last.

Roughly, one player-hour costs about 200 requests, so the 100,000-request daily
allowance covers several hundred player-hours. **Uploading every pack costs 130 KV
writes** of the free 1,000 a day: fine once or twice a day, not in a loop.
