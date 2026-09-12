// The four pages this service serves to a browser, in three languages.
//
// They are here rather than on a separate site for one reason: /buy is the
// only page that has to know which country the visitor is in, and this Worker
// is already the thing behind Cloudflare that gets told. A static site would
// have to ask this service anyway, and then the routing would live in two
// places instead of one.
//
// Everything is inline — no build step, no CDN, no fonts to fetch. Paddle.js
// is the single external script, and only on the page that opens a Paddle
// checkout.

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

/** The marketing site, which is where the policies live. Paddle's domain
 *  review fetches the *checkout* domain -- this service -- so the terms,
 *  privacy notice and refund policy have to be reachable from here too, not
 *  only from the site they are written on. */
export const SITE = "https://bubbletranslate.app";

const STYLE = `
  :root { color-scheme: dark; }
  footer.legal { max-width: 620px; margin: 26px auto 40px; padding: 0 18px; text-align: center;
    font-size: .82rem; display: flex; gap: 16px; justify-content: center; flex-wrap: wrap; }
  footer.legal a { color: #9fb0d4; text-decoration: none; }
  footer.legal a:hover { color: #eaf0ff; text-decoration: underline; }
  * { box-sizing: border-box; }
  body {
    margin: 0; padding: 40px 20px; background: #1e1f22; color: #f0f0f0;
    font: 15px/1.55 -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto,
          "Helvetica Neue", Arial, sans-serif;
    display: flex; justify-content: center;
  }
  .sheet { width: 100%; max-width: 620px; }
  .sheet.wide { max-width: 1040px; }
  table { width: 100%; border-collapse: collapse; font-size: 13px; margin: 6px 0 4px; }
  th, td { text-align: left; padding: 8px 10px; border-bottom: 1px solid #2e3034; vertical-align: top; }
  th { color: #9a9a9a; font-weight: 600; font-size: 12px; }
  td.num, th.num { text-align: right; font-variant-numeric: tabular-nums; }
  .scroll { overflow-x: auto; }
  .tiles { display: grid; gap: 10px; grid-template-columns: repeat(auto-fit, minmax(130px, 1fr)); margin: 18px 0; }
  .tile { background: #242629; border: 1px solid #3a3c41; border-radius: 10px; padding: 12px 14px; }
  .tile .n { font-size: 22px; font-weight: 600; font-variant-numeric: tabular-nums; }
  .tile .l { font-size: 12px; color: #9a9a9a; }
  .row { display: flex; gap: 8px; align-items: flex-end; flex-wrap: wrap; }
  .row > * { margin-top: 0; }
  .row button, .row input { width: auto; }
  form.inline { display: inline; }
  form.inline button { width: auto; padding: 5px 10px; font-size: 12px; margin: 0 2px 0 0; }
  code { font: 12px ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; color: #cfcfcf; }
  h1 { font-size: 22px; margin: 0 0 6px; font-weight: 600; }
  h2 { font-size: 15px; margin: 28px 0 10px; font-weight: 600; color: #dcdcdc; }
  p { margin: 0 0 14px; color: #b8b8b8; }
  .muted { color: #9a9a9a; font-size: 13px; }
  .plans { display: grid; gap: 12px; grid-template-columns: 1fr 1fr; margin: 22px 0 18px; }
  @media (max-width: 520px) { .plans { grid-template-columns: 1fr; } }
  .plan {
    border: 1px solid #3a3c41; border-radius: 10px; padding: 16px; cursor: pointer;
    background: #242629; display: block; position: relative;
  }
  .plan.on { border-color: #78d28c; background: #26302a; }
  .plan input { position: absolute; opacity: 0; pointer-events: none; }
  .plan .name { font-size: 13px; color: #b8b8b8; }
  .plan .price { font-size: 24px; font-weight: 600; margin: 4px 0 2px; }
  .plan .note { font-size: 12px; color: #78d28c; }
  .tiers { display: grid; gap: 12px; grid-template-columns: 1fr 1fr; margin: 18px 0 6px; }
  @media (max-width: 520px) { .tiers { grid-template-columns: 1fr; } }
  .tier { border: 1px solid #3a3c41; border-radius: 10px; padding: 12px 14px; font-size: 13px; }
  .tier .name { color: #b8b8b8; font-weight: 600; margin-bottom: 4px; }
  .tier.pro { border-color: #78d28c; }
  .tier.pro .name { color: #78d28c; }
  label.field { display: block; font-size: 13px; color: #b8b8b8; margin: 0 0 6px; }
  input[type=text], input[type=email], input[type=number], input[type=date], select {
    width: 100%; padding: 11px 12px; border-radius: 8px; border: 1px solid #3a3c41;
    background: #17181a; color: #f0f0f0; font-size: 15px; font-family: inherit;
  }
  input:focus, select:focus { outline: 2px solid #78d28c; outline-offset: -1px; }
  /* The admin edit panel: folded away so the listing stays a listing. */
  tr.editrow td { border-bottom: 1px solid #2e3034; padding-top: 0; }
  tr.editrow details > summary {
    cursor: pointer; color: #9a9a9a; font-size: 12px; padding: 2px 0; list-style: none;
  }
  tr.editrow details > summary::-webkit-details-marker { display: none; }
  tr.editrow details > summary::before { content: "▸ "; }
  tr.editrow details[open] > summary::before { content: "▾ "; }
  tr.editrow details[open] > summary { color: #dcdcdc; margin-bottom: 10px; }
  .row.edit { margin-bottom: 12px; }
  button.danger { background: #4a2326; color: #ffb4b4; }
  button {
    width: 100%; padding: 12px 16px; border-radius: 8px; border: 0; cursor: pointer;
    background: #78d28c; color: #14261a; font-size: 15px; font-weight: 600;
    font-family: inherit; margin-top: 14px;
  }
  button:disabled { opacity: .55; cursor: default; }
  button.quiet { background: #33363b; color: #e8e8e8; }
  .key {
    font: 20px/1.4 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    letter-spacing: .06em; background: #17181a; border: 1px solid #3a3c41;
    border-radius: 8px; padding: 16px; text-align: center; user-select: all;
    margin: 8px 0 14px; word-break: break-all;
  }
  .warn { color: #f5c382; }
  .err { color: #ff9696; }
  .ok { color: #78d28c; }
  hr { border: 0; border-top: 1px solid #33363b; margin: 26px 0; }
  a { color: #9fd8ad; }
  ol { color: #b8b8b8; padding-left: 20px; }
  li { margin-bottom: 6px; }
  iframe { width: 100%; border: 0; min-height: 720px; }
  .langs {
    position: fixed; top: 14px; right: 18px; display: flex; gap: 4px;
    font-size: 12px; letter-spacing: .04em;
  }
  .langs a {
    color: #9a9a9a; text-decoration: none; padding: 4px 7px; border-radius: 6px;
    border: 1px solid transparent;
  }
  .langs a:hover { color: #f0f0f0; }
  .langs a.on { color: #78d28c; border-color: #3a3c41; background: #242629; }
`;

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
    headers["set-cookie"] = `lang=${lang}; Path=/; Max-Age=31536000; SameSite=Lax`;
  }
  return new Response(
    `<!doctype html><html lang="${lang}"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>${escapeHtml(title)}</title><style>${STYLE}</style>${head}</head>
<body>${switcher}<div class="sheet${wide ? " wide" : ""}">${body}</div>${legalFooter(lang)}</body></html>`,
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
  const body = `
    ${heading}
    ${plans}
    <label class="field" for="email">${s.emailLabel}</label>
    <input id="email" type="email" required autocomplete="email" placeholder="${escapeHtml(s.emailPlaceholder)}">
    <button id="pay" type="button">${s.continueToPayment}</button>
    <p class="muted" style="margin-top:14px">${s.paddleNote}</p>
    <script>
      ${PLAN_SCRIPT}
      Paddle.Environment.set(${JSON.stringify(opts.paddleEnv)});
      Paddle.Initialize({ token: ${JSON.stringify(opts.clientToken ?? "")} });
      const prices = {
        monthly: ${JSON.stringify(opts.priceMonthly ?? "")},
        yearly: ${JSON.stringify(opts.priceYearly ?? "")},
      };
      // Undefined unless the edge actually placed the visitor. See the note on
      // BuyOptions.country: a sentinel must never reach Paddle as a country.
      const country = ${JSON.stringify(opts.country ?? null)};

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
          customData: { ref: created.ref },
          settings: {
            displayMode: 'overlay',
            variant: 'one-page',
            successUrl: ${JSON.stringify(withLang(opts.successUrl ?? "", ctx.lang))} + '&ref=' + created.ref,
          },
        });
      });
    </script>`;
  return page(s.proTitle, body, paddleScript, false, ctx);
}


// -- /welcome ----------------------------------------------------------------

export function donePage(ctx: PageContext, ref: string, support: string): Response {
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
       const ref = ${JSON.stringify(ref)};
       const support = ${JSON.stringify(support)};
       const w = ${JSON.stringify(words)};
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
    "",
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
       Paddle.Environment.set(${JSON.stringify(paddle.env ?? "production")});
       Paddle.Initialize({
         token: ${JSON.stringify(paddle.clientToken)},
         pwCustomer: { id: ${JSON.stringify(retainId)} },
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
