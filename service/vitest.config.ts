import { readFileSync } from "node:fs";
import { cloudflareTest } from "@cloudflare/vitest-pool-workers";
import { defineConfig } from "vitest/config";

// The tests run inside the real Workers runtime against a local, throwaway D1
// built from schema.sql, so a webhook is exercised end to end — signature,
// SQL and all — without touching Paddle or the deployed database.
export default defineConfig({
  plugins: [
    cloudflareTest({
      wrangler: { configPath: "./wrangler.toml" },
      miniflare: {
        // The bundled test runtime can lag the deployed date by a few weeks.
        compatibilityDate: "2026-08-22",
        bindings: {
          SCHEMA_SQL: readFileSync("./schema.sql", "utf8"),
          PADDLE_WEBHOOK_SECRET: "test-webhook-secret",
          // Lets the service mint its own signing key, as `wrangler dev` does.
          DEV_MODE: "1",
        },
      },
    }),
  ],
  test: { setupFiles: ["./test/setup.ts"] },
});
