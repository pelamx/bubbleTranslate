// The tier lineup, in one place so it can be edited without reading the page
// code. Two tiers today: the free one the app ships with, and Pro.
//
// Two things are deliberately *not* inlined here.
//
// Price ids come from the environment, not from this file. They differ between
// the sandbox and live accounts, and a `pri_...` literal committed next to the
// copy is exactly how a sandbox id reaches production -- where it fails at
// checkout rather than at deploy. `paddlePriceId` reads them from wrangler.toml.
//
// The visible words come from the i18n table, not from this file, because the
// buy page is served in English, Turkish and Spanish. A tier carries the *key*
// of its copy; `i18n.ts` carries the sentences.

import type { Cycle, Env } from "./env";
import { paddlePriceId } from "./env";

export interface Tier {
  /** Stable id. Used in markup and analytics, never shown to anyone. */
  id: "free" | "pro";
  /** Key into the strings table for this tier's name. */
  nameKey: "tierFreeName" | "tierProName";
  /** Whether this tier is bought. The free tier has no checkout button
   *  because there is nothing to charge for -- the app is already doing it. */
  purchasable: boolean;
  /** Resolved per cycle, from the environment. Null for the free tier, and
   *  null for a cycle whose price id has not been configured. */
  priceId(env: Env, cycle: Cycle): string | null;
}

export const TIERS: Tier[] = [
  {
    id: "free",
    nameKey: "tierFreeName",
    purchasable: false,
    priceId: () => null,
  },
  {
    id: "pro",
    nameKey: "tierProName",
    purchasable: true,
    priceId: (env, cycle) => paddlePriceId(env, cycle),
  },
];

export const proTier = (): Tier => TIERS.find((tier) => tier.id === "pro")!;
