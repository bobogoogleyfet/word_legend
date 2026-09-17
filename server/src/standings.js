import { DurableObject } from "cloudflare:workers";

/**
 * The all-time table: one row per player, ranked by the average their league
 * follows.
 *
 * One object holds every row, because a table has to be sorted across all of
 * them. It is written once per scored round per player -- far less traffic than a
 * round's leaderboard -- and read from the stats page.
 */
export class Standings extends DurableObject {
  /** File a player's standing after a scored round. */
  async record(id, name, league, average, best, games) {
    await this.ctx.storage.put(`p:${id}`, { name, league, average, best, games });
  }

  /**
   * The table, best average first, then best score. Account ids never leave this
   * object.
   */
  async table(limit = 50) {
    const rows = await this.ctx.storage.list({ prefix: "p:" });
    return [...rows.values()]
      .sort((a, b) => b.average - a.average || b.best - a.best || a.name.localeCompare(b.name))
      .slice(0, limit)
      .map(({ name, league, average, best, games }) => ({ name, league, average, best, games }));
  }
}
