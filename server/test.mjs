// Worker tests. Run with: node test.mjs
import worker, { Leaderboard, Player, Standings } from "./src/index.js";
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
          // The real storage also takes an object of several keys at once.
          async put(key, value) {
            if (key && typeof key === "object") {
              for (const [k, v] of Object.entries(key)) store.set(k, structuredClone(v));
              return;
            }
            store.set(key, structuredClone(value));
          },
          async get(key) { return structuredClone(store.get(key)); },
          async list({ prefix = "" } = {}) {
            return new Map([...store].filter(([k]) => k.startsWith(prefix)).sort(([a], [b]) => a.localeCompare(b)));
          },
          async deleteAll() { store.clear(); },
          async setAlarm(at) { store.set("__alarm", at); },
          async getAlarm() { return store.get("__alarm") ?? null; },
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
  PLAYER: makeNamespace(Player),
  STANDINGS: makeNamespace(Standings),
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

await test("the letter bonus follows the traced paths, and only valid ones", async () => {
  // C A T S across the top, E A R S under it.
  const grid = ["c", "a", "t", "s", "e", "a", "r", "s", "z", "z", "z", "z", "z", "z", "z", "z"];
  const e = envWith({ grid, theme: null, words: ["cat", "cats", "ears", "sat"], obscure: [] });
  await call(e, "/claim", { method: "POST", body: JSON.stringify({ id: ID, name: "wordfan" }) });
  const score = async (words, paths) =>
    (await body(await call(e, "/score", { method: "POST", body: JSON.stringify({ id: ID, round: roundInfo.round, words, paths }) }))).score;

  assert.equal(await score(["cats"]), 400, "no paths, no bonus");
  assert.equal(await score(["cats"], [[0, 1, 2, 3]]), 400 + 4 * 25, "four yellow stars");
  // CAT reuses C, A and T: three green stars.
  assert.equal(await score(["cats", "cat"], [[0, 1, 2, 3], [0, 1, 2]]), 400 + 100 + 4 * 25 + 3 * 100);
  // A path that does not spell its word, or jumps, earns nothing.
  assert.equal(await score(["cats"], [[4, 5, 6, 7]]), 400, "a path spelling EARS was counted for CATS");
  assert.equal(await score(["cats"], [[0, 1, 2, 15]]), 400, "a path that jumps across the board was counted");
});

await test("using every letter adds 500", async () => {
  const grid = "characterization".split("");
  const e = envWith({ grid, theme: null, words: ["characterization"], obscure: [] });
  await call(e, "/claim", { method: "POST", body: JSON.stringify({ id: ID, name: "wordfan" }) });
  // Snake across the rows so every step touches the last.
  const path = [0, 1, 2, 3, 7, 6, 5, 4, 8, 9, 10, 11, 15, 14, 13, 12];
  const snaked = path.map((i) => grid[i]).join("");
  const e2 = envWith({ grid, theme: null, words: [snaked], obscure: [] });
  await call(e2, "/claim", { method: "POST", body: JSON.stringify({ id: ID, name: "wordfan" }) });
  const res = await body(await call(e2, "/score", { method: "POST", body: JSON.stringify({ id: ID, round: roundInfo.round, words: [snaked], paths: [path] }) }));
  assert.equal(res.bonus, 16 * 25 + 500);
});

await test("the leaderboard shows each player's league", async () => {
  const e = env();
  await call(e, "/claim", { method: "POST", body: JSON.stringify({ id: ID, name: "wordfan" }) });
  await call(e, "/claim", { method: "POST", body: JSON.stringify({ id: ID2, name: "nobody" }) });
  await call(e, "/score", { method: "POST", body: JSON.stringify({ id: ID, round: roundInfo.round, words: realWords, league: 3 }) });
  await call(e, "/score", { method: "POST", body: JSON.stringify({ id: ID2, round: roundInfo.round, words: [], league: 99 }) });
  const table = await body(await call(e, `/leaderboard?round=${roundInfo.round}`));
  assert.equal(table.entries.find((r) => r.name === "wordfan").league, 3);
  assert.equal(table.entries.find((r) => r.name === "nobody").league, null, "an impossible league was stored");
});

await test("an older pack, with both tiers in words, still scores everything as common", async () => {
  const e = envWith({ grid: Array(16).fill("a"), theme: null, words: ["cat", "adit"] });
  await call(e, "/claim", { method: "POST", body: JSON.stringify({ id: ID, name: "wordfan" }) });
  const res = await body(await call(e, "/score", { method: "POST", body: JSON.stringify({ id: ID, round: roundInfo.round, words: ["cat", "adit"] }) }));
  assert.equal(res.score, 500);
});

await test("a month's own rounds are served when uploaded, the default ones otherwise", async () => {
  const month = String(new Date(Date.now()).getUTCMonth() + 1).padStart(2, "0");
  const board = (theme) => ({ grid: Array(16).fill("a"), theme, words: ["cat"], obscure: [] });
  const e = { ...env(), PACK_SIZE: "1" };
  e.ROUNDS = makeKV({ "pack:0": JSON.stringify([board("Animals")]) });
  assert.equal((await body(await call(e, "/round"))).board.theme, "Animals", "no default rounds served");

  e.ROUNDS = makeKV({ "pack:0": JSON.stringify([board("Animals")]), [`pack:m${month}:0`]: JSON.stringify([board("Halloween")]) });
  assert.equal((await body(await call(e, "/round"))).board.theme, "Halloween", "the month's rounds were not preferred");
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

await test("everyone who played a round is on its table", async () => {
  const e = env();
  const id = (i) => `${String(i).padStart(2, "0")}23456789ABCDEF`.slice(0, 16);
  for (let i = 0; i < 60; i++) {
    await call(e, "/claim", { method: "POST", body: JSON.stringify({ id: id(i), name: `player${i}` }) });
    await call(e, "/score", { method: "POST", body: JSON.stringify({ id: id(i), round: roundInfo.round, words: realWords.slice(0, (i % 5) + 1) }) });
  }
  const table = (await body(await call(e, `/leaderboard?round=${roundInfo.round}`))).entries;
  assert.equal(table.length, 60, "the table left players out");
  const scores = table.map((r) => r.score);
  assert.deepEqual(scores, [...scores].sort((a, b) => b - a), "the table is not best-first");

  const few = (await body(await call(e, `/leaderboard?round=${roundInfo.round}&limit=10`))).entries;
  assert.equal(few.length, 10, "a smaller page was not honoured");
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
  assert.deepEqual(await e.PLAYER.getByName(`player:${ID}`).recent(), [], "progress was banked");

  // The final score replaces the progress row and is the one banked.
  const done = await body(await call(e, "/score", { method: "POST", body: JSON.stringify({ id: ID, round: roundInfo.round, words: realWords }) }));
  table = await body(await call(e, `/leaderboard?round=${roundInfo.round}`));
  const mine = table.entries.find((r) => r.name === "wordfan");
  assert.equal(mine.final, true);
  assert.equal(mine.score, done.score);
  assert.deepEqual(await e.PLAYER.getByName(`player:${ID}`).recent(), [done.score]);

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
  assert.deepEqual(await e2.PLAYER.getByName(`player:${ID2}`).recent(), [], "a zero was banked");
  const table = await body(await call(e2, `/leaderboard?round=${roundInfo.round}`));
  assert.deepEqual(table.entries.map((r) => [r.name, r.score]), [["idler", 0]], "the idle player is missing from the table");
  assert.ok(played.score > 0);
});

await test("rank is the average of the last ten rounds", async () => {
  const e = env();
  await call(e, "/claim", { method: "POST", body: JSON.stringify({ id: ID, name: "wordfan" }) });
  const res = await body(await call(e, "/score", { method: "POST", body: JSON.stringify({ id: ID, round: roundInfo.round, words: realWords }) }));
  assert.deepEqual(await e.PLAYER.getByName(`player:${ID}`).recent(), [res.score]);
  assert.equal(res.average, res.score);
});

await test("a scored round costs no KV write, and claiming the same name again costs none", async () => {
  const e = env();
  let writes = 0;
  const put = e.ROUNDS.put.bind(e.ROUNDS);
  e.ROUNDS.put = async (...args) => { writes += 1; return put(...args); };
  await call(e, "/claim", { method: "POST", body: JSON.stringify({ id: ID, name: "wordfan" }) });
  const afterClaim = writes;
  await call(e, "/claim", { method: "POST", body: JSON.stringify({ id: ID, name: "wordfan" }) });
  assert.equal(writes, afterClaim, "re-claiming the same name wrote to KV");
  await call(e, "/score", { method: "POST", body: JSON.stringify({ id: ID, round: roundInfo.round, words: realWords }) });
  assert.equal(writes, afterClaim, "a scored round wrote to KV");
});

await test("scores a player had in KV carry into their Player object", async () => {
  const e = env();
  await call(e, "/claim", { method: "POST", body: JSON.stringify({ id: ID, name: "wordfan" }) });
  await e.ROUNDS.put(`player:${ID}`, JSON.stringify({ name: "wordfan", recent: [1000, 3000] }));
  const res = await body(await call(e, "/score", { method: "POST", body: JSON.stringify({ id: ID, round: roundInfo.round, words: realWords }) }));
  assert.deepEqual(await e.PLAYER.getByName(`player:${ID}`).recent(), [1000, 3000, res.score]);
  assert.equal(res.average, Math.round((1000 + 3000 + res.score) / 3));
});

await test("progress that changes nothing, or comes too fast, is not written", async () => {
  const e = env();
  await call(e, "/claim", { method: "POST", body: JSON.stringify({ id: ID, name: "wordfan" }) });
  const board = e.LEADERBOARD.getByName(`round:${roundInfo.round}`);
  const realNow = Date.now;
  let clock = realNow();
  Date.now = () => clock;
  try {
    const progress = (words) => call(e, "/progress", { method: "POST", body: JSON.stringify({ id: ID, round: roundInfo.round, words }) });
    await progress([]);
    const first = await board.table();
    assert.equal(first.length, 1);
    clock += 2000;
    await progress(realWords); // too soon after the last
    assert.equal((await board.table())[0].words, 0, "progress two seconds later was written");
    clock += 10000;
    await progress(realWords);
    assert.equal((await board.table())[0].words, realWords.length, "progress after the gap was not written");
  } finally {
    Date.now = realNow;
  }
});

await test("the all-time table ranks players by average, with their best and games", async () => {
  const e = env();
  const players = [
    { id: "0123456789ABCDEF", name: "steady", league: 2, scores: [6000, 6000] },
    { id: "FEDCBA9876543210", name: "spiky", league: 1, scores: [1000, 9000] },
    { id: "0000111122223333", name: "quiet", league: 0, scores: [500] },
  ];
  // Bank each player's rounds through their Player object, as a scored round does,
  // then file the standing the same way /score does.
  for (const p of players) {
    await call(e, "/claim", { method: "POST", body: JSON.stringify({ id: p.id, name: p.name }) });
    for (const s of p.scores) {
      const { recent, best, games } = await e.PLAYER.getByName(`player:${p.id}`).bank(s, []);
      const average = Math.round(recent.reduce((a, b) => a + b, 0) / recent.length);
      await e.STANDINGS.getByName("standings").record(p.id, p.name, p.league, average, best, games);
    }
  }
  const table = (await body(await call(e, "/standings"))).entries;
  assert.deepEqual(
    table.map((r) => [r.name, r.average, r.best, r.games, r.league]),
    [
      ["steady", 6000, 6000, 2, 2],
      ["spiky", 5000, 9000, 2, 1],
      ["quiet", 500, 500, 1, 0],
    ],
    "the table should be ranked by average, with each player's best and games"
  );
  assert.ok(!JSON.stringify(table).includes("0123456789ABCDEF"), "an account id leaked onto the table");

  const short = (await body(await call(e, "/standings?limit=1"))).entries;
  assert.equal(short.length, 1);
});

await test("a scored round puts the player on the all-time table", async () => {
  const e = env();
  await call(e, "/claim", { method: "POST", body: JSON.stringify({ id: ID, name: "wordfan" }) });
  const res = await body(await call(e, "/score", { method: "POST", body: JSON.stringify({ id: ID, round: roundInfo.round, words: realWords, league: 3 }) }));
  const table = (await body(await call(e, "/standings"))).entries;
  assert.deepEqual(table, [{ name: "wordfan", league: 3, average: res.score, best: res.score, games: 1 }]);
});

await test("unknown routes 404", async () => {
  assert.equal((await call(env(), "/nope")).status, 404);
});

console.log(`\n${passed} passed`);
