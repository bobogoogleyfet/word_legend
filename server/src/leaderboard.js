import { DurableObject } from "cloudflare:workers";

/**
 * One round's leaderboard.
 *
 * Every player's round ends on the same tick, so their scores arrive together.
 * Kept in KV, the table was a read-modify-write that let simultaneous
 * submissions overwrite each other, and KV's cached reads could hide a score
 * from other players for as long as the scorecard was open. A Durable Object per
 * round fixes both: its storage is strongly consistent, and each player's row is
 * its own key, so a submission never rewrites anyone else's.
 */

/** How long a round's table is kept once scores stop arriving. */
const KEEP_MS = 7 * 24 * 60 * 60 * 1000;

export class Leaderboard extends DurableObject {
  /**
   * File a player's result for this round, replacing any earlier one. `final` is
   * false for progress sent while the round is still being played, and true for
   * the score handed in at the end. Progress never overwrites a final score: a
   * slow progress report can land after the final one.
   */
  async submit(id, name, score, words, final = true, league = null) {
    const key = `score:${id}`;
    if (!final) {
      const existing = await this.ctx.storage.get(key);
      if (existing?.final) return;
    }
    await this.ctx.storage.put(key, { name, score, words, final, league });
    await this.ctx.storage.setAlarm(Date.now() + KEEP_MS);
  }

  /**
   * Names and scores, best first, and whether each is final yet. Account ids never
   * leave this object.
   */
  async table(limit = 50) {
    const rows = await this.ctx.storage.list({ prefix: "score:" });
    return [...rows.values()]
      .sort((a, b) => b.score - a.score || a.name.localeCompare(b.name))
      .slice(0, limit)
      // Rows from before `final` existed were all final scores.
      .map(({ name, score, words, final, league }) => ({ name, score, words, final: final ?? true, league: league ?? null }));
  }

  /** The round is long over: let the object go. */
  async alarm() {
    await this.ctx.storage.deleteAll();
  }
}
