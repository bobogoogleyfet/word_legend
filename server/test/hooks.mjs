// Lets the Worker's modules load under plain Node for tests: `cloudflare:workers`
// only exists inside the Workers runtime, so it resolves to a small stand-in.
import { register } from "node:module";

register("./loader.mjs", import.meta.url);
