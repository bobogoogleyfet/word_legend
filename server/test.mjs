// Worker tests. Run with: node test.mjs
import worker from "./src/index.js";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const pack = JSON.parse(readFileSync(process.argv[2], "utf8"));

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

const ORIGIN = "https://bobogoogleyfet.github.io";
const env = () => ({
  ROUNDS: makeKV({ "pack:0": JSON.stringify(pack) }),
  ROUND_SECONDS: "180", RESULTS_SECONDS: "60",
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
  const ok = await body(await call(e, "/claim", { method: "POST", body: JSON.stringify({ id: ID, name: "tanner" }) }));
  assert.equal(ok.ok, true);

  const clash = await call(e, "/claim", { method: "POST", body: JSON.stringify({ id: ID2, name: "TANNER" }) });
  assert.equal(clash.status, 409, "a second player took a taken name");

  const again = await call(e, "/claim", { method: "POST", body: JSON.stringify({ id: ID, name: "tanner" }) });
  assert.equal(again.status, 200, "the owner should be able to re-claim");
});

await test("malformed ids and names are refused", async () => {
  const e = env();
  for (const bad of [{ id: "nope", name: "tanner" }, { id: ID, name: "a" }, { id: ID, name: "has space" }]) {
    const r = await call(e, "/claim", { method: "POST", body: JSON.stringify(bad) });
    assert.equal(r.status, 400, `accepted ${JSON.stringify(bad)}`);
  }
});

// The board for the current round, so we can submit words that are really on it.
const current = pack[roundInfo.round % pack.length];
const realWords = current.words.slice(0, 5);

await test("real words score, and the score is computed server-side", async () => {
  const e = env();
  await call(e, "/claim", { method: "POST", body: JSON.stringify({ id: ID, name: "tanner" }) });
  const res = await body(await call(e, "/score", {
    method: "POST",
    // Note the bogus `score`: the server must ignore whatever the client claims.
    body: JSON.stringify({ id: ID, round: roundInfo.round, words: realWords, score: 999999 }),
  }));
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
  await call(e, "/claim", { method: "POST", body: JSON.stringify({ id: ID, name: "tanner" }) });
  const res = await body(await call(e, "/score", {
    method: "POST",
    body: JSON.stringify({ id: ID, round: roundInfo.round, words: ["zzzzzzzz", "qqqqqqqqq", "notaword"] }),
  }));
  assert.equal(res.score, 0, "a forged submission scored");
  assert.equal(res.rejected.length, 3);
});

await test("a word cannot be submitted twice for double points", async () => {
  const e = env();
  await call(e, "/claim", { method: "POST", body: JSON.stringify({ id: ID, name: "tanner" }) });
  const one = await body(await call(e, "/score", { method: "POST", body: JSON.stringify({ id: ID, round: roundInfo.round, words: [realWords[0]] }) }));
  const twice = await body(await call(e, "/score", { method: "POST", body: JSON.stringify({ id: ID, round: roundInfo.round, words: [realWords[0], realWords[0]] }) }));
  assert.equal(twice.score, one.score);
});

await test("a stale round is refused", async () => {
  const e = env();
  await call(e, "/claim", { method: "POST", body: JSON.stringify({ id: ID, name: "tanner" }) });
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
  await call(e, "/claim", { method: "POST", body: JSON.stringify({ id: ID, name: "tanner" }) });
  await call(e, "/score", { method: "POST", body: JSON.stringify({ id: ID, round: roundInfo.round, words: realWords }) });
  const table = await body(await call(e, `/leaderboard?round=${roundInfo.round}`));
  assert.equal(table.entries.length, 1);
  assert.equal(table.entries[0].name, "tanner");
  assert.equal(table.entries[0].id, undefined, "an account id leaked onto the leaderboard");
  assert.ok(!JSON.stringify(table).includes(ID), "an account id leaked onto the leaderboard");
});

await test("rank is the average of the last ten rounds", async () => {
  const e = env();
  await call(e, "/claim", { method: "POST", body: JSON.stringify({ id: ID, name: "tanner" }) });
  const res = await body(await call(e, "/score", { method: "POST", body: JSON.stringify({ id: ID, round: roundInfo.round, words: realWords }) }));
  const player = await e.ROUNDS.get(`player:${ID}`, "json");
  assert.deepEqual(player.recent, [res.score]);
  assert.equal(res.average, res.score);
});

await test("unknown routes 404", async () => {
  assert.equal((await call(env(), "/nope")).status, 404);
});

console.log(`\n${passed} passed`);
