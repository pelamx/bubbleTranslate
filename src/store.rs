//! Buying Pro inside the Mac App Store build.
//!
//! The App Store does not allow a link to our own checkout, so there Pro is
//! an App Store subscription, bought through StoreKit's own sheet. Every
//! other build buys in the browser and never reaches this module; see
//! `license::open_buy`.
//!
//! Not written yet: StoreKit 2 is a Swift API, and calling it needs a small
//! Swift bridge built and tested on a Mac. Until then the purchase is only
//! traced, which is why the App Store build is not ready to submit.

/// Starts buying Pro. `src` is the surface the click came from.
pub fn purchase(src: &str) {
    crate::trace!("store: purchase from {src} -- StoreKit is not wired up yet");
}
