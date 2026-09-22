// Small things the panel's parts share: page size, days and dates.

import { now } from "../tokens";

/** How many rows a listing shows. Enough to scan, few enough to render. */
export const PAGE_SIZE = 40;

export const DAY = 86_400;

export const date = (unix: number) => new Date(unix * 1000).toISOString().slice(0, 10);

/** The operator reads the panel in Turkey, which has been UTC+3 all year
 *  since 2016, so "today" starts at midnight there. */
export const LOCAL_OFFSET = 3 * 3600;

/** Unix time of the most recent local midnight. */
export function startOfToday(): number {
  return Math.floor((now() + LOCAL_OFFSET) / DAY) * DAY - LOCAL_OFFSET;
}
