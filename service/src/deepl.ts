// The DeepL fallback: the one route on which this service sees a translation.
//
// The app asks Google, then MyMemory, directly. Only when both have failed for
// the same selection does a copy without a DeepL key of its own send the text
// here, and this forwards it to DeepL with the key that lives in a secret.
// Nothing is stored but a count: how many requests and characters each install
// spent today, which is what keeps one copy from spending everyone's month.
//
// The key is a free one, 500,000 characters a month for every copy together,
// so it is rationed twice: per install per day, and for all installs per day,
// which spreads the month so that an outage on the first spares the thirtieth.

import type { Env } from "./env";
import { now } from "./tokens";

/** Requests one install may make in a UTC day. A fallback, not an engine:
 *  someone hitting it all day is better told than quietly served. */
export const DEEPL_PER_INSTALL = 50;
/** Characters in one request. A selection longer than this is not a fallback
 *  worth a sixtieth of the day's allowance. */
export const DEEPL_MAX_CHARS = 1500;
/** Characters all installs together may spend in a UTC day; about a thirtieth
 *  of the free plan's month, with some held back. */
export const DEEPL_DAILY_CHARS = 15_000;

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });

const refuse = (message: string, status: number) => json({ error: message }, status);

export async function deeplRelay(env: Env, body: any): Promise<Response> {
  const key = env.DEEPL_API_KEY?.trim();
  if (!key) return refuse("The DeepL fallback is not available.", 503);

  const install = String(body.install ?? "");
  const text = Array.isArray(body.text) ? body.text[0] : body.text;
  const target = String(body.target_lang ?? "");
  const source = body.source_lang == null ? "" : String(body.source_lang);
  if (!/^[0-9a-f]{32}$/.test(install) || typeof text !== "string" || !text.trim()) {
    return refuse("Bad request.", 400);
  }
  if (!/^[A-Z]{2}(-[A-Z]{2,4})?$/.test(target) || (source && !/^[A-Z]{2}$/.test(source))) {
    return refuse("Bad request.", 400);
  }
  if (text.length > DEEPL_MAX_CHARS) return refuse("The text is too long for the fallback.", 413);

  const t = now();
  const day = new Date(t * 1000).toISOString().slice(0, 10);
  const spent = await env.DB.prepare("SELECT COALESCE(SUM(chars), 0) AS chars FROM deepl_usage WHERE day = ?")
    .bind(day)
    .first<{ chars: number }>();
  if ((spent?.chars ?? 0) + text.length > DEEPL_DAILY_CHARS) {
    return refuse("The DeepL fallback has run out for today.", 429);
  }
  // Counted before DeepL is asked, so a request that fails still counts: a
  // copy retrying a failure is exactly what the limit is for.
  const row = await env.DB.prepare(
    `INSERT INTO deepl_usage (day, install, requests, chars) VALUES (?1, ?2, 1, ?3)
     ON CONFLICT (day, install) DO UPDATE SET requests = requests + 1, chars = chars + ?3
     RETURNING requests`,
  )
    .bind(day, install, text.length)
    .first<{ requests: number }>();
  // Nothing reads past today; the key's leading `day` keeps this cheap.
  await env.DB.prepare("DELETE FROM deepl_usage WHERE day < ?")
    .bind(new Date((t - 7 * 86_400) * 1000).toISOString().slice(0, 10))
    .run();
  if ((row?.requests ?? 0) > DEEPL_PER_INSTALL) {
    return refuse("The DeepL fallback has run out for today.", 429);
  }

  const payload: Record<string, unknown> = { text: [text], target_lang: target };
  if (source) payload.source_lang = source;
  const host = key.endsWith(":fx") ? "https://api-free.deepl.com" : "https://api.deepl.com";
  const res = await fetch(`${host}/v2/translate`, {
    method: "POST",
    headers: { authorization: `DeepL-Auth-Key ${key}`, "content-type": "application/json" },
    body: JSON.stringify(payload),
  });

  if (res.ok) {
    const answer: any = await res.json().catch(() => null);
    const first = answer?.translations?.[0];
    if (typeof first?.text !== "string") return refuse("DeepL sent back nothing.", 502);
    return json({
      translations: [{ text: first.text, detected_source_language: first.detected_source_language }],
    });
  }
  // 456 is DeepL's "this month's characters are spent".
  if (res.status === 429 || res.status === 456) {
    return refuse("The DeepL fallback has run out for now.", 429);
  }
  console.error(`deepl relay: DeepL answered ${res.status}`);
  return refuse("DeepL could not translate this.", 502);
}
