/**
 * Word Legend server.
 *
 * Everyone plays the same board at the same time. The clock is derived from the
 * wall clock rather than stored, so any number of Worker instances agree without
 * coordinating: round N runs from EPOCH + N*cycle.
 *
 * Scores are checked, not trusted. Each round's board and its full set of valid
 * words are precomputed by the Rust generator (which owns the lexicon) and
 * uploaded as packs; the Worker scores a submission by looking each word up in
 * that set. A browser cannot invent a score, because it would have to invent
 * words that are genuinely on the board.
 *
 * Each round's leaderboard lives in its own Durable Object (see leaderboard.js),
 * not in KV: scores for a round all land at once, and they must all be on the
 * table, and visible to everyone, straight away.
 */

export { Leaderboard } from "./leaderboard.js";
export { Player } from "./player.js";
export { Standings } from "./standings.js";

/** The all-time table. One object holds every player's row. */
const standings = (env) => env.STANDINGS.getByName("standings");

/** A player's banked scores. */
const playerScores = (env, id) => env.PLAYER.getByName(`player:${id}`);

/**
 * Round packs never change once uploaded, so each isolate keeps the ones it has
 * read for a while instead of reading KV on every request. Ten minutes also lets
 * a fresh upload through without a redeploy.
 */
const PACK_CACHE_MS = 10 * 60 * 1000;
/** Per KV binding, so a cached pack is always one from the namespace asked. */
const packCaches = new WeakMap();

/** The cache belonging to a binding, made on first use. */
function cacheFor(caches, binding) {
  let cache = caches.get(binding);
  if (!cache) {
    cache = new Map();
    caches.set(binding, cache);
  }
  return cache;
}

async function readPack(env, key) {
  const cache = cacheFor(packCaches, env.ROUNDS);
  const hit = cache.get(key);
  if (hit && Date.now() - hit.at < PACK_CACHE_MS) return hit.pack;
  const pack = await env.ROUNDS.get(key, "json");
  cache.set(key, { at: Date.now(), pack });
  return pack;
}

/**
 * Many players read the same table in the same second as a round ends. An
 * isolate answers repeats from memory for a moment, and forgets a round's table
 * as soon as a score for it is filed here.
 */
const TABLE_CACHE_MS = 1500;
const tableCaches = new WeakMap();

/**
 * The all-time table only changes as rounds end, so an isolate keeps its answer
 * for a while: it is read from the stats page, where a few seconds' age is fine.
 */
const STANDINGS_CACHE_MS = 30 * 1000;
const standingsCache = new WeakMap();

/** The Durable Object holding a round's leaderboard. */
const leaderboard = (env, round) => env.LEADERBOARD.getByName(`round:${round}`);

const CYCLE = (env) => num(env.ROUND_SECONDS, 180) + num(env.RESULTS_SECONDS, 30);
const num = (value, fallback) => {
  const n = parseInt(value, 10);
  return Number.isFinite(n) ? n : fallback;
};

/**
 * What a word scores. This must match the client's `score_word` exactly, or the
 * scorecard and the leaderboard will disagree: a superword (sixteen tiles, "qu"
 * being one) scores a flat bonus; anything else its length's points, with 10%
 * more for an obscure word.
 */
const SUPERWORD_POINTS = 20000;
function scoreWord(word, obscure) {
  const tiles = word.length - (word.match(/qu/g) || []).length;
  if (tiles >= 16) return SUPERWORD_POINTS;
  const points = wordPoints(word.length);
  return obscure ? (points * 11) / 10 : points;
}

/**
 * The letter bonus, matching the client's `letter_bonus`: 25 for each tile used
 * in a found word, 100 more for each used in two or more, and 500 for using every
 * tile. It follows the paths the player traced, each checked against the board.
 */
const LETTER_USED_BONUS = 25;
const LETTER_REUSED_BONUS = 100;
const FULL_BOARD_BONUS = 500;

/** A path is the tiles, as indices 0-15, a word was traced through. */
function pathSpells(grid, word, path) {
  if (!Array.isArray(path) || path.length === 0 || path.length > 16) return false;
  const seen = new Set();
  let spelled = "";
  for (let i = 0; i < path.length; i++) {
    const tile = path[i];
    if (!Number.isInteger(tile) || tile < 0 || tile > 15 || seen.has(tile)) return false;
    seen.add(tile);
    if (i > 0) {
      const prev = path[i - 1];
      const dr = Math.abs(Math.floor(tile / 4) - Math.floor(prev / 4));
      const dc = Math.abs((tile % 4) - (prev % 4));
      if (dr > 1 || dc > 1) return false;
    }
    spelled += grid[tile];
  }
  return spelled === word;
}

function letterBonus(uses) {
  const used = uses.filter((u) => u >= 1).length;
  const reused = uses.filter((u) => u >= 2).length;
  return used * LETTER_USED_BONUS + reused * LETTER_REUSED_BONUS + (used === 16 ? FULL_BOARD_BONUS : 0);
}

function wordPoints(length) {
  if (length <= 2) return 0;
  if (length === 3) return 100;
  if (length === 4) return 400;
  if (length === 5) return 800;
  if (length === 6) return 1400;
  if (length === 7) return 1800;
  if (length === 8) return 2200;
  return 2200 + 400 * (length - 8);
}

/** Rounds tick from a fixed epoch so every client lands on the same one. */
const EPOCH = Date.UTC(2026, 0, 1) / 1000;

function schedule(env, nowSeconds) {
  const cycle = CYCLE(env);
  const play = num(env.ROUND_SECONDS, 180);
  const elapsed = Math.max(0, nowSeconds - EPOCH);
  const round = Math.floor(elapsed / cycle);
  const into = elapsed - round * cycle;
  const playing = into < play;
  return {
    round,
    phase: playing ? "play" : "results",
    secondsLeft: Math.ceil(playing ? play - into : cycle - into),
    serverTime: Math.floor(nowSeconds),
  };
}

/** The month a round is played in, 1 to 12, by when it starts (UTC). */
function roundMonth(env, round) {
  return new Date((EPOCH + round * CYCLE(env)) * 1000).getUTCMonth() + 1;
}

/**
 * The board for a round, from the uploaded packs. Each month has its own rounds,
 * favouring that month's seasonal theme; a month not uploaded falls back to the
 * default rounds.
 */
async function loadRound(env, round) {
  const packCount = num(env.PACK_COUNT, 10);
  const packSize = num(env.PACK_SIZE, 100);
  const slot = round % (packCount * packSize);
  const index = Math.floor(slot / packSize);
  const month = String(roundMonth(env, round)).padStart(2, "0");
  const pack = (await readPack(env, `pack:m${month}:${index}`)) ?? (await readPack(env, `pack:${index}`));
  if (!pack) return null;
  const entry = pack[slot % packSize];
  if (!entry) return null;
  // Packs list common words in `words` and obscure ones in `obscure`. Older packs
  // put both tiers in `words`; everything there scores as common.
  const obscure = new Set(entry.obscure ?? []);
  return { grid: entry.grid, theme: entry.theme, words: new Set([...entry.words, ...obscure]), obscure };
}

// --- responses --------------------------------------------------------------

function corsHeaders(request, env) {
  const allowed = (env.ALLOWED_ORIGINS || "").split(",").map((s) => s.trim());
  const origin = request.headers.get("Origin") || "";
  return {
    "Access-Control-Allow-Origin": allowed.includes(origin) ? origin : allowed[0] || "",
    "Access-Control-Allow-Methods": "GET,POST,OPTIONS",
    "Access-Control-Allow-Headers": "Content-Type",
    "Access-Control-Max-Age": "86400",
  };
}

const json = (request, env, body, status = 200) =>
  new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json", ...corsHeaders(request, env) },
  });

const fail = (request, env, message, status = 400) =>
  json(request, env, { error: message }, status);

// --- validation -------------------------------------------------------------

const NAME_RE = /^[A-Za-z0-9_-]{3,16}$/;
const ID_RE = /^[0-9A-HJ-NP-TV-Z]{16}$/; // Crockford base32, as the client mints it

// --- routes -----------------------------------------------------------------

/** Where we are in the cycle, and the board if a round is under way. */
async function getRound(request, env) {
  const now = schedule(env, Date.now() / 1000);
  const round = await loadRound(env, now.round);
  if (!round) {
    return json(request, env, { ...now, board: null, error: "no rounds uploaded" });
  }
  // The word list is never sent: clients work it out themselves, and handing it
  // over would turn every round into a copying exercise.
  return json(request, env, { ...now, board: { grid: round.grid, theme: round.theme } });
}

/** Claim a display name. First to take it keeps it. */
async function postClaim(request, env, body) {
  const { id, name } = body;
  if (!ID_RE.test(id || "")) return fail(request, env, "bad id");
  if (!NAME_RE.test(name || "")) return fail(request, env, "bad name");

  const key = `name:${name.toLowerCase()}`;
  const owner = await env.ROUNDS.get(key);
  if (owner && owner !== id) {
    return fail(request, env, "That name is taken", 409);
  }

  const previous = await env.ROUNDS.get(`player:${id}`, "json");
  if (previous && previous.name && previous.name.toLowerCase() !== name.toLowerCase()) {
    await env.ROUNDS.delete(`name:${previous.name.toLowerCase()}`);
  }

  // Only a new or changed name costs KV writes; claiming the same name again, as
  // the game does each time it starts, costs none.
  if (owner !== id) await env.ROUNDS.put(key, id);
  if (!previous || previous.name !== name) {
    await env.ROUNDS.put(`player:${id}`, JSON.stringify({ ...(previous ?? {}), name }));
  }
  return json(request, env, { ok: true, name });
}

/**
 * Check a submission and score it against the round's board. Returns either
 * `{ error }` as a ready Response, or what was scored. The score is computed here
 * from the words, never taken from the client, and a word only counts if it is
 * genuinely on that board.
 */
async function scoreSubmission(request, env, body, phase) {
  const { id, round, words } = body;
  if (!ID_RE.test(id || "")) return { error: fail(request, env, "bad id") };
  if (!Number.isInteger(round)) return { error: fail(request, env, "bad round") };
  if (!Array.isArray(words)) return { error: fail(request, env, "bad words") };

  const now = schedule(env, Date.now() / 1000);
  // A round can only be submitted while it is current: not early, not later.
  if (round !== now.round || (phase && now.phase !== phase)) {
    return { error: fail(request, env, "that round is not current", 409) };
  }

  const player = await env.ROUNDS.get(`player:${id}`, "json");
  if (!player) return { error: fail(request, env, "unknown player, claim a name first", 403) };

  const board = await loadRound(env, round);
  if (!board) return { error: fail(request, env, "no board for that round", 503) };

  let score = 0;
  const accepted = [];
  const rejected = [];
  const seen = new Set();
  const paths = Array.isArray(body.paths) ? body.paths : [];
  const uses = new Array(16).fill(0);
  const grid = board.grid.map((t) => String(t).toLowerCase());
  words.slice(0, 500).forEach((raw, i) => {
    const word = String(raw || "").toLowerCase();
    if (seen.has(word)) return; // a word scores once
    seen.add(word);
    if (board.words.has(word)) {
      score += scoreWord(word, board.obscure.has(word));
      accepted.push(word);
      // Its tiles count toward the letter bonus only if its path really spells it.
      if (pathSpells(grid, word, paths[i])) {
        for (const tile of paths[i]) uses[tile] += 1;
      }
    } else {
      rejected.push(word);
    }
  });
  const bonus = letterBonus(uses);
  score += bonus;
  return { id, round, player, score, bonus, accepted, rejected };
}

/** The league a player's game reported, if it is one of the six. */
function leagueOf(body) {
  return Number.isInteger(body.league) && body.league >= 0 && body.league <= 5 ? body.league : null;
}

/**
 * Report progress while a round is being played: the player is on the round's
 * leaderboard from the moment they join, and their score keeps pace, so when
 * the round ends everyone who played is already there. Nothing is banked.
 */
async function postProgress(request, env, body) {
  const result = await scoreSubmission(request, env, body, "play");
  if (result.error) return result.error;
  const { id, round, player, score, accepted } = result;
  await leaderboard(env, round).submit(id, player.name, score, accepted.length, false, leagueOf(body));
  cacheFor(tableCaches, env.LEADERBOARD).delete(round);
  return json(request, env, { score, accepted: accepted.length });
}

/** Hand in a finished round: the final score, and the one that is banked. */
async function postScore(request, env, body) {
  const result = await scoreSubmission(request, env, body);
  if (result.error) return result.error;
  const { id, round, player, score, bonus, accepted, rejected } = result;

  await leaderboard(env, round).submit(id, player.name, score, accepted.length, true, leagueOf(body));
  cacheFor(tableCaches, env.LEADERBOARD).delete(round);

  // Rank is the average of the last ten rounds, same as the client shows. A round
  // with nothing found was not played -- it is on the leaderboard as a zero, but
  // it is not banked, or leaving a tab open would drag the average down. Scores a
  // player had in KV from before seed their Player object the first time.
  const { recent, best, games } = await playerScores(env, id).bank(score, player.recent ?? []);

  const average = recent.length ? Math.round(recent.reduce((a, b) => a + b, 0) / recent.length) : 0;
  // The all-time table follows the same average the league does.
  if (score > 0) {
    await standings(env).record(id, player.name, leagueOf(body), average, best, games);
    standingsCache.delete(env.STANDINGS);
  }
  return json(request, env, { score, bonus, accepted: accepted.length, rejected, average });
}

/** The all-time table: every player's league, average, best score and games. */
async function getStandings(request, env, url) {
  const asked = parseInt(url.searchParams.get("limit") ?? "", 10);
  const limit = Number.isFinite(asked) ? Math.min(Math.max(asked, 1), 100) : 50;
  const cached = standingsCache.get(env.STANDINGS);
  if (cached && cached.limit >= limit && Date.now() - cached.at < STANDINGS_CACHE_MS) {
    return json(request, env, { entries: cached.entries.slice(0, limit) });
  }
  const entries = await standings(env).table(limit);
  standingsCache.set(env.STANDINGS, { at: Date.now(), limit, entries });
  return json(request, env, { entries });
}

/** The table for a round, names and scores only. */
async function getLeaderboard(request, env, url) {
  const round = parseInt(url.searchParams.get("round") ?? "", 10);
  const target = Number.isFinite(round) ? round : schedule(env, Date.now() / 1000).round;
  // Ids stay inside the Durable Object: an id is the account, and a leaderboard
  // is public.
  const tableCache = cacheFor(tableCaches, env.LEADERBOARD);
  const cached = tableCache.get(target);
  if (cached && Date.now() - cached.at < TABLE_CACHE_MS) {
    return json(request, env, { round: target, entries: cached.entries });
  }
  const entries = await leaderboard(env, target).table(50);
  tableCache.set(target, { at: Date.now(), entries });
  return json(request, env, { round: target, entries });
}

export default {
  async fetch(request, env) {
    const url = new URL(request.url);

    if (request.method === "OPTIONS") {
      return new Response(null, { status: 204, headers: corsHeaders(request, env) });
    }

    try {
      if (url.pathname === "/round" && request.method === "GET") {
        return await getRound(request, env);
      }
      if (url.pathname === "/leaderboard" && request.method === "GET") {
        return await getLeaderboard(request, env, url);
      }
      if (url.pathname === "/standings" && request.method === "GET") {
        return await getStandings(request, env, url);
      }
      if (request.method === "POST") {
        const body = await request.json().catch(() => null);
        if (!body) return fail(request, env, "expected JSON");
        if (url.pathname === "/claim") return await postClaim(request, env, body);
        if (url.pathname === "/score") return await postScore(request, env, body);
        if (url.pathname === "/progress") return await postProgress(request, env, body);
      }
      return fail(request, env, "not found", 404);
    } catch (err) {
      // Never leak internals to a caller.
      console.error(err);
      return fail(request, env, "server error", 500);
    }
  },
};
