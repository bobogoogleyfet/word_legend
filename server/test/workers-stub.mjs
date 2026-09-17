// The part of `cloudflare:workers` the Worker uses, for tests under Node.
export class DurableObject {
  constructor(ctx, env) {
    this.ctx = ctx;
    this.env = env;
  }
}
