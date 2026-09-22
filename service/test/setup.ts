import { env } from "cloudflare:workers";
import { beforeEach } from "vitest";

// Every test starts from an empty database: the schema is replayed statement
// by statement (comments stripped), and each table is emptied.
const statements = (env as any).SCHEMA_SQL.replace(/--.*$/gm, "")
  .split(";")
  .map((s: string) => s.trim())
  .filter(Boolean);

beforeEach(async () => {
  for (const sql of statements) await env.DB.prepare(sql).run();
  for (const table of ["seats", "orders", "licences", "paddle_customers", "paddle_subscriptions"]) {
    await env.DB.prepare(`DELETE FROM ${table}`).run();
  }
});
