import { env } from "cloudflare:workers";
import { expect, it } from "vitest";
import worker from "../src/index";

const ping = (body: unknown) =>
  worker.fetch(
    new Request("https://api.bubbletranslate.app/v1/ping", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body),
    }),
    env,
  );

const install = (n: string) => n.repeat(32).slice(0, 32);

const tally = async (provider: string) =>
  await env.DB.prepare("SELECT ok, failed, reports FROM provider_health WHERE provider = ?")
    .bind(provider)
    .first<{ ok: number; failed: number; reports: number }>();

it("adds every install's tallies into the day's totals", async () => {
  await ping({
    install: install("a"),
    os: "macos",
    app: "0.3.6",
    providers: { Google: { ok: 40, failed: 2 }, MyMemory: { ok: 3, failed: 0 } },
  });
  await ping({
    install: install("b"),
    os: "windows",
    app: "0.3.6",
    providers: { Google: { ok: 10, failed: 8 } },
  });

  const google = await tally("Google");
  expect(google).toMatchObject({ ok: 50, failed: 10, reports: 2 });
  // The signal worth reading: the fallback winning anything means the one
  // ahead of it stopped answering.
  expect(await tally("MyMemory")).toMatchObject({ ok: 3, failed: 0, reports: 1 });
});

it("still counts the install when the tallies are nonsense", async () => {
  const before = await env.DB.prepare("SELECT COUNT(*) AS n FROM provider_health").first<{
    n: number;
  }>();

  for (const providers of [
    "not an object",
    ["an", "array"],
    { Google: "not a tally" },
    { Google: { ok: -5, failed: "many" } },
    { "": { ok: 1 } },
    { Google: { ok: 0, failed: 0 } },
    null,
    undefined,
  ]) {
    const res = await ping({ install: install("c"), os: "linux", app: "0.3.6", providers });
    // A ping's answer is ignored by the app, so a bad field must never cost it
    // the install count it came for.
    expect(res.status).toBe(204);
  }

  const seen = await env.DB.prepare("SELECT install FROM installs WHERE install = ?")
    .bind(install("c"))
    .first();
  expect(seen).not.toBeNull();

  const after = await env.DB.prepare("SELECT COUNT(*) AS n FROM provider_health").first<{
    n: number;
  }>();
  expect(after!.n).toBe(before!.n);
});

it("records nothing at all for a ping that carries no tallies", async () => {
  const res = await ping({ install: install("d"), os: "macos", app: "0.3.6" });
  expect(res.status).toBe(204);
  expect(await tally("DeepL")).toBeNull();
});

it("shows the backends in the admin panel, and warns when the fallbacks are winning", async () => {
  const admin = { ...env, ADMIN_PASSWORD: "letmein" };
  const dash = () =>
    worker
      .fetch(
        new Request("https://api.bubbletranslate.app/admin", {
          headers: { authorization: `Basic ${btoa("op:letmein")}` },
        }),
        admin,
      )
      .then((r) => r.text());

  // A healthy chain: the leader answers nearly everything.
  await ping({
    install: install("e"),
    os: "macos",
    app: "0.3.6",
    providers: { Google: { ok: 100, failed: 0 } },
  });
  let html = await dash();
  expect(html).toContain("Translation backends, last 7 days");
  expect(html).toContain("what a healthy chain looks like");

  // Now the fallback starts winning, which is the tell that the leader is
  // refusing even though nothing "failed" outright.
  await ping({
    install: install("f"),
    os: "linux",
    app: "0.3.6",
    providers: { MyMemory: { ok: 60, failed: 0 } },
  });
  html = await dash();
  expect(html).toContain("is refusing more than it should");
});

it("records the day the free allowance ran out, once per day", async () => {
  const id = install("d");
  const row = async () =>
    await env.DB.prepare("SELECT first_capped, last_capped, capped_days FROM installs WHERE install = ?")
      .bind(id)
      .first<{ first_capped: number | null; last_capped: number | null; capped_days: number }>();

  await ping({ install: id, os: "windows", app: "0.3.7" });
  expect(await row()).toMatchObject({ first_capped: null, capped_days: 0 });

  // A second capped ping the same day, from a restart, is the same day.
  await ping({ install: id, os: "windows", app: "0.3.7", capped: true });
  await ping({ install: id, os: "windows", app: "0.3.7", capped: true });
  const first = await row();
  expect(first?.capped_days).toBe(1);
  expect(first?.first_capped).not.toBeNull();

  // A later day adds one and leaves the first time alone.
  await env.DB.prepare("UPDATE installs SET last_capped = last_capped - 86400 WHERE install = ?")
    .bind(id)
    .run();
  await ping({ install: id, os: "windows", app: "0.3.7", capped: true });
  const later = await row();
  expect(later?.capped_days).toBe(2);
  expect(later?.first_capped).toBe(first?.first_capped);
});

it("takes the country from the request and nothing else", async () => {
  const id = install("e");
  const req = (country: string | undefined) => {
    const r = new Request("https://api.bubbletranslate.app/v1/ping", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ install: id, os: "linux", app: "0.3.7" }),
    });
    Object.defineProperty(r, "cf", { value: country ? { country } : undefined });
    return r;
  };
  await worker.fetch(req("TR"), env);
  const country = async () =>
    (await env.DB.prepare("SELECT country FROM installs WHERE install = ?").bind(id).first<{ country: string | null }>())
      ?.country;
  expect(await country()).toBe("TR");
  // A request that arrives without one keeps the last known country.
  await worker.fetch(req(undefined), env);
  expect(await country()).toBe("TR");
  // Nonsense is not stored.
  await worker.fetch(req("<script>"), env);
  expect(await country()).toBe("TR");
});
