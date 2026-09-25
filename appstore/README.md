# The Mac App Store build

Work in progress on the `appstore` branch. The direct download (DMG, Developer
ID, Paddle) is untouched: everything here is behind the `appstore` Cargo
feature, and `license::APP_STORE` is the one switch the code asks.

## What the App Store requires, and where it is handled

| Rule | Done | Where |
|---|---|---|
| Runs in the App Sandbox | yes | `bubbleTranslate.entitlements` (sandbox + outgoing network) |
| No update notice of its own | yes | `update::spawn` returns at once |
| No link to our own checkout | yes | every "Upgrade / Get Pro" goes through `license::open_buy` |
| Pro bought through the App Store | **no** | `src/store.rs` is a stub; needs StoreKit 2 via a Swift bridge |
| No helper processes the sandbox refuses | partly | machine id read from IOKit, not `ioreg`; OCR still runs `screencapture` |
| A sandboxed app cannot type into others | yes | synthetic Cmd+C off; Accessibility and copy-on-select remain |
| Packaged and signed for the store | yes, untested | `APPSTORE=1 ./bundle.sh`, `./release-appstore.sh` |

## Still to do, on a Mac

1. **Try the sandbox first.** `./release-appstore.sh` (without `UPLOAD`) and
   open the app. Select text in Safari, Notes, Mail, Terminal, Chrome, VS Code
   and a PDF, and note which ones produce a bubble. If too few do, stop here:
   the store build would be worse than the download.
2. **Screen reading (⌘⇧E).** `platform/macos/ocr.rs` starts `screencapture`,
   which the sandbox may refuse. If it does, capture with ScreenCaptureKit
   (`SCScreenshotManager`) instead, behind the same feature.
3. **StoreKit.** Two auto-renewing subscriptions in App Store Connect,
   monthly and yearly, at the prices in `license::PRICE_MONTHLY` /
   `PRICE_YEARLY`. A small Swift bridge for `Product.purchase()` and
   `Transaction.currentEntitlements`, called from `store::purchase`. "Manage
   subscription" should open the App Store's subscriptions page for an Apple
   subscriber.
4. **The licence service.** An endpoint for App Store Server Notifications
   V2: verify the JWS, then write the licence with `provider = 'apple'`, reusing
   `isLive` / `endLicence` so a cancellation keeps the paid term and a refund
   ends it. Idempotent on the original transaction id.
5. **App Store Connect.** Bundle id `com.pelamx.bubbleTranslate`, screenshots,
   description in three languages, privacy policy URL, the App Privacy form
   (the daily anonymous ping and the text sent for translation: "not linked
   to you"), and a review note explaining why Accessibility, Input Monitoring
   and Screen Recording are asked for, with the demo video.
6. **Small Business Program**, so Apple takes 15% rather than 30%.
7. On release: a `(macOS)` entry in `CHANGELOG.md`, and the App Store badge on
   the website beside the DMG.
