// OpenNext adapter config for Cloudflare Workers.
// Paste4Ever uses no DB bindings, no KV, no R2 — all external state lives
// in the Rust API. So the default Cloudflare config is enough.
import { defineCloudflareConfig } from "@opennextjs/cloudflare";

export default defineCloudflareConfig({});
