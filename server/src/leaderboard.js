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
  /** File a player's result for this round, replacing any earlier one. */
  async submit(id, name, score, words) {
    await this.ctx.storage.put(`score:${id}`, { name, score, words });
    await this.ctx.storage.setAlarm(Date.now() + KEEP_MS);
  }

  /** Names and scores, best first. Account ids never leave this object. */
  async table(limit = 50) {
    const rows = await this.ctx.storage.list({ prefix: "score:" });
    return [...rows.values()]
      .sort((a, b) => b.score - a.score || a.name.localeCompare(b.name))
      .slice(0, limit)
      .map(({ name, score, words }) => ({ name, score, words }));
  }

  /** The round is long over: let the object go. */
  async alarm() {
    await this.ctx.storage.deleteAll();
  }
}
