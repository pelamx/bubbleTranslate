// Small things the panel's parts share: page size, days and dates.

import { now } from "../tokens";

/** How many rows a listing shows. Enough to scan, few enough to render. */
export const PAGE_SIZE = 40;

export const DAY = 86_400;

/** A running copy says it is still open every five minutes, so one that has
 *  not been heard from in ten has been closed, or its machine is asleep. One
 *  missed beat is allowed for. */
export const ONLINE_WINDOW = 10 * 60;

export const date = (unix: number) => new Date(unix * 1000).toISOString().slice(0, 10);

/** The operator reads the panel in Turkey, which has been UTC+3 all year
 *  since 2016, so "today" starts at midnight there. */
export const LOCAL_OFFSET = 3 * 3600;

/** Unix time of the most recent local midnight. */
export function startOfToday(): number {
  return Math.floor((now() + LOCAL_OFFSET) / DAY) * DAY - LOCAL_OFFSET;
}

/** A slow outside read -- Paddle, Anthropic, DeepL -- kept for `ttl` seconds
 *  in the colo's cache, so a panel reloaded twice in a minute does not ask
 *  all of them again. A string is a reason the read failed, and is never
 *  kept: the next load should try again rather than repeat the failure. */
export async function cached<T>(key: string, ttl: number, read: () => Promise<T | string>): Promise<T | string> {
  const cache = caches.default;
  const request = new Request(`https://admin-cache.invalid/${encodeURIComponent(key)}`);
  try {
    const hit = await cache.match(request);
    if (hit) return (await hit.json()) as T;
  } catch {
    // An unreadable entry is a miss.
  }
  const value = await read();
  if (typeof value !== "string") {
    try {
      await cache.put(
        request,
        new Response(JSON.stringify(value), {
          headers: { "content-type": "application/json", "cache-control": `max-age=${ttl}` },
        }),
      );
    } catch (err) {
      console.error(`could not cache ${key}`, err);
    }
  }
  return value;
}
