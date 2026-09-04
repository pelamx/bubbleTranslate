// The four pages this service serves to a browser.
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

export const escapeHtml = (value: unknown): string =>
  String(value ?? "")
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");

/** Lira, from the integer kuruş the rest of the service deals in. */
export const lira = (kurus: number) =>
  `₺${(kurus / 100).toLocaleString("tr-TR", { minimumFractionDigits: 2, maximumFractionDigits: 2 })}`;

const STYLE = `
  :root { color-scheme: dark; }
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
  label.field { display: block; font-size: 13px; color: #b8b8b8; margin: 0 0 6px; }
  input[type=text], input[type=email] {
    width: 100%; padding: 11px 12px; border-radius: 8px; border: 1px solid #3a3c41;
    background: #17181a; color: #f0f0f0; font-size: 15px; font-family: inherit;
  }
  input:focus { outline: 2px solid #78d28c; outline-offset: -1px; }
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
`;

export function page(title: string, body: string, head = "", wide = false): Response {
  return new Response(
    `<!doctype html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>${escapeHtml(title)}</title><style>${STYLE}</style>${head}</head>
<body><div class="sheet${wide ? " wide" : ""}">${body}</div></body></html>`,
    { headers: { "content-type": "text/html; charset=utf-8" } },
  );
}

const planCard = (cycle: Cycle, price: string, checked: boolean, note: string) => `
  <label class="plan${checked ? " on" : ""}">
    <input type="radio" name="cycle" value="${cycle}"${checked ? " checked" : ""}>
    <div class="name">${cycle === "yearly" ? "Yearly" : "Monthly"}</div>
    <div class="price">${escapeHtml(price)}</div>
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
  turkey: boolean;
  configured: boolean;
  reason?: string;
  monthly: string;
  yearly: string;
  src: string;
  otherUrl: string;
  /** Paddle only. */
  clientToken?: string;
  paddleEnv?: string;
  priceMonthly?: string;
  priceYearly?: string;
  successUrl?: string;
}

export function buyPage(opts: BuyOptions): Response {
  if (!opts.configured) {
    return page(
      "bubbleTranslate Pro",
      `<h1>bubbleTranslate Pro</h1>
       <p class="warn">Checkout is not available yet.</p>
       <p class="muted">${escapeHtml(opts.reason ?? "The payment provider for your region is not configured.")}</p>`,
    );
  }

  const savings =
    opts.turkey
      ? "12 ay, tek ödeme"
      : "two months free";
  // The plan cards are radio inputs, so on the PayTR page they have to sit
  // *inside* the form that posts them — a radio outside the form it belongs to
  // is simply not submitted, and the server would be left guessing which plan
  // was bought. Hence `heading` and `plans` separately rather than one block:
  // the Paddle page reads the selection with JavaScript and does not care, but
  // this one is a plain form post and cares a great deal.
  const heading = `
    <h1>bubbleTranslate Pro</h1>
    <p>${
      opts.turkey
        ? "Sınırsız çeviri, üç cihaz. Ödeme PayTR üzerinden alınır."
        : "Unlimited translations, three devices. Billed through Paddle."
    }</p>`;
  const plans = `
    <div class="plans">
      ${planCard("monthly", opts.monthly, false, opts.turkey ? "aylık" : "billed monthly")}
      ${planCard("yearly", opts.yearly, true, savings)}
    </div>`;

  const footer = `
    <hr>
    <p class="muted">${
      opts.turkey
        ? `Türkiye dışındaysanız <a href="${escapeHtml(opts.otherUrl)}">dolar üzerinden ödeyebilirsiniz</a>.`
        : `In Turkey? <a href="${escapeHtml(opts.otherUrl)}">Pay in lira instead</a>.`
    }</p>`;

  if (opts.turkey) {
    return page(
      "bubbleTranslate Pro",
      `${heading}
       <form method="post" action="/checkout/paytr">
         ${plans}
         <input type="hidden" name="src" value="${escapeHtml(opts.src)}">
         <label class="field" for="email">E-posta adresiniz — lisans anahtarı buraya gönderilir</label>
         <input id="email" type="email" name="email" required autocomplete="email"
                placeholder="siz@ornek.com">
         <button type="submit">Ödemeye geç</button>
       </form>
       <p class="muted" style="margin-top:14px">
         Kart bilgileriniz PayTR'ye gider, bu sunucuya değil. Satın alma tek seferliktir
         ve süre sonunda kendiliğinden yenilenmez.
       </p>
       ${footer}
       <script>${PLAN_SCRIPT}</script>`,
    );
  }

  // Paddle. The overlay wants the price id, and carries our order ref through
  // to the webhook in `customData` — that ref is how the success page knows
  // which licence to reveal.
  const paddleScript = `
    <script src="https://cdn.paddle.com/paddle/v2/paddle.js"></script>`;
  const body = `
    ${heading}
    ${plans}
    <label class="field" for="email">Your email — the licence key is sent here</label>
    <input id="email" type="email" required autocomplete="email" placeholder="you@example.com">
    <button id="pay" type="button">Continue to payment</button>
    <p class="muted" style="margin-top:14px">
      Card details go to Paddle, not to this server. Paddle is the merchant of
      record and handles VAT and invoicing.
    </p>
    ${footer}
    <script>
      ${PLAN_SCRIPT}
      Paddle.Environment.set(${JSON.stringify(opts.paddleEnv === "production" ? "production" : "sandbox")});
      Paddle.Initialize({ token: ${JSON.stringify(opts.clientToken ?? "")} });
      const prices = {
        monthly: ${JSON.stringify(opts.priceMonthly ?? "")},
        yearly: ${JSON.stringify(opts.priceYearly ?? "")},
      };
      document.getElementById('pay').addEventListener('click', async () => {
        const email = document.getElementById('email');
        if (!email.reportValidity()) return;
        const cycle = document.querySelector('.plan input:checked').value;
        if (!prices[cycle]) { alert('That plan is not available yet.'); return; }
        // The ref is minted server-side so the order row exists before the
        // webhook can arrive — a webhook is quite capable of beating the
        // buyer's redirect back to us.
        const created = await fetch('/checkout/paddle', {
          method: 'POST',
          headers: { 'content-type': 'application/json' },
          body: JSON.stringify({ cycle, email: email.value }),
        }).then((r) => r.json());
        if (!created.ref) { alert(created.error || 'Could not start checkout.'); return; }
        Paddle.Checkout.open({
          items: [{ priceId: prices[cycle], quantity: 1 }],
          customer: { email: email.value },
          customData: { ref: created.ref },
          settings: {
            successUrl: ${JSON.stringify(opts.successUrl ?? "")} + '?ref=' + created.ref,
          },
        });
      });
    </script>`;
  return page("bubbleTranslate Pro", body, paddleScript);
}

// -- the PayTR iframe --------------------------------------------------------

export function paytrPage(iframeSrc: string, ref: string): Response {
  return page(
    "Ödeme — bubbleTranslate Pro",
    `<h1>Ödeme</h1>
     <p class="muted">Sipariş ${escapeHtml(ref.slice(0, 12))}</p>
     <iframe src="${escapeHtml(iframeSrc)}"
             id="paytriframe" frameborder="0" scrolling="no"></iframe>
     <script src="https://www.paytr.com/js/iframeResizer.min.js"></script>
     <script>iFrameResize({}, '#paytriframe');</script>`,
  );
}

// -- /done -------------------------------------------------------------------

export function donePage(ref: string, support: string): Response {
  // The page is rendered before the outcome is known: the buyer may well
  // arrive back here before the processor's webhook does. So it polls, and
  // says plainly that it is waiting rather than showing an empty box.
  return page(
    "Thank you — bubbleTranslate Pro",
    `<h1>Thank you</h1>
     <div id="body">
       <p id="status">Confirming your payment…</p>
     </div>
     <script>
       const ref = ${JSON.stringify(ref)};
       const support = ${JSON.stringify(support)};
       const body = document.getElementById('body');
       let tries = 0;
       async function poll() {
         tries++;
         let order;
         try { order = await fetch('/v1/order/' + ref).then((r) => r.json()); }
         catch { order = null; }
         if (order && order.status === 'paid' && order.key) {
           body.innerHTML =
             '<p class="ok">Payment received. Here is your licence key:</p>' +
             '<div class="key">' + order.key + '</div>' +
             '<h2>Activating it</h2><ol>' +
             '<li>Open bubbleTranslate.</li>' +
             '<li>Go to the <b>Account</b> section.</li>' +
             '<li>Paste the key and choose <b>Activate</b>.</li></ol>' +
             '<p class="muted">It works on up to three machines. Keep this key — ' +
             'this page stops showing it after an hour.</p>';
           return;
         }
         if (order && order.status === 'failed') {
           body.innerHTML = '<p class="err">The payment did not go through.</p>' +
             '<p class="muted">' + (order.failure || '') + ' Nothing was charged. ' +
             'You can <a href="/buy">try again</a>.</p>';
           return;
         }
         if (tries > 40) {
           body.innerHTML = '<p class="warn">This is taking longer than expected.</p>' +
             '<p class="muted">If you were charged, email ' + support +
             ' with reference <b>' + ref.slice(0, 12) + '</b> and the key will be sent to you.</p>';
           return;
         }
         setTimeout(poll, 2000);
       }
       poll();
     </script>`,
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
  message?: string;
  error?: string;
}

export function accountPage(view: AccountView | null, support: string, error?: string): Response {
  const form = `
    <form method="post" action="/account">
      <label class="field" for="key">Your licence key</label>
      <input id="key" type="text" name="key" required placeholder="BT-XXXXX-XXXXX-XXXXX"
             value="${escapeHtml(view?.key ?? "")}" autocapitalize="characters" spellcheck="false">
      <button type="submit">Look it up</button>
    </form>`;

  if (!view) {
    return page(
      "Your subscription — bubbleTranslate",
      `<h1>Your subscription</h1>
       <p>Enter the key you were sent to see its status.</p>
       ${error ? `<p class="err">${escapeHtml(error)}</p>` : ""}
       ${form}
       <hr>
       <p class="muted">Lost the key? Email ${escapeHtml(support)} from the address you
       bought with.</p>`,
    );
  }

  const cancel = view.cancellable
    ? `<form method="post" action="/account/cancel" style="margin-top:18px">
         <input type="hidden" name="key" value="${escapeHtml(view.key)}">
         <button class="quiet" type="submit">Cancel subscription</button>
         <p class="muted" style="margin-top:8px">
           You keep Pro until ${escapeHtml(view.renews ?? "the end of the paid period")}.
         </p>
       </form>`
    : `<p class="muted" style="margin-top:18px">
         This is a fixed-term licence — there is no recurring charge to cancel.
         It simply ends on ${escapeHtml(view.renews ?? "its expiry date")}, and you can
         <a href="/buy">buy another term</a> whenever you like.
       </p>`;

  return page(
    "Your subscription — bubbleTranslate",
    `<h1>Your subscription</h1>
     ${view.message ? `<p class="ok">${escapeHtml(view.message)}</p>` : ""}
     ${view.error ? `<p class="err">${escapeHtml(view.error)}</p>` : ""}
     <h2>${escapeHtml(view.plan)} — ${escapeHtml(view.cycle)}</h2>
     <p class="muted">
       Status: <b class="${view.status === "active" ? "ok" : "warn"}">${escapeHtml(view.status)}</b><br>
       ${view.renews ? `Ends ${escapeHtml(view.renews)}<br>` : ""}
       Devices: ${view.seats} of ${view.seatLimit} in use<br>
       Paid through ${escapeHtml(view.provider === "paytr" ? "PayTR" : "Paddle")}
     </p>
     ${cancel}
     <hr>
     ${form}
     <p class="muted">Questions: ${escapeHtml(support)}</p>`,
  );
}
