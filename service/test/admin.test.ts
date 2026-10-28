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

/** A tile's words without its markup: the platform and its count sit in
 *  separate elements, so they line up as columns. */
const text = (html: string) => html.replace(/<[^>]+>/g, "");

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

it("slows a guesser down without locking anyone else out", async () => {
  const from = (ip: string, password: string) =>
    worker.fetch(
      new Request("https://api.bubbletranslate.app/admin", {
        headers: { authorization: `Basic ${btoa(`op:${password}`)}`, "CF-Connecting-IP": ip },
      }),
      withPassword,
    );
  expect((await from("203.0.113.9", "guess")).status).toBe(401);
  // Locked out for a second now, even with the right password.
  expect((await from("203.0.113.9", "letmein")).status).toBe(401);
  expect((await from("198.51.100.4", "letmein")).status).toBe(200);
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
  const upTo = text(tile.slice(0, tile.indexOf("</div></div>")));
  expect(upTo).toContain("macOS 1");
  // The one that is not the operator's stays out of this tile...
  expect(upTo).toContain("Windows 0");
  // ...and the operator's stays out of the count it would otherwise inflate.
  const everyone = html.slice(html.indexOf("Installs ever seen"));
  expect(text(everyone.slice(0, everyone.indexOf("</div></div>")))).toContain("macOS 0");
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
  // Seen today, so it is a point on the map too.
  expect(html).toContain("<title>Germany: 1</title>");
});

/** An update is only visible as a known install reporting a new version, so
 *  the ping has to notice the change before it overwrites the old one. */
it("shows a copy that moved to a new version, and not one that stayed", async () => {
  const ping = (install: string, app: string) =>
    worker.fetch(
      new Request("https://api.bubbletranslate.app/v1/ping", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ install, os: "linux", app, plan: "free" }),
      }),
      env,
    );
  const moved = "c".repeat(32);
  const stayed = "d".repeat(32);
  await ping(moved, "0.4.1");
  await ping(stayed, "0.4.1");
  await ping(moved, "0.4.2");
  await ping(stayed, "0.4.1");

  const { results } = await env.DB.prepare("SELECT install, from_app, to_app FROM updates").all();
  expect(results).toEqual([{ install: moved, from_app: "0.4.1", to_app: "0.4.2" }]);

  const html = await (await get("/admin")).text();
  const card = html.slice(html.indexOf("Updates, last 30 days"));
  expect(card).toContain("cccccccc");
  expect(card).toContain("0.4.1 → <b>0.4.2</b>");
  expect(card.slice(0, card.indexOf("</section>"))).not.toContain("dddddddd");
});

it("shows which version this week's users are on", async () => {
  const t = Math.floor(Date.now() / 1000);
  const add = (install: string, app: string, seen: number) =>
    env.DB.prepare(
      `INSERT OR REPLACE INTO installs (install, os, app, plan, first_seen, last_seen)
       VALUES (?1, 'windows', ?2, 'free', ?3, ?3)`,
    )
      .bind(install, app, seen)
      .run();
  await add("1".repeat(32), "0.4.9", t);
  await add("2".repeat(32), "0.4.9", t - 3 * 86400);
  await add("3".repeat(32), "0.3.1", t - 20 * 86400);
  const html = await (await get("/admin")).text();
  const card = html.slice(html.indexOf("Active users by version"));
  const body = card.slice(0, card.indexOf("</section>"));
  expect(body).toContain("<b>2</b> <span class=\"muted\">· 1 today</span>");
  // Not seen this week, so not an active user.
  expect(body).not.toContain("0.3.1");
});

/** "Online now" counts copies that said they are still open in the last ten
 *  minutes, not everyone who opened the app today. */
it("counts a copy as online while it keeps saying it is open", async () => {
  const post = (path: string, body: object) =>
    worker.fetch(
      new Request(`https://api.bubbletranslate.app${path}`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(body),
      }),
      env,
    );
  const open = "e".repeat(32);
  const closed = "9".repeat(32);
  const t = Math.floor(Date.now() / 1000);
  await post("/v1/ping", { install: open, os: "linux", app: "0.4.4", plan: "free" });
  await post("/v1/ping", { install: closed, os: "macos", app: "0.4.4", plan: "free" });
  // Both opened today; one went quiet an hour ago, the other is still beating.
  await env.DB.prepare("UPDATE installs SET alive_at = ?1").bind(t - 3600).run();
  await post("/v1/alive", { install: open });
  // An id the daily ping never created gets no row.
  await post("/v1/alive", { install: "8".repeat(32) });

  const { results } = await env.DB.prepare("SELECT install FROM installs WHERE alive_at > ?1")
    .bind(t - 600)
    .all();
  expect(results).toEqual([{ install: open }]);

  const html = await (await get("/admin")).text();
  const tile = html.slice(html.indexOf("Online now"));
  const upTo = tile.slice(0, tile.indexOf("</div></div>"));
  expect(upTo).toContain(">1<");
  expect(text(upTo)).toContain("Linux 1");
  expect(text(upTo)).toContain("macOS 0");
});

/** A copy left open across midnight counts for the new day as soon as it says
 *  it is still open, without waiting for its next daily ping. */
it("counts an app still open after midnight as active today", async () => {
  const activeToday = async () => {
    const html = await (await get("/admin")).text();
    const tile = html.slice(html.indexOf('<p class="k">Active users'));
    return Number(/<div class="v">(\d+)<\/div>/.exec(tile)![1]);
  };
  const before = await activeToday();
  const t = Math.floor(Date.now() / 1000);
  // Last daily ping three days ago, but said "still open" a moment ago.
  await env.DB.prepare(
    `INSERT OR REPLACE INTO installs (install, os, app, plan, first_seen, last_seen, alive_at)
     VALUES (?1, 'linux', '0.4.4', 'free', ?2, ?2, ?3)`,
  )
    .bind("7".repeat(32), t - 3 * 86400, t)
    .run();
  expect(await activeToday()).toBe(before + 1);
});

it("lists Paddle refunds with what went back and what left the balance", async () => {
  const { id } = await issueLicence(env, {
    provider: "paddle",
    cycle: "yearly",
    email: "buyer@example.com",
    providerRef: "sub_1",
  });
  await env.DB.prepare(
    `INSERT INTO paddle_adjustments
       (adjustment_id, transaction_id, subscription_id, action, type, status, currency,
        total, tax, balance_currency, earnings, retained_fee, paddle_created, event_at, created_at, updated_at)
     VALUES ('adj_1', 'txn_1', 'sub_1', 'refund', 'partial', 'approved', 'USD',
        2000, 333, 'USD', 1517, 150, 1760443334, 1, 1, 1),
            ('adj_2', 'txn_2', 'sub_2', 'refund', 'full', 'pending_approval', 'USD',
        200, 33, 'USD', 107, 0, 1760443335, 1, 1, 1)`,
  ).run();
  const html = await (await get("/admin")).text();
  expect(html).toContain("Refunds (2 · 1 waiting for Paddle)");
  expect(html).toContain(id);
  expect(html).toContain("buyer@example.com");
  expect(html).toContain("−$16.67");
  expect(html).toContain("fee kept $1.50");
  // Only the approved refund counts in the totals.
  expect(text(html)).toContain("$20.00 back to buyers · $16.67 out of the balance, $1.50 of it Paddle's kept fees");
});
