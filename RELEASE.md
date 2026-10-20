# Preparing 0.4.21

The client version is shared by Linux, macOS and Windows through `Cargo.toml`.
The bundle version, Windows executable resources, and Store package version
are derived from it. The Microsoft Store version is `0.4.21.0`.
`CHANGELOG.md` holds the release notes under **Unreleased** until publication.
The analytics and remote configuration service is already deployed.

## Prepare packages without publishing

Run these commands on each platform:

| Platform | Command | Output |
|---|---|---|
| Linux x86-64, glibc 2.28+ | `./release-linux.sh --prepare-only` | `dist/0.4.21/linux/bubbleTranslate-linux-x86_64` |
| macOS Apple Silicon + Intel | `./release.sh --prepare-only` | `dist/0.4.21/macos/bubbleTranslate.dmg` |
| Windows x64 | `./release.ps1 -PrepareOnly` | `dist/0.4.21/windows/bubbleTranslate-windows-x64.zip` and `.exe` |
| Microsoft Store | `./package.ps1` | `bubbleTranslate.msix`, with version `0.4.21.0` |

Preparation mode writes local checksums and copies the release notes alongside
each download. It does not contact GitHub, upload packages, change update
manifests, or prune old releases. The original release commands still publish;
use the preparation flag while staging this version.

Linux requires cargo-zigbuild and Zig. macOS packaging requires Xcode tools
and both Rust targets (`aarch64-apple-darwin`, `x86_64-apple-darwin`). Without
the Intel target the existing bundle script falls back to Apple Silicon only;
install both and verify with `lipo -verify_arch arm64 x86_64` before release.
Windows packaging uses MSVC and PowerShell. The Store package also requires
the existing `windows/store-identity.json` from Partner Center and the Windows
SDK; preparing the package does not submit it.

The **Prepare release candidates** GitHub Actions workflow builds all three
native download formats and uploads private workflow artifacts for 14 days.
Run it from Actions after these source changes reach the repository. It has
read-only repository permissions and cannot publish a release. Its macOS
artifact is ad-hoc signed, without Apple notarization; the public DMG must be
rebuilt on the signing machine using the existing Developer ID and notary
profile. Windows signing follows the existing `SIGN_THUMBPRINT` setting.
Workflow artifacts are candidates, not evidence of a successful desktop run.

## Verify the candidate

Start the actual package on each desktop, select text in another application,
and verify that the bubble appears with a translation. Also check a region
and a whole-screen OCR translation, manual input, and relaunch. Check that
the version reports `0.4.21` and the correct channel; the Store build must
report `channel store` and leave updates to the Store.

The existing `/admin` panel should show the new version and feature/error
counts after five minutes, or after a normal exit. Remote overrides should
apply without changing saved settings. With diagnostics disabled, the new
feature/error counters must stop collecting and sending. Offline startup
must still translate using local settings and available providers.

Before publishing downloads, rename **Unreleased** to **0.4.21** with the
actual publication date, then use the normal release commands on the machines
where the candidate was verified. Keep the existing signing/notarization
steps. The release scripts update each platform's published manifest only
when its download is released. The root `latest.json` is maintained by the
existing mirror workflow and must not be bumped during preparation.
