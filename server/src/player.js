import { DurableObject } from "cloudflare:workers";

/**
 * One player's banked scores: the last ten, whose average is their rank.
 *
 * These used to live in KV beside the player's name, rewritten every round the
 * player scored -- and KV allows only a thousand writes a day on the free plan,
 * so a handful of players could use them up. A Durable Object per player writes
 * to its own storage instead, and KV is written only when a name is claimed.
 */
export class Player extends DurableObject {
  /**
   * Bank a round's score and return the recent scores. `legacy` carries the scores
   * a player had in KV before this object existed; they seed it the first time.
   * A score of zero is not banked: a round with nothing found was not played.
   */
  async bank(score, legacy = []) {
    let recent = await this.ctx.storage.get("recent");
    if (recent === undefined) recent = Array.isArray(legacy) ? legacy.slice(-10) : [];
    if (score > 0) {
      recent = [...recent, score].slice(-10);
      await this.ctx.storage.put("recent", recent);
    }
    return recent;
  }

  /** The recent scores, without banking anything. */
  async recent() {
    return (await this.ctx.storage.get("recent")) ?? [];
  }
}
