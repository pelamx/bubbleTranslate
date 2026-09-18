// The four pages this service serves to a browser, in three languages.
//
// They are here rather than on a separate site for one reason: /buy is the
// only page that has to know which country the visitor is in, and this Worker
// is already the thing behind Cloudflare that gets told. A static site would
// have to ask this service anyway, and then the routing would live in two
// places instead of one.
//
// Everything is inline bar two fetches, both only on the pages a buyer sees:
// the site's typeface, so the checkout reads as the same product one click
// after the site, and Paddle.js, on the one page that opens a checkout.

import type { Cycle } from "./env";
import { TIERS } from "./tiers";
import { DEFAULT_LANG, LANGS, type Lang, type Strings, switchedTo, t, withLang } from "./i18n";

/// What the free tier allows per day. Display only: the number the client
/// enforces is `FREE_DAILY_TRANSLATIONS` in `src/license.rs`, and this one has
/// to say the same thing or the page and the bubble will disagree.
const FREE_DAILY_TRANSLATIONS = 10;

export const escapeHtml = (value: unknown): string =>
  String(value ?? "")
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");

/** JSON for embedding inside an inline `<script>` block.
 *
 *  `JSON.stringify` alone is not safe there: the HTML parser ends a script
 *  element at the first `</script>` it sees, *including inside a string
 *  literal*, so a value containing that sequence closes the block early and
 *  everything after it becomes markup. Escaping `<` (and, for good measure,
 *  `>` and the Unicode line separators, which are valid JSON but line
 *  terminators to a JavaScript parser) makes the embedded JSON incapable of
 *  terminating the element while parsing to exactly the same value. */
const jsonForScript = (value: unknown): string =>
  JSON.stringify(value)
    .replace(/</g, "\\u003c")
    .replace(/>/g, "\\u003e")
    .replace(/\u2028/g, "\\u2028")
    .replace(/\u2029/g, "\\u2029");

/** The marketing site, which is where the policies live. Paddle's domain
 *  review fetches the *checkout* domain -- this service -- so the terms,
 *  privacy notice and refund policy have to be reachable from here too, not
 *  only from the site they are written on. */
export const SITE = "https://bubbletranslate.app";

/** The design tokens of the marketing site, repeated here on purpose.
 *
 *  A buyer arrives on this domain mid-thought, one click after the site, and
 *  hands over a card. Anything that reads as a different product at that
 *  moment is a reason to stop. These values are therefore not a theme: they
 *  are the same values `bubbletranslate.app/styles.css` declares, and they
 *  have to be changed in both places together. */
const STYLE = `
  :root {
    color-scheme: dark;
    --bg: #0b1020; --bg-soft: #121a33; --card: #16203f; --border: #24325c;
    --text: #eaf0ff; --muted: #9fb0d4;
    --brand: #4f8cff; --brand-2: #7c5cff; --accent: #23d5ab;
  }
  footer.legal { max-width: 620px; margin: 30px auto 44px; padding: 0 18px; text-align: center;
    font-size: .82rem; display: flex; gap: 16px; justify-content: center; flex-wrap: wrap; }
  footer.legal a { color: var(--muted); text-decoration: none; }
  footer.legal a:hover { color: var(--text); text-decoration: underline; }
  * { box-sizing: border-box; }
  body {
    margin: 0; padding: 40px 20px; color: var(--text);
    background: radial-gradient(1200px 600px at 80% -10%, rgba(124,92,255,.18), transparent 60%),
                radial-gradient(900px 500px at 0% 10%, rgba(79,140,255,.16), transparent 55%),
                var(--bg);
    background-attachment: fixed;
    font: 15px/1.6 "Inter", system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI",
          Roboto, "Helvetica Neue", Arial, sans-serif;
    /* A column, not a row: the legal footer is a sibling of the sheet, and in
       a row it lands beside the content instead of under it. */
    display: flex; flex-direction: column; align-items: center;
    position: relative;
  }
  /* Room for the language switcher above the mark on a narrow screen, where
     the two would otherwise share the same line. */
  @media (max-width: 560px) { body { padding-top: 52px; } }
  .sheet { width: 100%; max-width: 620px; }
  .sheet.wide { max-width: 1040px; }
  /* The mark, so the page that takes the money is visibly the same product
     as the page that asked for it. */
  .brandbar { display: flex; align-items: center; gap: 10px; margin: 0 0 26px; font-weight: 800; font-size: 1.1rem; }
  .brandbar svg { width: 32px; height: 32px; flex: none; }
  table { width: 100%; border-collapse: collapse; font-size: 13px; margin: 6px 0 4px; }
  th, td { text-align: left; padding: 8px 10px; border-bottom: 1px solid var(--border); vertical-align: top; }
  th { color: var(--muted); font-weight: 600; font-size: 12px; }
  td.num, th.num { text-align: right; font-variant-numeric: tabular-nums; }
  .scroll { overflow-x: auto; }
  .tiles { display: grid; gap: 10px; grid-template-columns: repeat(auto-fit, minmax(130px, 1fr)); margin: 18px 0; }
  .tile { background: var(--card); border: 1px solid var(--border); border-radius: 12px; padding: 12px 14px; }
  .tile .n { font-size: 22px; font-weight: 700; font-variant-numeric: tabular-nums; }
  .tile .l { font-size: 12px; color: var(--muted); }
  .row { display: flex; gap: 8px; align-items: flex-end; flex-wrap: wrap; }
  .row > * { margin-top: 0; }
  .row button, .row input { width: auto; }
  form.inline { display: inline; }
  form.inline button { width: auto; padding: 5px 10px; font-size: 12px; margin: 0 2px 0 0; }
  code { font: 12px ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; color: #c7d4f0; }
  h1 { font-size: 28px; line-height: 1.15; margin: 0 0 8px; font-weight: 800; letter-spacing: -.4px; }
  h2 { font-size: 15px; margin: 28px 0 10px; font-weight: 700; color: var(--text); }
  p { margin: 0 0 14px; color: var(--muted); }
  .muted { color: var(--muted); font-size: 13px; }
  /* The instrument that takes the money. Raised off the page background so it
     reads as a till rather than as another paragraph with a button under it. */
  .paycard {
    background: linear-gradient(180deg, rgba(22,32,63,.92), rgba(18,26,51,.92));
    border: 1px solid var(--border); border-radius: 18px;
    padding: 6px 22px 22px; margin: 22px 0 0;
    box-shadow: 0 24px 60px rgba(0,0,0,.4);
  }
  @media (max-width: 520px) { .paycard { padding: 4px 16px 18px; } }
  .paycard-top {
    display: flex; align-items: center; justify-content: center; gap: 7px;
    padding: 14px 0 2px; font-size: 12.5px; color: var(--muted);
    letter-spacing: .01em;
  }
  .paycard-top svg { color: var(--accent); flex: none; }
  .fineprint { font-size: 12.5px; margin: 16px 2px 0; text-align: center; }
  .plans { display: grid; gap: 12px; grid-template-columns: 1fr 1fr; margin: 16px 0 18px; }
  @media (max-width: 520px) { .plans { grid-template-columns: 1fr; } }
  .plan {
    /* Darker than the panel it sits in, so the two cards read as things to
       choose between rather than as more panel. */
    border: 1px solid var(--border); border-radius: 14px; padding: 16px; cursor: pointer;
    background: rgba(11,16,32,.55); display: block; position: relative;
    transition: border-color .15s ease, box-shadow .15s ease, background .2s ease;
  }
  .plan:hover { border-color: #33488a; }
  .plan.on {
    border-color: var(--brand); background: var(--bg-soft);
    box-shadow: 0 0 0 1px var(--brand), 0 10px 30px rgba(79,140,255,.22);
  }
  .plan input { position: absolute; opacity: 0; pointer-events: none; }
  .plan .name { font-size: 13px; color: var(--muted); }
  .plan .price { font-size: 26px; font-weight: 700; margin: 4px 0 2px; letter-spacing: -.5px; }
  .plan .note { font-size: 12px; color: var(--accent); }
  .tiers { display: grid; gap: 12px; grid-template-columns: 1fr 1fr; margin: 18px 0 6px; }
  @media (max-width: 520px) { .tiers { grid-template-columns: 1fr; } }
  /* What each tier gives, stated before the panel. Deliberately quiet: this
     is context for the decision, not the decision. */
  .tier { border: 1px solid transparent; border-left: 2px solid var(--border);
    border-radius: 0; padding: 2px 0 2px 14px; font-size: 13px; color: var(--muted); }
  .tier .name { color: var(--text); font-weight: 600; margin-bottom: 3px; }
  .tier.pro { border-left-color: var(--brand); }
  .tier.pro .name { color: var(--brand); }
  /* What the buyer needs to know before typing a card number, in one line. */
  .trust { display: flex; gap: 8px 18px; flex-wrap: wrap; justify-content: center;
    margin: 16px 0 0; font-size: 12.5px; color: var(--muted); }
  .trust span { display: inline-flex; align-items: center; gap: 6px; }
  .trust span::before { content: ""; width: 5px; height: 5px; border-radius: 50%;
    background: var(--accent); flex: none; }
  label.field { display: block; font-size: 13px; color: var(--muted); margin: 0 0 6px; }
  input[type=text], input[type=email], input[type=number], input[type=date], select, textarea {
    width: 100%; padding: 12px 14px; border-radius: 10px; border: 1px solid var(--border);
    background: rgba(11,16,32,.6); color: var(--text); font-size: 15px; font-family: inherit;
  }
  input::placeholder, textarea::placeholder { color: #6b7ba6; }
  input:focus, select:focus, textarea:focus {
    outline: none; border-color: var(--brand); box-shadow: 0 0 0 3px rgba(79,140,255,.25);
  }
  textarea.keys {
    font: 14px/1.7 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    letter-spacing: .06em; resize: vertical; margin: 8px 0 14px;
  }
  /* The admin edit panel: folded away so the listing stays a listing. */
  tr.editrow td { border-bottom: 1px solid var(--border); padding-top: 0; }
  tr.editrow details > summary {
    cursor: pointer; color: var(--muted); font-size: 12px; padding: 2px 0; list-style: none;
  }
  tr.editrow details > summary::-webkit-details-marker { display: none; }
  tr.editrow details > summary::before { content: "▸ "; }
  tr.editrow details[open] > summary::before { content: "▾ "; }
  tr.editrow details[open] > summary { color: var(--text); margin-bottom: 10px; }
  .row.edit { margin-bottom: 12px; }
  button.danger { background: #4a1f2b; color: #ffb4c4; box-shadow: none; }
  button {
    width: 100%; padding: 13px 18px; border-radius: 999px; border: 0; cursor: pointer;
    background: linear-gradient(135deg, var(--brand), var(--brand-2)); color: #fff;
    font-size: 15px; font-weight: 600; font-family: inherit; margin-top: 14px;
    box-shadow: 0 10px 30px rgba(79,140,255,.35);
    transition: transform .15s ease, box-shadow .15s ease;
  }
  button:hover:not(:disabled) { transform: translateY(-2px); box-shadow: 0 16px 40px rgba(79,140,255,.45); }
  button:disabled { opacity: .55; cursor: default; box-shadow: none; }
  button.quiet { background: transparent; border: 1px solid var(--border); color: var(--text); box-shadow: none; }
  button.quiet:hover:not(:disabled) { background: var(--bg-soft); }
  .key {
    font: 20px/1.4 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    letter-spacing: .06em; background: rgba(11,16,32,.6); border: 1px solid var(--brand);
    border-radius: 12px; padding: 16px; text-align: center; user-select: all;
    margin: 8px 0 14px; word-break: break-all;
  }
  .warn { color: #f5c382; }
  .err { color: #ff9696; }
  .ok { color: var(--accent); }
  hr { border: 0; border-top: 1px solid var(--border); margin: 26px 0; }
  a { color: var(--brand); }
  ol { color: var(--muted); padding-left: 20px; }
  li { margin-bottom: 6px; }
  iframe { width: 100%; border: 0; min-height: 720px; }
  /* Absolute, not fixed: pinned to the top of the page rather than to the
     viewport, so it scrolls away instead of riding over the panel that takes
     the card details. */
  .langs {
    position: absolute; top: 14px; right: 18px; display: flex; gap: 4px;
    font-size: 12px; letter-spacing: .04em;
  }
  .langs a {
    color: var(--muted); text-decoration: none; padding: 4px 8px; border-radius: 999px;
    border: 1px solid transparent;
  }
  .langs a:hover { color: var(--text); }
  .langs a.on { color: var(--text); border-color: var(--brand); background: var(--card); }
  @media (prefers-reduced-motion: reduce) {
    button, .plan { transition: none; }
    button:hover:not(:disabled) { transform: none; }
  }
`;

/** A closed padlock, drawn rather than fetched. The one piece of iconography
 *  on the page, above the controls that ask for money. */
const LOCK = `<svg viewBox="0 0 24 24" aria-hidden="true" focusable="false" width="14" height="14"
  fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round">
  <rect x="4" y="10.5" width="16" height="10" rx="2.5"/>
  <path d="M8 10.5V7a4 4 0 0 1 8 0v3.5"/>
</svg>`;

/** The site's mark, inline so the checkout page never waits on another
 *  origin to look like itself. Kept in step with `bubbletranslate.app`'s
 *  favicon.svg. */
const LOGO = `<svg viewBox="0 0 64 64" aria-hidden="true" focusable="false">
  <defs><linearGradient id="btg" x1="0" y1="0" x2="1" y2="1">
    <stop offset="0" stop-color="#4f8cff"/><stop offset="1" stop-color="#7c5cff"/>
  </linearGradient>
  <clipPath id="btglobe"><ellipse cx="34" cy="29" rx="11" ry="8.4"/></clipPath></defs>
  <g fill="url(#btg)">
    <circle cx="22" cy="27" r="12"/><circle cx="37" cy="21" r="14.5"/>
    <circle cx="47" cy="30" r="11"/><circle cx="30" cy="35" r="12.5"/>
    <circle cx="43" cy="37" r="10.5"/><circle cx="18" cy="47" r="4.6"/>
    <circle cx="12" cy="55" r="3"/>
  </g>
  <g clip-path="url(#btglobe)" fill="none" stroke="#fff" stroke-width="1.5">
    <line x1="23" y1="29" x2="45" y2="29"/><line x1="24.5" y1="23.4" x2="43.5" y2="23.4"/>
    <line x1="24.5" y1="34.6" x2="43.5" y2="34.6"/><line x1="34" y1="20.6" x2="34" y2="37.4"/>
    <ellipse cx="34" cy="29" rx="4" ry="8.4"/>
  </g>
  <ellipse cx="34" cy="29" rx="11" ry="8.4" fill="none" stroke="#fff" stroke-width="1.8"/>
</svg>`;

/** The mark plus the wordmark, linking back to the site it came from. */
const brandBar = (): string =>
  `<a class="brandbar" href="${SITE}/" style="color:inherit;text-decoration:none">${LOGO}<span>BubbleTranslate</span></a>`;

/** Where the page is being served, which the language switcher needs to
 *  link back to. Absent on the admin panel, which has no switcher. */
export interface PageContext {
  lang: Lang;
  url: URL;
}

/** The policy links every page below carries.
 *
 *  Not decoration: a checkout domain whose terms, privacy notice and refund
 *  policy cannot be reached from it is the documented reason Paddle sends a
 *  domain review back as `action_required`. */
function legalFooter(lang: Lang): string {
  const s = t(lang);
  const link = (href: string, label: string) =>
    `<a href="${escapeHtml(href)}">${escapeHtml(label)}</a>`;
  return `<footer class="legal">${[
    link(`${SITE}/`, s.legalHome),
    link(`${SITE}/terms`, s.legalTerms),
    link(`${SITE}/privacy`, s.legalPrivacy),
    link(`${SITE}/refunds`, s.legalRefunds),
  ].join("")}</footer>`;
}

export function page(
  title: string,
  body: string,
  head = "",
  wide = false,
  ctx?: PageContext,
): Response {
  const lang = ctx?.lang ?? DEFAULT_LANG;
  const switcher = ctx
    ? `<nav class="langs" aria-label="Language">${LANGS.map(
        (code) =>
          `<a href="${escapeHtml(switchedTo(ctx.url, code))}" hreflang="${code}"` +
          ` lang="${code}" title="${escapeHtml(t(code).langName)}"` +
          `${code === lang ? ' class="on" aria-current="true"' : ""}>${code.toUpperCase()}</a>`,
      ).join("")}</nav>`
    : "";
  const headers: Record<string, string> = { "content-type": "text/html; charset=utf-8" };
  if (ctx) {
    // The cookie is what brings the buyer back in their language after the
    // processor's redirect, which lands on a URL written before they left.
    // Secure: the service is HTTPS-only, and a cookie that rides only on
    // HTTPS cannot be read out of a downgrade. HttpOnly costs nothing — no
    // script on this service reads it, and the choice survives the trip in
    // `?lang=` and the form field anyway.
    headers["set-cookie"] =
      `lang=${lang}; Path=/; Max-Age=31536000; SameSite=Lax; Secure; HttpOnly`;
  }
  // Inter is the site's typeface, so a buyer arriving one click later reads
  // the same letterforms. Only on the pages a buyer sees: the admin panel
  // has no reason to wait on a font from another origin, and the stack falls
  // back to system-ui everywhere if the request never lands.
  const font = ctx
    ? `<link rel="preconnect" href="https://fonts.googleapis.com">
<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
<link href="https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600;700;800&display=swap" rel="stylesheet">`
    : "";
  return new Response(
    `<!doctype html><html lang="${lang}"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<link rel="icon" type="image/svg+xml" href="${SITE}/favicon.svg">
<title>${escapeHtml(title)}</title>${font}<style>${STYLE}</style>${head}</head>
<body>${switcher}<div class="sheet${wide ? " wide" : ""}">${ctx ? brandBar() : ""}${body}</div>${legalFooter(lang)}</body></html>`,
    { headers },
  );
}

const planCard = (s: Strings, cycle: Cycle, price: string, checked: boolean, note: string) => `
  <label class="plan${checked ? " on" : ""}">
    <input type="radio" name="cycle" value="${cycle}"${checked ? " checked" : ""}>
    <div class="name">${cycle === "yearly" ? s.yearly : s.monthly}</div>
    <div class="price" data-cycle="${cycle}">${escapeHtml(price)}</div>
    <div class="note">${escapeHtml(note)}</div>
  </label>`;

/** Keeps the selected card highlighted. Twelve lines of DOM rather than a
 *  framework, because this page has exactly one piece of state. */
const PLAN_SCRIPT = `
  for (const input of document.querySelectorAll('.plan input')) {
    input.addEventListener('change', () => {
      for (const plan of document.querySelectorAll('.plan')) {
        plan.classList.toggle('on', plan.contains(document.querySelector('.plan input:checked')));
      }
    });
  }`;

// -- /buy --------------------------------------------------------------------

export interface BuyOptions {
  ctx: PageContext;
  configured: boolean;
  /** A key into the strings, so the reason reads in the page's language. */
  reason?: "reasonPaddleUnconfigured";
  monthly: string;
  yearly: string;
  src: string;
  clientToken?: string;
  paddleEnv?: "production" | "sandbox";
  priceMonthly?: string;
  priceYearly?: string;
  successUrl?: string;
  /** A real ISO 3166-1 alpha-2 code, or undefined. Never a sentinel:
   *  Cloudflare sends XX for an address it cannot place and T1 for Tor.
   *  Neither is a country, and Paddle rejects them. When this is undefined
   *  the browser omits `address` entirely and Paddle geolocates the visitor's
   *  IP, which is both more accurate than our guess and the documented
   *  behaviour. */
  country?: string;
}

export function buyPage(opts: BuyOptions): Response {
  const { ctx } = opts;
  const s = t(ctx.lang);
  if (!opts.configured) {
    return page(
      s.proTitle,
      `<h1>${s.proTitle}</h1>
       <p class="warn">${s.checkoutUnavailable}</p>
       <p class="muted">${escapeHtml(opts.reason ? s[opts.reason] : s.providerNotConfigured)}</p>`,
      "",
      false,
      ctx,
    );
  }

  // The free tier is described here, next to Pro, because this is the page
  // the bubble sends someone to at the moment they hit the wall — the one
  // place they will read what the wall is and what removes it.
  const heading = `
    <h1>${s.proTitle}</h1>
    <p>${s.introPaddle}</p>
    <div class="tiers">
      ${TIERS.map(
        (tier) => `
      <div class="tier${tier.purchasable ? " pro" : ""}" data-tier="${tier.id}">
        <div class="name">${s[tier.nameKey]}</div>
        ${tier.id === "free" ? s.tierFree(FREE_DAILY_TRANSLATIONS) : s.tierPro}
      </div>`,
      ).join("")}
    </div>`;
  const plans = `
    <div class="plans">
      ${planCard(s, "monthly", opts.monthly, false, s.billedMonthly)}
      ${planCard(s, "yearly", opts.yearly, true, s.yearlyNotePaddle)}
    </div>`;

  // Paddle. The overlay wants the price id, and carries our order ref through
  // to the webhook in `customData` — that ref is how the success page knows
  // which licence to reveal.
  const paddleScript = `
    <script src="https://cdn.paddle.com/paddle/v2/paddle.js"></script>`;
  // Everything that takes money sits inside one raised panel: the cycle, the
  // address the key is sent to, and the button. A checkout that looks like a
  // discrete instrument is read as one, where the same controls loose on the
  // page read as a form someone bolted on.
  const body = `
    ${heading}
    <section class="paycard" aria-label="${escapeHtml(s.proTitle)}">
      <header class="paycard-top">${LOCK}<span>${s.securePayment}</span></header>
      ${plans}
      <label class="field" for="email">${s.emailLabel}</label>
      <input id="email" type="email" required autocomplete="email" placeholder="${escapeHtml(s.emailPlaceholder)}">
      <button id="pay" type="button">${s.continueToPayment}</button>
      <div class="trust">
        <span>${s.trustPlatforms}</span><span>${s.trustDevices}</span><span>${s.trustCancel}</span>
      </div>
    </section>
    <p class="muted fineprint">${s.paddleNote}</p>
    <script>
      ${PLAN_SCRIPT}
      Paddle.Environment.set(${jsonForScript(opts.paddleEnv)});
      Paddle.Initialize({ token: ${jsonForScript(opts.clientToken ?? "")} });
      // Ad click ids arrive on this page's own URL (the landing site stamps
      // them onto the buy link -- a different origin, so localStorage cannot
      // cross). They ride through Paddle in customData and come back on the
      // transaction.completed webhook, which is where the conversion is pushed.
      const clickIds = (() => {
        const p = new URLSearchParams(location.search);
        const out = {};
        for (const k of ['gclid', 'gbraid', 'wbraid', 'fbc', 'fbp']) {
          const v = p.get(k);
          if (v) out[k] = v;
        }
        return out;
      })();
      const prices = {
        monthly: ${jsonForScript(opts.priceMonthly ?? "")},
        yearly: ${jsonForScript(opts.priceYearly ?? "")},
      };
      // Undefined unless the edge actually placed the visitor. See the note on
      // BuyOptions.country: a sentinel must never reach Paddle as a country.
      const country = ${jsonForScript(opts.country ?? null)};

      // The cards are rendered with the dollar price, then corrected to the
      // buyer's own currency by Paddle. The figures shown are Paddle's
      // formatted strings exactly as returned -- the totals it will actually
      // charge, tax included, in the currency it will charge them in. Nothing
      // here parses, converts or re-formats a price: the dollar amount is a
      // fallback for a failed request, not an input to arithmetic.
      (async () => {
        const items = Object.entries(prices)
          .filter(([, id]) => id)
          .map(([cycle, id]) => ({ cycle, priceId: id }));
        if (!items.length) return;
        try {
          const preview = await Paddle.PricePreview({
            items: items.map((item) => ({ priceId: item.priceId, quantity: 1 })),
            ...(country ? { address: { countryCode: country } } : {}),
          });
          for (const line of preview.data.details.lineItems) {
            const match = items.find((item) => item.priceId === line.price.id);
            if (!match) continue;
            const cell = document.querySelector('.price[data-cycle="' + match.cycle + '"]');
            if (cell) cell.textContent = line.formattedTotals.total;
          }
        } catch (err) {
          // A failed preview leaves the dollar prices standing. It must never
          // blank the cards or block the buy button: the checkout overlay
          // prices the transaction itself regardless of what this showed.
          console.error('price preview failed', err);
        }
      })();
      const words = ${JSON.stringify({ planUnavailable: s.planUnavailable, checkoutFailed: s.checkoutFailed })};
      document.getElementById('pay').addEventListener('click', async () => {
        const email = document.getElementById('email');
        if (!email.reportValidity()) return;
        const cycle = document.querySelector('.plan input:checked').value;
        if (!prices[cycle]) { alert(words.planUnavailable); return; }
        // The ref is minted server-side so the order row exists before the
        // webhook can arrive — a webhook is quite capable of beating the
        // buyer's redirect back to us.
        const created = await fetch('/checkout/paddle', {
          method: 'POST',
          headers: { 'content-type': 'application/json' },
          body: JSON.stringify({ cycle, email: email.value }),
        }).then((r) => r.json());
        if (!created.ref) { alert(created.error || words.checkoutFailed); return; }
        Paddle.Checkout.open({
          items: [{ priceId: prices[cycle], quantity: 1 }],
          customer: { email: email.value },
          customData: { ref: created.ref, ...clickIds },
          settings: {
            displayMode: 'overlay',
            variant: 'one-page',
            successUrl: ${jsonForScript(withLang(opts.successUrl ?? "", ctx.lang))} + '&ref=' + created.ref,
          },
        });
      });
    </script>`;
  return page(s.proTitle, body, paddleScript, false, ctx);
}


// -- /welcome ----------------------------------------------------------------

/** A Google Ads purchase conversion to report once the order shows as paid. */
export interface Conversion {
  tagId: string;
  sendTo: string;
  /** List price in USD, like the server-side upload in ads.ts. */
  value: number;
}

/** The Google tag, with every consent type denied unless the visitor allowed
 *  it on the marketing site. That choice is a cookie on the parent domain
 *  precisely so it reaches this subdomain; there is no banner here, so no
 *  choice means denied, and Google gets only a cookieless ping. */
function googleTag(tagId: string): string {
  return `<script>
    window.dataLayer = window.dataLayer || [];
    function gtag(){dataLayer.push(arguments);}
    const granted = /(?:^|; )bt_consent=granted(?:;|$)/.test(document.cookie) ? 'granted' : 'denied';
    gtag('consent', 'default', {
      ad_storage: granted, ad_user_data: granted,
      ad_personalization: granted, analytics_storage: granted
    });
    gtag('set', 'ads_data_redaction', true);
    gtag('js', new Date());
    gtag('config', ${jsonForScript(tagId)});
  </script>
  <script async src="https://www.googletagmanager.com/gtag/js?id=${encodeURIComponent(tagId)}"></script>`;
}

export function donePage(
  ctx: PageContext,
  ref: string,
  support: string,
  conversion: Conversion | null = null,
): Response {
  const s = t(ctx.lang);
  // The page is rendered before the outcome is known: the buyer may well
  // arrive back here before the processor's webhook does. So it polls, and
  // says plainly that it is waiting rather than showing an empty box.
  //
  // Everything the script can say is handed to it here, so it says it in the
  // page's language and the escaping happens once, on this side.
  const words = {
    paymentReceived: s.paymentReceived,
    activating: s.activating,
    step1: s.step1,
    step2: s.step2,
    step3: s.step3,
    keepKey: s.keepKey,
    paymentFailed: s.paymentFailed,
    nothingCharged: s.nothingCharged,
    youCan: s.youCan,
    tryAgain: s.tryAgain,
    takingLong: s.takingLong,
    ifCharged: s.ifCharged,
    withReference: s.withReference,
    keyWillBeSent: s.keyWillBeSent,
    buyUrl: withLang("/buy", ctx.lang),
  };
  return page(
    `${s.thankYou} — ${s.proTitle}`,
    `<h1>${s.thankYou}</h1>
     <div id="body">
       <p id="status">${s.confirming}</p>
     </div>
     <script>
       const ref = ${jsonForScript(ref)};
       const support = ${jsonForScript(support)};
       const w = ${jsonForScript(words)};
       const conversion = ${jsonForScript(conversion && { send_to: conversion.sendTo, value: conversion.value })};
       const body = document.getElementById('body');
       let tries = 0;
       async function poll() {
         tries++;
         let order;
         try { order = await fetch('/v1/order/' + ref).then((r) => r.json()); }
         catch { order = null; }
         if (order && order.status === 'paid' && order.key) {
           body.innerHTML =
             '<p class="ok">' + w.paymentReceived + '</p>' +
             '<div class="key">' + order.key + '</div>' +
             '<h2>' + w.activating + '</h2><ol>' +
             '<li>' + w.step1 + '</li>' +
             '<li>' + w.step2 + '</li>' +
             '<li>' + w.step3 + '</li></ol>' +
             '<p class="muted">' + w.keepKey + '</p>';
           // transaction_id is what lets Google drop a reload of this page
           // as a duplicate of the same sale.
           if (conversion && typeof gtag === 'function') {
             gtag('event', 'conversion', {
               send_to: conversion.send_to, value: conversion.value,
               currency: 'USD', transaction_id: ref
             });
           }
           return;
         }
         if (order && order.status === 'failed') {
           body.innerHTML = '<p class="err">' + w.paymentFailed + '</p>' +
             '<p class="muted">' + (order.failure || '') + ' ' + w.nothingCharged + ' ' +
             w.youCan + ' <a href="' + w.buyUrl + '">' + w.tryAgain + '</a>.</p>';
           return;
         }
         if (tries > 40) {
           body.innerHTML = '<p class="warn">' + w.takingLong + '</p>' +
             '<p class="muted">' + w.ifCharged + ' ' + support + ' ' + w.withReference +
             ' <b>' + ref.slice(0, 12) + '</b> ' + w.keyWillBeSent + '</p>';
           return;
         }
         setTimeout(poll, 2000);
       }
       poll();
     </script>`,
    conversion ? googleTag(conversion.tagId) : "",
    false,
    ctx,
  );
}

// -- /account ----------------------------------------------------------------

export interface AccountView {
  key: string;
  plan: string;
  cycle: string;
  status: string;
  renews: string | null;
  seats: number;
  seatLimit: number;
  provider: string;
  cancellable: boolean;
  /** Whether the Paddle-hosted portal can be opened -- true only once a
   *  webhook has told us which Paddle customer this licence belongs to. */
  portal: boolean;
  /** The Paddle customer this licence belongs to, `ctm_…`, or null until a
   *  webhook has said so. Retain is initialised with it and nothing else on
   *  the page needs it. It has to be Paddle's own id: an internal one, or an
   *  email, would initialise cleanly and then attribute every session to
   *  nobody -- worse than leaving Retain switched off. */
  customerId: string | null;
  /** Set when Paddle is scheduled to cancel at the end of the paid period.
   *  Shown, not enforced: the subscription is live until that date. */
  scheduledCancelAt: string | null;
  message?: string;
  error?: string;
}

export function accountPage(
  ctx: PageContext,
  view: AccountView | null,
  support: string,
  error?: string,
  paddle?: { clientToken?: string; env?: "production" | "sandbox" },
): Response {
  const s = t(ctx.lang);
  const form = `
    <form method="post" action="/account">
      <input type="hidden" name="lang" value="${ctx.lang}">
      <label class="field" for="key">${s.keyLabel}</label>
      <input id="key" type="text" name="key" required placeholder="BT-XXXXX-XXXXX-XXXXX"
             value="${escapeHtml(view?.key ?? "")}" autocapitalize="characters" spellcheck="false">
      <button type="submit">${s.lookUp}</button>
    </form>`;
  const title = `${s.yourSubscription} — bubbleTranslate`;

  if (!view) {
    return page(
      title,
      `<h1>${s.yourSubscription}</h1>
       <p>${s.enterKeyToSee}</p>
       ${error ? `<p class="err">${escapeHtml(error)}</p>` : ""}
       ${form}
       <hr>
       <p class="muted">${s.lostKey(escapeHtml(support))}</p>`,
      "",
      false,
      ctx,
    );
  }

  const buyUrl = escapeHtml(withLang("/buy", ctx.lang));
  const cancel = view.cancellable
    ? `<form method="post" action="/account/cancel" style="margin-top:18px">
         <input type="hidden" name="key" value="${escapeHtml(view.key)}">
         <input type="hidden" name="lang" value="${ctx.lang}">
         <button class="quiet" type="submit">${s.cancelSubscription}</button>
         <p class="muted" style="margin-top:8px">
           ${s.keepProUntil(escapeHtml(view.renews ?? s.endOfPaidPeriod))}
         </p>
       </form>`
    : `<p class="muted" style="margin-top:18px">
         ${s.fixedTerm(escapeHtml(view.renews ?? s.itsExpiryDate), buyUrl)}
       </p>`;

  const portal = view.portal
    ? `<form method="post" action="/account/portal" style="margin-top:18px">
         <input type="hidden" name="key" value="${escapeHtml(view.key)}">
         <input type="hidden" name="lang" value="${ctx.lang}">
         <button class="quiet" type="submit">${s.manageBilling}</button>
         <p class="muted" style="margin-top:8px">${s.manageBillingNote}</p>
       </form>`
    : "";

  const status = s.status[view.status] ?? view.status;
  const cycle = s.cycle[view.cycle] ?? view.cycle;

  // Paddle Retain, which does nothing until Paddle.js has been told which
  // customer is reading the page. The id is matched against Paddle's own shape
  // rather than trusted: this is the one page with a real `ctm_…` to hand, and
  // anything else here would be silently wrong rather than loudly broken.
  // Without one, nothing is emitted at all -- no script, no token, no empty
  // Initialize. Retain is a live-only product; on sandbox this initialises and
  // simply has nothing to show, which is documented and not worth branching on.
  const retainId =
    view.customerId && /^ctm_[a-z0-9]+$/.test(view.customerId) ? view.customerId : null;
  const retain =
    retainId && paddle?.clientToken
      ? {
          head: `<script src="https://cdn.paddle.com/paddle/v2/paddle.js"></script>`,
          body: `
     <script>
       Paddle.Environment.set(${jsonForScript(paddle.env ?? "production")});
       Paddle.Initialize({
         token: ${jsonForScript(paddle.clientToken)},
         pwCustomer: { id: ${jsonForScript(retainId)} },
       });
     </script>`,
        }
      : { head: "", body: "" };

  return page(
    title,
    `<h1>${s.yourSubscription}</h1>
     ${view.message ? `<p class="ok">${escapeHtml(view.message)}</p>` : ""}
     ${view.error ? `<p class="err">${escapeHtml(view.error)}</p>` : ""}
     <h2>${escapeHtml(view.plan)} — ${escapeHtml(cycle)}</h2>
     <p class="muted">
       ${s.statusLabel}: <b class="${view.status === "active" ? "ok" : "warn"}">${escapeHtml(status)}</b><br>
       ${view.renews ? `${s.ends} ${escapeHtml(view.renews)}<br>` : ""}
       ${s.devices(view.seats, view.seatLimit)}<br>
       ${s.paidThrough} Paddle
     </p>
     ${view.scheduledCancelAt ? `<p class="muted">${s.scheduledToCancel(escapeHtml(view.scheduledCancelAt))}</p>` : ""}
     ${portal}
     ${cancel}
     <hr>
     ${form}
     <p class="muted">${s.questions}: ${escapeHtml(support)}</p>${retain.body}`,
    retain.head,
    false,
    ctx,
  );
}
