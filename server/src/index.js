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
 */

const CYCLE = (env) => num(env.ROUND_SECONDS, 180) + num(env.RESULTS_SECONDS, 60);
const num = (value, fallback) => {
  const n = parseInt(value, 10);
  return Number.isFinite(n) ? n : fallback;
};

/** Scoring must match the client exactly, or the scorecard will lie. */
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

/** The board for a round, from the uploaded packs. */
async function loadRound(env, round) {
  const packCount = num(env.PACK_COUNT, 10);
  const packSize = num(env.PACK_SIZE, 100);
  const slot = round % (packCount * packSize);
  const pack = await env.ROUNDS.get(`pack:${Math.floor(slot / packSize)}`, "json");
  if (!pack) return null;
  const entry = pack[slot % packSize];
  return entry ? { grid: entry.grid, theme: entry.theme, words: new Set(entry.words) } : null;
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

  await env.ROUNDS.put(key, id);
  await env.ROUNDS.put(
    `player:${id}`,
    JSON.stringify({ name, recent: previous?.recent ?? [] })
  );
  return json(request, env, { ok: true, name });
}

/**
 * Submit a round. The score is computed here from the words, never taken from
 * the client, and a word only counts if it is genuinely on that board.
 */
async function postScore(request, env, body) {
  const { id, round, words } = body;
  if (!ID_RE.test(id || "")) return fail(request, env, "bad id");
  if (!Number.isInteger(round)) return fail(request, env, "bad round");
  if (!Array.isArray(words)) return fail(request, env, "bad words");

  const now = schedule(env, Date.now() / 1000);
  // A round can only be submitted while it is current: not early, not later.
  if (round !== now.round) {
    return fail(request, env, "that round is not current", 409);
  }

  const player = await env.ROUNDS.get(`player:${id}`, "json");
  if (!player) return fail(request, env, "unknown player, claim a name first", 403);

  const board = await loadRound(env, round);
  if (!board) return fail(request, env, "no board for that round", 503);

  let score = 0;
  const accepted = [];
  const rejected = [];
  const seen = new Set();
  for (const raw of words.slice(0, 500)) {
    const word = String(raw || "").toLowerCase();
    if (seen.has(word)) continue; // a word scores once
    seen.add(word);
    if (board.words.has(word)) {
      score += wordPoints(word.length);
      accepted.push(word);
    } else {
      rejected.push(word);
    }
  }

  const key = `round:${round}`;
  const table = (await env.ROUNDS.get(key, "json")) || [];
  const existing = table.findIndex((row) => row.id === id);
  const row = { id, name: player.name, score, words: accepted.length };
  if (existing >= 0) table[existing] = row;
  else table.push(row);
  table.sort((a, b) => b.score - a.score);
  await env.ROUNDS.put(key, JSON.stringify(table), { expirationTtl: 60 * 60 * 24 * 7 });

  // Rank is the average of the last ten rounds, same as the client shows.
  const recent = [...(player.recent ?? []), score].slice(-10);
  await env.ROUNDS.put(`player:${id}`, JSON.stringify({ ...player, recent }));

  const average = Math.round(recent.reduce((a, b) => a + b, 0) / recent.length);
  return json(request, env, { score, accepted: accepted.length, rejected, average });
}

/** The table for a round, names and scores only. */
async function getLeaderboard(request, env, url) {
  const round = parseInt(url.searchParams.get("round") ?? "", 10);
  const target = Number.isFinite(round) ? round : schedule(env, Date.now() / 1000).round;
  const table = (await env.ROUNDS.get(`round:${target}`, "json")) || [];
  // Ids stay server-side: an id is the account, and a leaderboard is public.
  return json(request, env, {
    round: target,
    entries: table.slice(0, 50).map(({ name, score, words }) => ({ name, score, words })),
  });
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
      if (request.method === "POST") {
        const body = await request.json().catch(() => null);
        if (!body) return fail(request, env, "expected JSON");
        if (url.pathname === "/claim") return await postClaim(request, env, body);
        if (url.pathname === "/score") return await postScore(request, env, body);
      }
      return fail(request, env, "not found", 404);
    } catch (err) {
      // Never leak internals to a caller.
      console.error(err);
      return fail(request, env, "server error", 500);
    }
  },
};
