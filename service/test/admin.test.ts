import { env } from "cloudflare:workers";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import worker from "../src/index";
import { issueLicence } from "../src/licences";

const withPassword = { ...env, ADMIN_PASSWORD: "letmein" };
const get = (path: string, e: any = withPassword, auth = "letmein") =>
  worker.fetch(
    new Request(`https://api.bubbletranslate.app${path}`, {
      headers: auth ? { authorization: `Basic ${btoa(`op:${auth}`)}` } : {},
    }),
    e,
  );

// The download counts come from GitHub; the panel must render without them.
beforeEach(() => {
  vi.spyOn(globalThis, "fetch").mockResolvedValue(new Response("", { status: 503 }));
});
afterEach(() => vi.restoreAllMocks());

it("does not exist when no password is configured", async () => {
  expect((await get("/admin", { ...env, ADMIN_PASSWORD: undefined })).status).toBe(404);
});

it("asks for the password", async () => {
  expect((await get("/admin", withPassword, "")).status).toBe(401);
});

it("renders the dashboard and finds a licence by email", async () => {
  await issueLicence(env, { provider: "paddle", cycle: "yearly", email: "someone@example.com" });
  const res = await get("/admin?q=someone%40example.com");
  expect(res.status).toBe(200);
  const html = await res.text();
  expect(html).toContain("someone@example.com");
  expect(html).toContain("yearly");
});

const post = (path: string, form: Record<string, string>) =>
  worker.fetch(
    new Request(`https://api.bubbletranslate.app${path}`, {
      method: "POST",
      headers: { authorization: `Basic ${btoa("op:letmein")}` },
      body: new URLSearchParams(form),
    }),
    withPassword,
  );

it("writes each action to the history, with only what changed", async () => {
  const { id } = await issueLicence(env, { provider: "paddle", cycle: "yearly", email: "a@example.com" });
  await post("/admin/extend", { id, days: "30" });
  const licence = await env.DB.prepare("SELECT * FROM licences WHERE id = ?").bind(id).first<any>();
  await post("/admin/update", {
    id,
    email: "b@example.com",
    cycle: "yearly",
    status: "active",
    seat_limit: "5",
    expires_at: new Date(licence.expires_at * 1000).toISOString().slice(0, 10),
    translation_limit: "",
  });
  const { results } = await env.DB.prepare(
    "SELECT action, licence_id, detail FROM admin_log ORDER BY id",
  ).all<any>();
  expect(results.map((r: any) => r.action)).toEqual(["extend", "edit"]);
  expect(results[1].licence_id).toBe(id);
  expect(results[1].detail).toBe("email a@example.com → b@example.com, devices 3 → 5");

  const html = await (await get("/admin")).text();
  expect(html).toContain("Your actions");
  expect(html).toContain("devices 3 → 5");
});

it("lists licences ending soon with whether they renew", async () => {
  const { id } = await issueLicence(env, { provider: "manual", cycle: "monthly", email: "soon@example.com", termSeconds: 5 * 86_400 });
  const html = await (await get("/admin")).text();
  expect(html).toContain("Ending in the next 30 days (1 · 1 won't renew)");
  expect(html).toContain(id);
});

it("exports licences as CSV, without keys and with formulas defused", async () => {
  await issueLicence(env, { provider: "manual", cycle: "yearly", email: "=HYPERLINK(1)@example.com" });
  const res = await get("/admin/export.csv?what=licences");
  expect(res.headers.get("content-type")).toContain("text/csv");
  const csv = await res.text();
  expect(csv.split("\r\n")[0]).toBe(
    '"id","email","provider","provider_ref","plan","cycle","status","seat_limit","devices","translation_limit","created","ends","mine"',
  );
  expect(csv).toContain(`"'=HYPERLINK(1)@example.com"`);
  expect(csv).not.toMatch(/key_hash|BT-/);
  expect((await get("/admin/export.csv?what=nope")).status).toBe(404);
  expect((await get("/admin/export.csv?what=licences", withPassword, "")).status).toBe(401);
});

it("counts monthly recurring revenue from licences set to renew", async () => {
  await issueLicence(env, { provider: "paddle", cycle: "monthly", email: "m@example.com" });
  await issueLicence(env, { provider: "paddle", cycle: "yearly", email: "y@example.com" });
  const html = await (await get("/admin")).text();
  // $2 + $20/12
  expect(html).toContain("$3.67");
});

/** The operator's own machines are kept out of every count on purpose, which
 *  leaves "I installed it and nothing appeared" indistinguishable from a ping
 *  that never arrived. The tile that answers it has to count exactly the
 *  machines the others leave out, and split them the same way. */
it("counts the operator's own machines, by platform, apart from everyone else's", async () => {
  const mine = "a".repeat(32);
  const theirs = "b".repeat(32);
  const ping = (install: string, os: string) =>
    worker.fetch(
      new Request("https://api.bubbletranslate.app/v1/ping", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ install, os, app: "0.3.5", plan: "free" }),
      }),
      env,
    );

  await ping(mine, "macos");
  await ping(theirs, "windows");

  // The variable is a comma-separated list, not JSON.
  const res = await get("/admin", { ...withPassword, ADMIN_IGNORE_INSTALLS: mine });
  const html = await res.text();

  const tile = html.slice(html.indexOf("Your own machines"));
  const upTo = tile.slice(0, tile.indexOf("</div></div>"));
  expect(upTo).toContain("macOS 1");
  // The one that is not the operator's stays out of this tile...
  expect(upTo).toContain("Windows 0");
  // ...and the operator's stays out of the count it would otherwise inflate.
  const everyone = html.slice(html.indexOf("Installs ever seen"));
  expect(everyone.slice(0, everyone.indexOf("</div></div>"))).toContain("macOS 0");
});

it("shows who ran out of the free allowance, and where installs are", async () => {
  const t = Math.floor(Date.now() / 1000);
  await env.DB.prepare(
    `INSERT OR REPLACE INTO installs
       (install, os, app, plan, first_seen, last_seen, country, first_capped, last_capped, capped_days)
     VALUES (?1, 'windows', '0.3.7', 'free', ?2, ?3, 'DE', ?4, ?3, 4)`,
  )
    .bind("f".repeat(32), t - 6 * 86400, t, t - 4 * 86400)
    .run();
  const html = await (await get("/admin")).text();
  expect(html).toContain("Hitting the free limit");
  expect(html).toContain("ffffffff");
  expect(html).toContain("median <b>2</b>");
  expect(html).toContain("Installs by country");
  expect(html).toContain("Germany");
});
