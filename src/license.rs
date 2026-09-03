//! The free daily allowance, and the signed entitlement that lifts it.
//!
//! There are no accounts here and no passwords. A purchase produces a licence
//! key, the key is exchanged once for a short-lived token signed by the
//! licence service, and from then on that token is checked locally against a
//! public key compiled into this binary. The service is not on the path of a
//! translation and never sees one: Pro only lifts a counter, so the text the
//! user selects goes straight to the providers exactly as it does today.
//!
//! The token carries its own expiry, which is also how a licence is revoked —
//! a refund simply stops the next refresh, and the entitlement lapses on its
//! own within [`TOKEN_TTL_HINT`]. There is no revocation list to poll and no
//! moment where a working app has to ask permission to keep working.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Translations a day without a subscription.
///
/// Deliberately also carried in the token as `lim`, so this is only the
/// fallback for an install that has never talked to the service. Changing the
/// number for everyone is a server-side edit, not a release.
pub const FREE_DAILY_TRANSLATIONS: u32 = 5;

/// Where the app sends someone who wants to subscribe, and where it sends a
/// subscriber who wants to cancel. Both take a `?src=` so the funnel can be
/// measured by the surface the click came from.
pub const BUY_URL: &str = "https://bubbletranslate.app/buy";
pub const MANAGE_URL: &str = "https://bubbletranslate.app/account";

/// The licence service. Overridable in debug builds so the client can be
/// developed against a local server.
const LICENSE_API: &str = "https://api.bubbletranslate.app";

/// Ed25519 public key of the licence service, hex-encoded.
///
/// Still the placeholder: until the service exists and its real key is pasted
/// in here, every token fails verification and the app stays on the free tier.
/// That is the correct failure direction, and [`verify`] says so
/// explicitly rather than reporting a generic bad signature.
const PUBLIC_KEY_HEX: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// Salt for the device fingerprint. Its only job is to keep the value from
/// being the machine id itself — see [`device_id`].
const DEVICE_SALT: &str = "bubbleTranslate/device/v1";

/// How long a freshly issued token lasts, as the client understands it. The
/// service is the authority; this is only used to describe the offline
/// allowance in the settings window.
pub const TOKEN_TTL_HINT: u64 = 30 * 86_400;

/// Refresh once the token has this long left. A month of validity refreshed
/// with ten days to spare means three weeks of failed refreshes before a
/// subscriber notices anything, which is the point.
const REFRESH_WINDOW: u64 = 10 * 86_400;

/// Tolerance for a clock that is ahead of the service's. Rejecting a token
/// whose `iat` is in the future catches a hand-edited cache, but a laptop
/// running a few minutes fast must not lock itself out.
const CLOCK_SKEW: u64 = 24 * 3600;

/// The licence state and the meter, bundled because everything that reads one
/// reads the other: the settings window shows both, and the engine consults
/// both before every selection.
#[derive(Clone)]
pub struct Licensing {
    pub license: Arc<Mutex<License>>,
    pub quota: Arc<Mutex<crate::quota::Quota>>,
}

// -- what the app actually asks -------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plan {
    Free,
    Pro,
}

impl Plan {
    pub fn label(self) -> &'static str {
        match self {
            Plan::Free => "Free",
            Plan::Pro => "Pro",
        }
    }
}

/// What this install is allowed to do. The one thing the rest of the app reads.
#[derive(Debug, Clone)]
pub struct Entitlement {
    pub plan: Plan,
    /// Translations a day, or `None` for unlimited.
    pub daily_limit: Option<u32>,
    /// Unix seconds at which the token stops being valid. 0 on the free tier.
    pub exp: u64,
}

impl Entitlement {
    pub fn free() -> Self {
        Self {
            plan: Plan::Free,
            daily_limit: Some(FREE_DAILY_TRANSLATIONS),
            exp: 0,
        }
    }

    pub fn is_pro(&self) -> bool {
        self.plan == Plan::Pro
    }
}

/// How the licence got into its current state, phrased for the settings window.
#[derive(Debug, Clone)]
pub enum Status {
    /// No key has ever been entered.
    None,
    /// A valid token, with the renewal date the service last reported.
    Active { renews: Option<String> },
    /// A token that was valid and is not any more.
    Lapsed,
    /// A key was entered and the service or the network refused it. The string
    /// is shown to the user, so it has to read as an explanation.
    Problem(String),
}

/// The shared licence state, held behind a mutex alongside the config.
pub struct License {
    pub entitlement: Entitlement,
    pub status: Status,
    /// Set while an activation or refresh is in flight, so the settings window
    /// can disable the button and show a spinner.
    pub busy: bool,
    token: Option<String>,
}

impl License {
    fn inactive(status: Status) -> Self {
        Self {
            entitlement: Entitlement::free(),
            status,
            busy: false,
            token: None,
        }
    }

    /// Reads the cached token and verifies it. Never touches the network, so
    /// this is safe to call before the first frame.
    pub fn load() -> Self {
        let Some(cached) = read_cache() else {
            return Self::inactive(Status::None);
        };
        match verify(&cached.token) {
            Ok(claims) => {
                crate::trace!("licence   {} valid until {}", claims.plan, claims.exp);
                Self {
                    entitlement: claims.entitlement(),
                    status: Status::Active {
                        renews: cached.renews,
                    },
                    busy: false,
                    token: Some(cached.token),
                }
            }
            Err(err) => {
                crate::trace!("licence   cached token rejected: {err}");
                // An expired token is an ordinary lapse; anything else means
                // the file was edited, the key rotated, or the token belongs
                // to another machine. Both land on the free tier, but only one
                // of them is worth alarming the user about.
                Self::inactive(match err {
                    VerifyError::Expired => Status::Lapsed,
                    other => Status::Problem(other.to_string()),
                })
            }
        }
    }

    /// Whether a background refresh is worth making right now.
    pub fn wants_refresh(&self) -> bool {
        self.token.is_some() && self.entitlement.exp.saturating_sub(now()) < REFRESH_WINDOW
    }

    pub fn token(&self) -> Option<&str> {
        self.token.as_deref()
    }

    /// Applies a grant from the service and caches it.
    pub fn accept(&mut self, grant: Grant) -> Result<(), String> {
        let claims = verify(&grant.token).map_err(|e| e.to_string())?;
        self.entitlement = claims.entitlement();
        self.status = Status::Active {
            renews: grant.renews.clone(),
        };
        self.token = Some(grant.token.clone());
        write_cache(&Cached {
            token: grant.token,
            renews: grant.renews,
        });
        Ok(())
    }

    /// Drops the licence from this machine. The service is told separately, so
    /// this succeeding does not depend on the network being up.
    pub fn forget(&mut self) {
        self.entitlement = Entitlement::free();
        self.status = Status::None;
        self.token = None;
        let _ = std::fs::remove_file(path());
    }
}

// -- the token ------------------------------------------------------------

/// The signed payload. Short field names because the whole token travels in a
/// text box the user may have to paste by hand if support ever asks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    /// Licence id — not the key, which never leaves the service in this form.
    pub lic: String,
    pub plan: String,
    /// Daily translation limit; absent or null means unlimited.
    #[serde(default)]
    pub lim: Option<u32>,
    /// The device fingerprint this token was issued to.
    pub dev: String,
    pub iat: u64,
    pub exp: u64,
}

impl Claims {
    fn entitlement(&self) -> Entitlement {
        let plan = if self.plan == "pro" {
            Plan::Pro
        } else {
            Plan::Free
        };
        Entitlement {
            plan,
            // A pro token with no `lim` is unlimited; a free one falls back to
            // the compiled-in allowance rather than to no limit at all, so a
            // malformed payload can never be a free upgrade.
            daily_limit: match (plan, self.lim) {
                (_, Some(n)) => Some(n),
                (Plan::Pro, None) => None,
                (Plan::Free, None) => Some(FREE_DAILY_TRANSLATIONS),
            },
            exp: self.exp,
        }
    }
}

#[derive(Debug)]
pub enum VerifyError {
    /// No real service key has been compiled in yet.
    NoServiceKey,
    Malformed(String),
    BadSignature,
    Expired,
    /// Issued to a different machine.
    WrongDevice,
    /// Issued in the future — the file has been edited, or a clock is wrong.
    NotYetValid,
}

impl std::fmt::Display for VerifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VerifyError::NoServiceKey => {
                write!(f, "this build has no licence service key compiled in")
            }
            VerifyError::Malformed(why) => write!(f, "unreadable licence token ({why})"),
            VerifyError::BadSignature => write!(f, "licence token failed its signature check"),
            VerifyError::Expired => write!(f, "licence token has expired"),
            VerifyError::WrongDevice => write!(f, "licence token belongs to another device"),
            VerifyError::NotYetValid => write!(f, "licence token is dated in the future"),
        }
    }
}

/// Checks a token's signature and its claims against this machine and clock.
pub fn verify(token: &str) -> Result<Claims, VerifyError> {
    let key = service_key()?;

    let (payload_b64, sig_b64) = token
        .split_once('.')
        .ok_or_else(|| VerifyError::Malformed("no signature".into()))?;

    let payload = B64
        .decode(payload_b64)
        .map_err(|e| VerifyError::Malformed(e.to_string()))?;
    let sig_bytes: [u8; 64] = B64
        .decode(sig_b64)
        .map_err(|e| VerifyError::Malformed(e.to_string()))?
        .try_into()
        .map_err(|_| VerifyError::Malformed("signature is the wrong length".into()))?;

    // Verified before the payload is parsed: nothing inside an unsigned blob
    // gets to influence anything, including which parser runs on it.
    key.verify_strict(payload_b64.as_bytes(), &Signature::from_bytes(&sig_bytes))
        .map_err(|_| VerifyError::BadSignature)?;

    let claims: Claims = serde_json::from_slice(&payload)
        .map_err(|e| VerifyError::Malformed(format!("payload is not valid JSON ({e})")))?;

    let now = now();
    if claims.exp <= now {
        return Err(VerifyError::Expired);
    }
    if claims.iat > now + CLOCK_SKEW {
        return Err(VerifyError::NotYetValid);
    }
    if claims.dev != device_id() {
        return Err(VerifyError::WrongDevice);
    }

    Ok(claims)
}

fn service_key() -> Result<VerifyingKey, VerifyError> {
    // A debug build may point at a locally generated keypair, which is what
    // makes the whole client testable before the service exists. Release
    // builds ignore the variable entirely — otherwise it would be a licence
    // bypass anyone could set.
    #[cfg(debug_assertions)]
    let hex = std::env::var("BUBBLETRANSLATE_LICENSE_PUBKEY")
        .unwrap_or_else(|_| PUBLIC_KEY_HEX.to_string());
    #[cfg(not(debug_assertions))]
    let hex = PUBLIC_KEY_HEX.to_string();

    if hex.trim_matches('0').is_empty() {
        return Err(VerifyError::NoServiceKey);
    }
    let bytes: [u8; 32] = decode_hex(&hex)
        .ok_or_else(|| VerifyError::Malformed("service key is not 32 hex bytes".into()))?;
    VerifyingKey::from_bytes(&bytes).map_err(|_| VerifyError::BadSignature)
}

// -- talking to the service -----------------------------------------------

/// What the service hands back on a successful activation or refresh.
#[derive(Debug, Clone)]
pub struct Grant {
    pub token: String,
    /// Next renewal, as the service formatted it. Displayed, never enforced —
    /// the token's own `exp` is the only thing with authority here.
    pub renews: Option<String>,
}

/// Exchanges a licence key for a token, binding this device to the licence.
pub fn activate(agent: &ureq::Agent, key: &str) -> Result<Grant, String> {
    let key = key.trim();
    if key.is_empty() {
        return Err("Enter your licence key first.".into());
    }
    post(
        agent,
        "/v1/activate",
        serde_json::json!({
            "key": key,
            "device": device_id(),
            "os": std::env::consts::OS,
            "app": env!("CARGO_PKG_VERSION"),
        }),
    )
}

/// Re-signs an existing token before it expires.
pub fn refresh(agent: &ureq::Agent, token: &str) -> Result<Grant, String> {
    post(
        agent,
        "/v1/refresh",
        serde_json::json!({ "token": token, "device": device_id() }),
    )
}

/// Releases this device so its slot can be used elsewhere. Best effort: the
/// local licence is dropped whether or not the service agrees.
pub fn deactivate(agent: &ureq::Agent, token: &str) {
    let _ = post(
        agent,
        "/v1/deactivate",
        serde_json::json!({ "token": token, "device": device_id() }),
    );
}

fn post(agent: &ureq::Agent, route: &str, body: serde_json::Value) -> Result<Grant, String> {
    let base = api_base();
    let response = agent
        .post(format!("{base}{route}"))
        .header("Content-Type", "application/json")
        .send(body.to_string().as_str())
        .map_err(|e| format!("Could not reach the licence service ({e})."))?;

    let status = response.status().as_u16();
    let text = response
        .into_body()
        .read_to_string()
        .map_err(|e| format!("Could not read the licence service's reply ({e})."))?;

    if status != 200 {
        // The service explains refusals in the body; a bare status code is
        // the fallback, not the message.
        let detail = serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .and_then(|v| {
                v.get("error")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
            });
        return Err(match detail {
            Some(msg) => msg,
            None => format!("The licence service refused this (HTTP {status})."),
        });
    }

    let json: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("Unreadable reply ({e})."))?;
    let token = json
        .get("token")
        .and_then(serde_json::Value::as_str)
        .ok_or("The licence service sent no token.")?
        .to_string();

    Ok(Grant {
        token,
        renews: json
            .get("renews")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
    })
}

fn api_base() -> String {
    #[cfg(debug_assertions)]
    {
        std::env::var("BUBBLETRANSLATE_LICENSE_API").unwrap_or_else(|_| LICENSE_API.to_string())
    }
    #[cfg(not(debug_assertions))]
    {
        LICENSE_API.to_string()
    }
}

// -- device identity -------------------------------------------------------

/// A stable, opaque name for this installation.
///
/// Derived rather than raw: the machine id is hashed with a salt and
/// truncated, so what reaches the service identifies an install without being
/// a hardware identifier anyone now holds. Falling back to the hostname, and
/// then to a constant, keeps a machine with no readable id usable — a licence
/// that refuses to activate is worse than a device slot that is shared.
pub fn device_id() -> String {
    let raw = machine_id().unwrap_or_else(|| "unknown-machine".to_string());
    let mut hasher = Sha256::new();
    hasher.update(DEVICE_SALT.as_bytes());
    hasher.update(raw.as_bytes());
    hasher
        .finalize()
        .iter()
        .take(16)
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[cfg(target_os = "linux")]
fn machine_id() -> Option<String> {
    for path in ["/etc/machine-id", "/var/lib/dbus/machine-id"] {
        if let Ok(id) = std::fs::read_to_string(path) {
            let id = id.trim().to_string();
            if !id.is_empty() {
                return Some(id);
            }
        }
    }
    hostname()
}

#[cfg(target_os = "macos")]
fn machine_id() -> Option<String> {
    // IOPlatformUUID, read through ioreg rather than linking IOKit for one
    // string. The line looks like:
    //     "IOPlatformUUID" = "5D5C6E3A-..."
    let out = std::process::Command::new("/usr/sbin/ioreg")
        .args(["-rd1", "-c", "IOPlatformExpertDevice"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let id = text
        .lines()
        .find(|line| line.contains("IOPlatformUUID"))
        .and_then(|line| line.split('=').nth(1))
        .map(|value| value.trim().trim_matches('"').to_string())
        .filter(|value| !value.is_empty());
    id.or_else(hostname)
}

fn hostname() -> Option<String> {
    std::process::Command::new("hostname")
        .output()
        .ok()
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .filter(|name| !name.is_empty())
}

// -- the cache file --------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
struct Cached {
    token: String,
    #[serde(default)]
    renews: Option<String>,
}

/// The token lives in the data directory, not next to `config.toml`.
///
/// The config file is one users are invited to edit by hand — the README says
/// so. Machine state that happens to be security-relevant should not sit in
/// the same file as their choice of target language.
pub fn path() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("bubbleTranslate")
        .join("license.json")
}

fn read_cache() -> Option<Cached> {
    let raw = std::fs::read_to_string(path()).ok()?;
    serde_json::from_str(&raw).ok()
}

fn write_cache(cached: &Cached) {
    let path = path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    match serde_json::to_string_pretty(cached) {
        Ok(json) => {
            if let Err(err) = std::fs::write(&path, json) {
                // Not fatal: the entitlement is live in memory for this
                // session, and the next launch simply asks again.
                eprintln!("bubbleTranslate: could not save {}: {err}", path.display());
            }
        }
        Err(err) => eprintln!("bubbleTranslate: could not encode the licence ({err})"),
    }
}

// -- odds and ends ---------------------------------------------------------

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn decode_hex(hex: &str) -> Option<[u8; 32]> {
    let hex = hex.trim();
    if hex.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (index, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(hex.get(index * 2..index * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}

/// Minting licences locally, so the client can be built and lived with before
/// the licence service exists.
///
/// Compiled only into debug builds, alongside the environment override in
/// [`service_key`] that makes the token it produces verifiable. A release
/// build contains neither, so neither is a way past the real key.
#[cfg(debug_assertions)]
pub mod dev {
    use ed25519_dalek::{Signer, SigningKey};

    use super::*;

    /// The dev signing key, kept between runs so the public key printed by one
    /// invocation still verifies the token minted by the next.
    fn seed_path() -> PathBuf {
        path().with_file_name("dev-signing-key")
    }

    fn signing_key() -> SigningKey {
        if let Ok(hex) = std::fs::read_to_string(seed_path())
            && let Some(seed) = decode_hex(hex.trim())
        {
            return SigningKey::from_bytes(&seed);
        }
        // /dev/urandom rather than a rand crate: this is a development tool on
        // two Unixes, and it is not worth a dependency the app never ships.
        let mut seed = [0u8; 32];
        let random = std::fs::read("/dev/urandom").expect("no /dev/urandom");
        seed.copy_from_slice(&random[..32]);

        let path = seed_path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let hex: String = seed.iter().map(|b| format!("{b:02x}")).collect();
        let _ = std::fs::write(&path, hex);
        SigningKey::from_bytes(&seed)
    }

    /// Signs a token for *this* device and installs it, the way a successful
    /// activation would. Returns the public key to run the app with.
    pub fn mint(plan: &str, days: u64) -> String {
        let key = signing_key();
        let issued = now();
        let claims = Claims {
            lic: "lc_dev".into(),
            plan: plan.to_string(),
            lim: None,
            dev: device_id(),
            iat: issued,
            exp: issued + days * 86_400,
        };

        let payload = B64.encode(serde_json::to_vec(&claims).expect("claims are serialisable"));
        let signature = key.sign(payload.as_bytes());
        let token = format!("{payload}.{}", B64.encode(signature.to_bytes()));

        write_cache(&Cached {
            token,
            renews: Some("(development licence)".into()),
        });

        key.verifying_key()
            .to_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The placeholder key must never verify anything. If this ever passes by
    /// accident, every build ships an entitlement anyone can mint.
    #[test]
    fn the_placeholder_key_is_not_a_key() {
        assert!(matches!(service_key(), Err(VerifyError::NoServiceKey)));
    }

    #[test]
    fn a_malformed_token_is_rejected_not_panicked_on() {
        for token in ["", "no-dot", "a.b", "....", "!!!.???"] {
            assert!(verify(token).is_err(), "{token:?} was accepted");
        }
    }

    /// The device id has to be stable across calls, or every launch would burn
    /// a device slot.
    #[test]
    fn the_device_id_is_stable_and_opaque() {
        let first = device_id();
        assert_eq!(first, device_id());
        assert_eq!(first.len(), 32);
        assert!(first.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(!first.contains(&machine_id().unwrap_or_default()));
    }

    /// A payload claiming Pro with no limit is unlimited; a free one with no
    /// limit falls back to the allowance rather than to no limit at all.
    #[test]
    fn a_missing_limit_never_becomes_a_free_upgrade() {
        let claims = |plan: &str| Claims {
            lic: "lc_1".into(),
            plan: plan.into(),
            lim: None,
            dev: "d".into(),
            iat: 0,
            exp: 0,
        };
        assert_eq!(claims("pro").entitlement().daily_limit, None);
        assert_eq!(
            claims("free").entitlement().daily_limit,
            Some(FREE_DAILY_TRANSLATIONS),
        );
        assert_eq!(
            claims("nonsense").entitlement().daily_limit,
            Some(FREE_DAILY_TRANSLATIONS),
        );
    }
}
