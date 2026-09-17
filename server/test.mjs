// Worker tests. Run with: node test.mjs
import worker, { Leaderboard } from "./src/index.js";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const pack = JSON.parse(readFileSync(process.argv[2], "utf8"));

// Pin the clock ten seconds into a round, so tests that depend on the phase run
// the same whenever they are run.
const EPOCH_MS = Date.UTC(2026, 0, 1);
const CYCLE_MS = (180 + 30) * 1000;
const PINNED_NOW = EPOCH_MS + 40_000 * CYCLE_MS + 10_000;
Date.now = () => PINNED_NOW;

/** Minimal stand-in for Workers KV. */
function makeKV(seed = {}) {
  const store = new Map(Object.entries(seed));
  return {
    store,
    async get(key, type) {
      const v = store.get(key);
      if (v === undefined) return null;
      return type === "json" ? JSON.parse(v) : v;
    },
    async put(key, value) { store.set(key, value); },
    async delete(key) { store.delete(key); },
  };
}

/**
 * Stand-in for a Durable Object namespace: one instance per name, each with its
 * own storage, called directly the way RPC calls it. Every call is awaited
 * through a promise, as it would be across the RPC boundary.
 */
function makeNamespace(Class) {
  const instances = new Map();
  return {
    getByName(name) {
      if (!instances.has(name)) {
        const store = new Map();
        const storage = {
          async put(key, value) { store.set(key, structuredClone(value)); },
          async get(key) { return structuredClone(store.get(key)); },
          async list({ prefix = "" } = {}) {
            return new Map([...store].filter(([k]) => k.startsWith(prefix)).sort(([a], [b]) => a.localeCompare(b)));
          },
          async deleteAll() { store.clear(); },
          async setAlarm() {},
        };
        instances.set(name, new Class({ storage }, {}));
      }
      const target = instances.get(name);
      return new Proxy(target, {
        get: (obj, prop) => (...args) => Promise.resolve().then(() => obj[prop](...args)),
      });
    },
  };
}

const ORIGIN = "https://bobogoogleyfet.github.io";
const env = () => ({
  ROUNDS: makeKV({ "pack:0": JSON.stringify(pack) }),
  LEADERBOARD: makeNamespace(Leaderboard),
  ROUND_SECONDS: "180", RESULTS_SECONDS: "30",
  PACK_COUNT: "1", PACK_SIZE: String(pack.length),
  ALLOWED_ORIGINS: ORIGIN,
});

const call = (e, path, init = {}) =>
  worker.fetch(new Request(`https://x${path}`, {
    ...init,
    headers: { Origin: ORIGIN, "Content-Type": "application/json", ...(init.headers || {}) },
  }), e);

const body = (r) => r.json();
let passed = 0;
async function test(name, fn) {
  try { await fn(); console.log(`  ok   ${name}`); passed++; }
  catch (e) { console.log(`  FAIL ${name}\n       ${e.message}`); process.exitCode = 1; }
}

console.log("worker:");

const e0 = env();
const roundInfo = await body(await call(e0, "/round"));

await test("a round exposes a board but never its word list", async () => {
  assert.ok(Number.isInteger(roundInfo.round));
  assert.equal(roundInfo.board.grid.length, 16);
  assert.ok(["play", "results"].includes(roundInfo.phase));
  assert.ok(roundInfo.secondsLeft > 0 && roundInfo.secondsLeft <= 240);
  assert.equal(roundInfo.board.words, undefined, "word list must not be sent to clients");
});

await test("cors is restricted to the configured origin", async () => {
  const r = await call(e0, "/round");
  assert.equal(r.headers.get("Access-Control-Allow-Origin"), ORIGIN);
});

const ID = "0123456789ABCDEF";
const ID2 = "FEDCBA9876543210";

await test("a name can be claimed once", async () => {
  const e = env();
  const ok = await body(await call(e, "/claim", { method: "POST", body: JSON.stringify({ id: ID, name: "wordfan" }) }));
  assert.equal(ok.ok, true);

  const clash = await call(e, "/claim", { method: "POST", body: JSON.stringify({ id: ID2, name: "WORDFAN" }) });
  assert.equal(clash.status, 409, "a second player took a taken name");

  const again = await call(e, "/claim", { method: "POST", body: JSON.stringify({ id: ID, name: "wordfan" }) });
  assert.equal(again.status, 200, "the owner should be able to re-claim");
});

await test("malformed ids and names are refused", async () => {
  const e = env();
  for (const bad of [{ id: "nope", name: "wordfan" }, { id: ID, name: "a" }, { id: ID, name: "has space" }]) {
    const r = await call(e, "/claim", { method: "POST", body: JSON.stringify(bad) });
    assert.equal(r.status, 400, `accepted ${JSON.stringify(bad)}`);
  }
});

// The board for the current round, so we can submit words that are really on it.
const current = pack[roundInfo.round % pack.length];
const realWords = current.words.slice(0, 5);

await test("real words score, and the score is computed server-side", async () => {
  const e = env();
  await call(e, "/claim", { method: "POST", body: JSON.stringify({ id: ID, name: "wordfan" }) });
  const res = await body(await call(e, "/score", {
    method: "POST",
    // Note the bogus `score`: the server must ignore whatever the client claims.
    body: JSON.stringify({ id: ID, round: roundInfo.round, words: realWords, score: 999999 }),
  }));
  // realWords come from `words`, the common tier: plain length points.
  const expected = realWords.reduce((sum, w) => {
    const n = w.length;
    const pts = n <= 2 ? 0 : n === 3 ? 100 : n === 4 ? 400 : n === 5 ? 800
      : n === 6 ? 1400 : n === 7 ? 1800 : n === 8 ? 2200 : 2200 + 400 * (n - 8);
    return sum + pts;
  }, 0);
  assert.equal(res.score, expected, "server score should ignore the client's number");
  assert.notEqual(res.score, 999999);
});

await test("invented words score nothing", async () => {
  const e = env();
  await call(e, "/claim", { method: "POST", body: JSON.stringify({ id: ID, name: "wordfan" }) });
  const res = await body(await call(e, "/score", {
    method: "POST",
    body: JSON.stringify({ id: ID, round: roundInfo.round, words: ["zzzzzzzz", "qqqqqqqqq", "notaword"] }),
  }));
  assert.equal(res.score, 0, "a forged submission scored");
  assert.equal(res.rejected.length, 3);
});

await test("a word cannot be submitted twice for double points", async () => {
  const e = env();
  await call(e, "/claim", { method: "POST", body: JSON.stringify({ id: ID, name: "wordfan" }) });
  const one = await body(await call(e, "/score", { method: "POST", body: JSON.stringify({ id: ID, round: roundInfo.round, words: [realWords[0]] }) }));
  const twice = await body(await call(e, "/score", { method: "POST", body: JSON.stringify({ id: ID, round: roundInfo.round, words: [realWords[0], realWords[0]] }) }));
  assert.equal(twice.score, one.score);
});

await test("a stale round is refused", async () => {
  const e = env();
  await call(e, "/claim", { method: "POST", body: JSON.stringify({ id: ID, name: "wordfan" }) });
  const r = await call(e, "/score", { method: "POST", body: JSON.stringify({ id: ID, round: roundInfo.round - 5, words: realWords }) });
  assert.equal(r.status, 409);
});

await test("scores need a claimed name", async () => {
  const e = env();
  const r = await call(e, "/score", { method: "POST", body: JSON.stringify({ id: ID2, round: roundInfo.round, words: realWords }) });
  assert.equal(r.status, 403);
});

await test("the leaderboard shows names but never account ids", async () => {
  const e = env();
  await call(e, "/claim", { method: "POST", body: JSON.stringify({ id: ID, name: "wordfan" }) });
  await call(e, "/score", { method: "POST", body: JSON.stringify({ id: ID, round: roundInfo.round, words: realWords }) });
  const table = await body(await call(e, `/leaderboard?round=${roundInfo.round}`));
  assert.equal(table.entries.length, 1);
  assert.equal(table.entries[0].name, "wordfan");
  assert.equal(table.entries[0].id, undefined, "an account id leaked onto the leaderboard");
  assert.ok(!JSON.stringify(table).includes(ID), "an account id leaked onto the leaderboard");
});

/** An env whose every round is this one board. */
const envWith = (entry) => ({ ...env(), ROUNDS: makeKV({ "pack:0": JSON.stringify([entry]) }), PACK_SIZE: "1" });

await test("obscure words score 10% more, and a superword scores its bonus", async () => {
  const e = envWith({
    grid: Array(16).fill("a"),
    theme: null,
    words: ["cat", "cats", "characterization"],
    obscure: ["adit", "adits"],
  });
  await call(e, "/claim", { method: "POST", body: JSON.stringify({ id: ID, name: "wordfan" }) });
  const score = async (words) =>
    (await body(await call(e, "/score", { method: "POST", body: JSON.stringify({ id: ID, round: roundInfo.round, words }) }))).score;
  assert.equal(await score(["cat"]), 100);
  assert.equal(await score(["adit"]), 440, "an obscure four-letter word is 400 + 10%");
  assert.equal(await score(["adits"]), 880);
  assert.equal(await score(["characterization"]), 20000, "a superword scores its bonus");
  assert.equal(await score(["cats", "adits"]), 400 + 880);
});

await test("an older pack, with both tiers in words, still scores everything as common", async () => {
  const e = envWith({ grid: Array(16).fill("a"), theme: null, words: ["cat", "adit"] });
  await call(e, "/claim", { method: "POST", body: JSON.stringify({ id: ID, name: "wordfan" }) });
  const res = await body(await call(e, "/score", { method: "POST", body: JSON.stringify({ id: ID, round: roundInfo.round, words: ["cat", "adit"] }) }));
  assert.equal(res.score, 500);
});

await test("players finishing at the same moment all reach the leaderboard", async () => {
  // Everyone's round ends on the same tick, so submissions really do arrive
  // together. A read-modify-write table loses all but one of them.
  const e = env();
  const players = ["0123456789ABCDEF", "FEDCBA9876543210", "0000111122223333", "ABCDABCDABCDABCD"];
  for (const [i, id] of players.entries()) {
    await call(e, "/claim", { method: "POST", body: JSON.stringify({ id, name: `player${i}` }) });
  }
  await Promise.all(players.map((id, i) =>
    call(e, "/score", { method: "POST", body: JSON.stringify({ id, round: roundInfo.round, words: realWords.slice(0, i + 1) }) })
  ));
  const table = await body(await call(e, `/leaderboard?round=${roundInfo.round}`));
  assert.equal(table.entries.length, players.length, `only ${table.entries.map((r) => r.name)} made the table`);
  const scores = table.entries.map((r) => r.score);
  assert.deepEqual(scores, [...scores].sort((a, b) => b - a), "the table is not best-first");
});

await test("a resubmission replaces the player's row rather than adding one", async () => {
  const e = env();
  await call(e, "/claim", { method: "POST", body: JSON.stringify({ id: ID, name: "wordfan" }) });
  for (const words of [realWords.slice(0, 1), realWords]) {
    await call(e, "/score", { method: "POST", body: JSON.stringify({ id: ID, round: roundInfo.round, words }) });
  }
  const table = await body(await call(e, `/leaderboard?round=${roundInfo.round}`));
  assert.equal(table.entries.length, 1);
  assert.equal(table.entries[0].words, realWords.length);
});

await test("progress puts a player on the table at once, without banking anything", async () => {
  const e = env();
  await call(e, "/claim", { method: "POST", body: JSON.stringify({ id: ID, name: "wordfan" }) });
  await call(e, "/claim", { method: "POST", body: JSON.stringify({ id: ID2, name: "latecomer" }) });

  // Both join; one has found a couple of words already.
  const joined = await call(e, "/progress", { method: "POST", body: JSON.stringify({ id: ID, round: roundInfo.round, words: realWords.slice(0, 2) }) });
  assert.equal(roundInfo.phase, "play", "the test clock should be mid-round");
  assert.equal(joined.status, 200);
  await call(e, "/progress", { method: "POST", body: JSON.stringify({ id: ID2, round: roundInfo.round, words: [] }) });

  let table = await body(await call(e, `/leaderboard?round=${roundInfo.round}`));
  assert.deepEqual(table.entries.map((r) => [r.name, r.final]).sort(), [["latecomer", false], ["wordfan", false]]);
  assert.deepEqual((await e.ROUNDS.get(`player:${ID}`, "json")).recent, [], "progress was banked");

  // The final score replaces the progress row and is the one banked.
  const done = await body(await call(e, "/score", { method: "POST", body: JSON.stringify({ id: ID, round: roundInfo.round, words: realWords }) }));
  table = await body(await call(e, `/leaderboard?round=${roundInfo.round}`));
  const mine = table.entries.find((r) => r.name === "wordfan");
  assert.equal(mine.final, true);
  assert.equal(mine.score, done.score);
  assert.deepEqual((await e.ROUNDS.get(`player:${ID}`, "json")).recent, [done.score]);

  // A slow progress report landing after the final score does not undo it.
  await call(e, "/progress", { method: "POST", body: JSON.stringify({ id: ID, round: roundInfo.round, words: [] }) });
  table = await body(await call(e, `/leaderboard?round=${roundInfo.round}`));
  assert.equal(table.entries.find((r) => r.name === "wordfan").score, done.score, "late progress overwrote a final score");
});

await test("progress is refused once the round is over", async () => {
  const e = env();
  await call(e, "/claim", { method: "POST", body: JSON.stringify({ id: ID, name: "wordfan" }) });
  const later = Date.now;
  Date.now = () => PINNED_NOW + 175_000; // into the results window of the same round
  try {
    const r = await call(e, "/progress", { method: "POST", body: JSON.stringify({ id: ID, round: roundInfo.round, words: [] }) });
    assert.equal(r.status, 409, "progress was taken during results");
    const done = await call(e, "/score", { method: "POST", body: JSON.stringify({ id: ID, round: roundInfo.round, words: [] }) });
    assert.equal(done.status, 200, "the final score must still be taken during results");
  } finally {
    Date.now = later;
  }
});

await test("a round with nothing found is on the table but not banked", async () => {
  const e = env();
  await call(e, "/claim", { method: "POST", body: JSON.stringify({ id: ID, name: "wordfan" }) });
  const played = await body(await call(e, "/score", { method: "POST", body: JSON.stringify({ id: ID, round: roundInfo.round, words: realWords }) }));
  const e2 = env();
  await call(e2, "/claim", { method: "POST", body: JSON.stringify({ id: ID2, name: "idler" }) });
  const idle = await body(await call(e2, "/score", { method: "POST", body: JSON.stringify({ id: ID2, round: roundInfo.round, words: [] }) }));
  assert.equal(idle.score, 0);
  assert.equal(idle.average, 0);
  assert.deepEqual((await e2.ROUNDS.get(`player:${ID2}`, "json")).recent, [], "a zero was banked");
  const table = await body(await call(e2, `/leaderboard?round=${roundInfo.round}`));
  assert.deepEqual(table.entries.map((r) => [r.name, r.score]), [["idler", 0]], "the idle player is missing from the table");
  assert.ok(played.score > 0);
});

await test("rank is the average of the last ten rounds", async () => {
  const e = env();
  await call(e, "/claim", { method: "POST", body: JSON.stringify({ id: ID, name: "wordfan" }) });
  const res = await body(await call(e, "/score", { method: "POST", body: JSON.stringify({ id: ID, round: roundInfo.round, words: realWords }) }));
  const player = await e.ROUNDS.get(`player:${ID}`, "json");
  assert.deepEqual(player.recent, [res.score]);
  assert.equal(res.average, res.score);
});

await test("unknown routes 404", async () => {
  assert.equal((await call(env(), "/nope")).status, 404);
});

console.log(`\n${passed} passed`);
