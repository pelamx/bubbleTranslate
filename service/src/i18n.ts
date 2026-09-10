// The three languages the pages speak, and how one is chosen.
//
// Language and payment processor are two different questions. Where the
// visitor *is* decides whether they see PayTR or Paddle (see `inTurkey` in
// index.ts); what they *read* is theirs to pick, from the switcher at the top
// right of every page. English is the default for everyone: the app itself is
// in English, and a page that guesses a language from an IP address is wrong
// for every traveller, expatriate and VPN user at once.
//
// The choice is carried three ways so it survives the whole journey: as
// `?lang=` on links, as a hidden field in forms, and as a cookie for the
// return trip from a payment processor, which comes back on a URL this
// service wrote before the buyer left.

export type Lang = "en" | "tr" | "es";

export const LANGS: readonly Lang[] = ["en", "tr", "es"];
export const DEFAULT_LANG: Lang = "en";

export const isLang = (value: unknown): value is Lang =>
  typeof value === "string" && (LANGS as readonly string[]).includes(value);

/** `?lang=` wins, then the cookie, then English. Never the IP address. */
export function pickLang(request: Request, url: URL): Lang {
  const fromQuery = url.searchParams.get("lang")?.toLowerCase();
  if (isLang(fromQuery)) return fromQuery;
  const cookie = request.headers.get("cookie") ?? "";
  const match = /(?:^|;\s*)lang=([a-z]{2})/.exec(cookie);
  if (match && isLang(match[1])) return match[1];
  return DEFAULT_LANG;
}

/** A relative link with the language carried along. */
export function withLang(path: string, lang: Lang): string {
  const joiner = path.includes("?") ? "&" : "?";
  return `${path}${joiner}lang=${lang}`;
}

/** The same URL, in another language — what the switcher links to. */
export function switchedTo(url: URL, lang: Lang): string {
  const next = new URL(url.toString());
  next.searchParams.set("lang", lang);
  return `${next.pathname}${next.search}`;
}

// -- the strings -------------------------------------------------------------
//
// One object per language, checked against the English one by the type below,
// so adding a string to English without its two translations is a compile
// error rather than a blank spot on the Turkish page.

const en = {
  langName: "English",
  proTitle: "bubbleTranslate Pro",

  // /buy
  checkoutUnavailable: "Checkout is not available yet.",
  providerNotConfigured: "The payment provider for your region is not configured.",
  reasonPaytrUnconfigured:
    "PayTR is not configured on this server yet. Set the merchant credentials and the lira prices, or pay in dollars instead.",
  reasonPaddleUnconfigured: "Paddle is not configured on this server yet.",
  introPaytr:
    "The free version is limited to ten translations a day. Pro removes the limit; billed in lira through PayTR.",
  introPaddle:
    "The free version is limited to ten translations a day. Pro removes the limit; billed through Paddle.",
  tierFreeName: "Free",
  tierFree: (n: number) =>
    `${n} translations a day, one machine. The count comes back at midnight; past it, you need Pro to keep going.`,
  tierProName: "Pro",
  tierPro: "Unlimited translations, three devices. No daily limit.",
  monthly: "Monthly",
  yearly: "Yearly",
  billedMonthly: "billed monthly",
  yearlyNotePaddle: "two months free",
  yearlyNotePaytr: "12 months, one payment",
  emailLabel: "Your email — the licence key is sent here",
  emailPlaceholder: "you@example.com",
  continueToPayment: "Continue to payment",
  paytrNote:
    "Card details go to PayTR, not to this server. The purchase is a single payment and does not renew by itself when the term ends.",
  paddleNote:
    "Card details go to Paddle, not to this server. Paddle is the merchant of record and handles VAT and invoicing.",
  footerTurkey: (url: string) => `Outside Turkey? <a href="${url}">Pay in dollars instead</a>.`,
  footerOther: (url: string) => `In Turkey? <a href="${url}">Pay in lira instead</a>.`,
  planUnavailable: "That plan is not available yet.",
  checkoutFailed: "Could not start checkout.",

  // the PayTR frame
  paymentTitle: "Payment",
  order: "Order",

  // /done
  thankYou: "Thank you",
  confirming: "Confirming your payment…",
  paymentReceived: "Payment received. Here is your licence key:",
  activating: "Activating it",
  step1: "Open bubbleTranslate.",
  step2: "Go to the <b>Account</b> section.",
  step3: "Paste the key and choose <b>Activate</b>.",
  keepKey: "It works on up to three machines. Keep this key — this page stops showing it after an hour.",
  paymentFailed: "The payment did not go through.",
  nothingCharged: "Nothing was charged.",
  tryAgain: "try again",
  youCan: "You can",
  takingLong: "This is taking longer than expected.",
  ifCharged: "If you were charged, email",
  withReference: "with reference",
  keyWillBeSent: "and the key will be sent to you.",

  // /account
  yourSubscription: "Your subscription",
  enterKeyToSee: "Enter the key you were sent to see its status.",
  keyLabel: "Your licence key",
  lookUp: "Look it up",
  lostKey: (support: string) => `Lost the key? Email ${support} from the address you bought with.`,
  cancelSubscription: "Cancel subscription",
  keepProUntil: (date: string) => `You keep Pro until ${date}.`,
  endOfPaidPeriod: "the end of the paid period",
  fixedTerm: (date: string, buyUrl: string) =>
    `This is a fixed-term licence — there is no recurring charge to cancel. It simply ends on ${date}, and you can <a href="${buyUrl}">buy another term</a> whenever you like.`,
  itsExpiryDate: "its expiry date",
  statusLabel: "Status",
  ends: "Ends",
  devices: (used: number, limit: number) => `Devices: ${used} of ${limit} in use`,
  paidThrough: "Paid through",
  questions: "Questions",
  status: { active: "active", expired: "expired", cancelled: "cancelled", refunded: "refunded" } as Record<string, string>,
  cycle: { monthly: "monthly", yearly: "yearly" } as Record<string, string>,

  // messages the handlers put on the pages
  enterYourKey: "Enter your licence key.",
  keyNotRecognised: "That licence key was not recognised.",
  noRecurringCharge: "This licence has no recurring charge to cancel.",
  cancelled: (date: string) => `Cancelled. Pro keeps working until ${date}.`,
  cancelNotConfigured: "Subscription management is not configured on this server.",
  manageBilling: "Manage billing",
  manageBillingNote:
    "Opens Paddle, where you can update your card, download invoices and change your plan.",
  portalNotAvailable: "Billing management is not available for this licence yet.",
  paddleRefusedPortal: "Paddle could not open the billing portal. Please contact support.",
  scheduledToCancel: (date: string) => `Scheduled to cancel on ${date}. Pro works until then.`,
  paddleRefusedCancel: "Paddle could not cancel this subscription. Please contact support.",
  paddleUnreachable: "Could not reach Paddle. Please try again shortly.",

  // plain-text replies from the PayTR form post
  invalidPlan: "Invalid plan.",
  invalidEmail: "Enter a valid email address.",
  paytrNotConfigured: "PayTR is not configured.",
  priceNotSet: "This plan has no price set.",
  paymentCouldNotStart: "The payment could not be started.",
  paytrUnreachable: "Could not reach the payment provider. Please try again.",
  paymentIncomplete: "The payment was not completed.",
};

export type Strings = typeof en;

const tr: Strings = {
  langName: "Türkçe",
  proTitle: "bubbleTranslate Pro",

  checkoutUnavailable: "Ödeme henüz kullanılamıyor.",
  providerNotConfigured: "Bölgeniz için ödeme sağlayıcısı ayarlanmamış.",
  reasonPaytrUnconfigured:
    "PayTR bu sunucuda henüz ayarlanmadı. Mağaza bilgilerini ve lira fiyatlarını girin ya da dolar üzerinden ödeyin.",
  reasonPaddleUnconfigured: "Paddle bu sunucuda henüz ayarlanmadı.",
  introPaytr:
    "Ücretsiz sürüm günde on çeviriyle sınırlıdır. Pro sınırı kaldırır; ödeme lira olarak PayTR üzerinden alınır.",
  introPaddle:
    "Ücretsiz sürüm günde on çeviriyle sınırlıdır. Pro sınırı kaldırır; ödeme Paddle üzerinden alınır.",
  tierFreeName: "Ücretsiz",
  tierFree: (n: number) =>
    `Günde ${n} çeviri, tek cihaz. Sayaç her gece yarısı sıfırlanır; sonrasında devam etmek için Pro gerekir.`,
  tierProName: "Pro",
  tierPro: "Sınırsız çeviri, üç cihaz. Günlük limit yok.",
  monthly: "Aylık",
  yearly: "Yıllık",
  billedMonthly: "aylık",
  yearlyNotePaddle: "iki ay bedava",
  yearlyNotePaytr: "12 ay, tek ödeme",
  emailLabel: "E-posta adresiniz — lisans anahtarı buraya gönderilir",
  emailPlaceholder: "siz@ornek.com",
  continueToPayment: "Ödemeye geç",
  paytrNote:
    "Kart bilgileriniz PayTR'ye gider, bu sunucuya değil. Satın alma tek seferliktir ve süre sonunda kendiliğinden yenilenmez.",
  paddleNote:
    "Kart bilgileriniz Paddle'a gider, bu sunucuya değil. Satıcı Paddle'dır; KDV ve faturayı o düzenler.",
  footerTurkey: (url: string) => `Türkiye dışındaysanız <a href="${url}">dolar üzerinden ödeyebilirsiniz</a>.`,
  footerOther: (url: string) => `Türkiye'de misiniz? <a href="${url}">Lira ile ödeyin</a>.`,
  planUnavailable: "Bu plan henüz kullanılamıyor.",
  checkoutFailed: "Ödeme başlatılamadı.",

  paymentTitle: "Ödeme",
  order: "Sipariş",

  thankYou: "Teşekkürler",
  confirming: "Ödemeniz doğrulanıyor…",
  paymentReceived: "Ödeme alındı. Lisans anahtarınız:",
  activating: "Etkinleştirme",
  step1: "bubbleTranslate'i açın.",
  step2: "<b>Account</b> bölümüne gidin.",
  step3: "Anahtarı yapıştırıp <b>Activate</b>'e basın.",
  keepKey: "En fazla üç cihazda çalışır. Anahtarı saklayın — bu sayfa bir saat sonra onu göstermeyi bırakır.",
  paymentFailed: "Ödeme gerçekleşmedi.",
  nothingCharged: "Hiçbir ücret alınmadı.",
  tryAgain: "tekrar deneyebilirsiniz",
  youCan: "İsterseniz",
  takingLong: "Bu beklenenden uzun sürüyor.",
  ifCharged: "Ücret alındıysa",
  withReference: "referansıyla",
  keyWillBeSent: "adresine yazın; anahtar size gönderilir.",

  yourSubscription: "Aboneliğiniz",
  enterKeyToSee: "Durumunu görmek için size gönderilen anahtarı girin.",
  keyLabel: "Lisans anahtarınız",
  lookUp: "Sorgula",
  lostKey: (support: string) => `Anahtarı mı kaybettiniz? Satın aldığınız adresten ${support} adresine yazın.`,
  cancelSubscription: "Aboneliği iptal et",
  keepProUntil: (date: string) => `Pro ${date} tarihine kadar sizde kalır.`,
  endOfPaidPeriod: "ödenen dönemin sonu",
  fixedTerm: (date: string, buyUrl: string) =>
    `Bu sabit süreli bir lisans — iptal edilecek yinelenen bir ücret yok. ${date} tarihinde kendiliğinden biter; dilediğiniz zaman <a href="${buyUrl}">yeni bir dönem satın alabilirsiniz</a>.`,
  itsExpiryDate: "bitiş tarihi",
  statusLabel: "Durum",
  ends: "Bitiş",
  devices: (used: number, limit: number) => `Cihazlar: ${limit} cihazdan ${used} tanesi kullanımda`,
  paidThrough: "Ödeme yolu:",
  questions: "Sorularınız için",
  status: { active: "etkin", expired: "süresi dolmuş", cancelled: "iptal edilmiş", refunded: "iade edilmiş" },
  cycle: { monthly: "aylık", yearly: "yıllık" },

  enterYourKey: "Lisans anahtarınızı girin.",
  keyNotRecognised: "Bu lisans anahtarı tanınmadı.",
  noRecurringCharge: "Bu lisansın iptal edilecek yinelenen bir ücreti yok.",
  cancelled: (date: string) => `İptal edildi. Pro ${date} tarihine kadar çalışmaya devam eder.`,
  cancelNotConfigured: "Abonelik yönetimi bu sunucuda ayarlanmamış.",
  manageBilling: "Faturalandırmayı yönet",
  manageBillingNote:
    "Kartınızı güncelleyebileceğiniz, faturalarınızı indirebileceğiniz ve planınızı değiştirebileceğiniz Paddle sayfasını açar.",
  portalNotAvailable: "Bu lisans için faturalandırma yönetimi henüz kullanılamıyor.",
  paddleRefusedPortal: "Paddle faturalandırma portalını açamadı. Lütfen destek ile iletişime geçin.",
  scheduledToCancel: (date: string) => `${date} tarihinde iptal edilecek. Pro o tarihe kadar çalışır.`,
  paddleRefusedCancel: "Paddle bu aboneliği iptal edemedi. Lütfen destekle iletişime geçin.",
  paddleUnreachable: "Paddle'a ulaşılamadı. Lütfen kısa süre sonra tekrar deneyin.",

  invalidPlan: "Geçersiz plan.",
  invalidEmail: "Geçerli bir e-posta adresi girin.",
  paytrNotConfigured: "PayTR yapılandırılmamış.",
  priceNotSet: "Bu planın fiyatı ayarlanmamış.",
  paymentCouldNotStart: "Ödeme başlatılamadı.",
  paytrUnreachable: "Ödeme sağlayıcısına ulaşılamadı. Lütfen tekrar deneyin.",
  paymentIncomplete: "Ödeme tamamlanamadı.",
};

const es: Strings = {
  langName: "Español",
  proTitle: "bubbleTranslate Pro",

  checkoutUnavailable: "El pago aún no está disponible.",
  providerNotConfigured: "El proveedor de pagos de tu región no está configurado.",
  reasonPaytrUnconfigured:
    "PayTR aún no está configurado en este servidor. Configura las credenciales del comercio y los precios en liras, o paga en dólares.",
  reasonPaddleUnconfigured: "Paddle aún no está configurado en este servidor.",
  introPaytr:
    "La versión gratuita está limitada a diez traducciones al día. Pro elimina el límite; se cobra en liras a través de PayTR.",
  introPaddle:
    "La versión gratuita está limitada a diez traducciones al día. Pro elimina el límite; se cobra a través de Paddle.",
  tierFreeName: "Gratis",
  tierFree: (n: number) =>
    `${n} traducciones al día, un equipo. El contador se reinicia a medianoche; a partir de ahí necesitas Pro para seguir.`,
  tierProName: "Pro",
  tierPro: "Traducciones ilimitadas, tres dispositivos. Sin límite diario.",
  monthly: "Mensual",
  yearly: "Anual",
  billedMonthly: "cobro mensual",
  yearlyNotePaddle: "dos meses gratis",
  yearlyNotePaytr: "12 meses, un solo pago",
  emailLabel: "Tu correo — la clave de licencia se envía aquí",
  emailPlaceholder: "tu@ejemplo.com",
  continueToPayment: "Continuar al pago",
  paytrNote:
    "Los datos de la tarjeta van a PayTR, no a este servidor. La compra es un pago único y no se renueva sola al terminar el periodo.",
  paddleNote:
    "Los datos de la tarjeta van a Paddle, no a este servidor. Paddle es el vendedor registrado y se encarga del IVA y la facturación.",
  footerTurkey: (url: string) => `¿Fuera de Turquía? <a href="${url}">Paga en dólares</a>.`,
  footerOther: (url: string) => `¿En Turquía? <a href="${url}">Paga en liras</a>.`,
  planUnavailable: "Ese plan aún no está disponible.",
  checkoutFailed: "No se pudo iniciar el pago.",

  paymentTitle: "Pago",
  order: "Pedido",

  thankYou: "Gracias",
  confirming: "Confirmando tu pago…",
  paymentReceived: "Pago recibido. Esta es tu clave de licencia:",
  activating: "Cómo activarla",
  step1: "Abre bubbleTranslate.",
  step2: "Ve a la sección <b>Account</b>.",
  step3: "Pega la clave y elige <b>Activate</b>.",
  keepKey: "Funciona en hasta tres equipos. Guarda esta clave: esta página deja de mostrarla al cabo de una hora.",
  paymentFailed: "El pago no se completó.",
  nothingCharged: "No se ha cobrado nada.",
  tryAgain: "intentarlo de nuevo",
  youCan: "Puedes",
  takingLong: "Esto está tardando más de lo esperado.",
  ifCharged: "Si se te cobró, escribe a",
  withReference: "con la referencia",
  keyWillBeSent: "y te enviaremos la clave.",

  yourSubscription: "Tu suscripción",
  enterKeyToSee: "Introduce la clave que recibiste para ver su estado.",
  keyLabel: "Tu clave de licencia",
  lookUp: "Consultar",
  lostKey: (support: string) => `¿Perdiste la clave? Escribe a ${support} desde la dirección con la que compraste.`,
  cancelSubscription: "Cancelar suscripción",
  keepProUntil: (date: string) => `Conservas Pro hasta el ${date}.`,
  endOfPaidPeriod: "final del periodo pagado",
  fixedTerm: (date: string, buyUrl: string) =>
    `Esta es una licencia de plazo fijo: no hay ningún cobro recurrente que cancelar. Simplemente termina el ${date}, y puedes <a href="${buyUrl}">comprar otro periodo</a> cuando quieras.`,
  itsExpiryDate: "su fecha de vencimiento",
  statusLabel: "Estado",
  ends: "Termina el",
  devices: (used: number, limit: number) => `Dispositivos: ${used} de ${limit} en uso`,
  paidThrough: "Pagado a través de",
  questions: "Preguntas",
  status: { active: "activa", expired: "vencida", cancelled: "cancelada", refunded: "reembolsada" },
  cycle: { monthly: "mensual", yearly: "anual" },

  enterYourKey: "Introduce tu clave de licencia.",
  keyNotRecognised: "No se reconoce esa clave de licencia.",
  noRecurringCharge: "Esta licencia no tiene ningún cobro recurrente que cancelar.",
  cancelled: (date: string) => `Cancelada. Pro sigue funcionando hasta el ${date}.`,
  cancelNotConfigured: "La gestión de suscripciones no está configurada en este servidor.",
  manageBilling: "Gestionar la facturación",
  manageBillingNote:
    "Abre Paddle, donde puedes actualizar tu tarjeta, descargar facturas y cambiar de plan.",
  portalNotAvailable: "La gestión de facturación aún no está disponible para esta licencia.",
  paddleRefusedPortal: "Paddle no pudo abrir el portal de facturación. Contacta con soporte.",
  scheduledToCancel: (date: string) => `Se cancelará el ${date}. Pro funciona hasta esa fecha.`,
  paddleRefusedCancel: "Paddle no pudo cancelar esta suscripción. Contacta con soporte.",
  paddleUnreachable: "No se pudo contactar con Paddle. Inténtalo de nuevo en unos minutos.",

  invalidPlan: "Plan no válido.",
  invalidEmail: "Introduce una dirección de correo válida.",
  paytrNotConfigured: "PayTR no está configurado.",
  priceNotSet: "Este plan no tiene precio configurado.",
  paymentCouldNotStart: "No se pudo iniciar el pago.",
  paytrUnreachable: "No se pudo contactar con el proveedor de pagos. Inténtalo de nuevo.",
  paymentIncomplete: "El pago no se completó.",
};

const STRINGS: Record<Lang, Strings> = { en, tr, es };

export const t = (lang: Lang): Strings => STRINGS[lang];
