# bubbleTranslate

Select text anywhere — a PDF, a terminal, a browser, an editor — and a small
bubble appears at the cursor with the translation. Runs on macOS and Linux.

The translator, the provider chain and the interface are the same code on both.
What differs is how a desktop lets an application find out what is selected,
which is a surprisingly large difference: see
[How it reads the selection](#how-it-reads-the-selection).

## Install

| | Download | What you get |
|---|---|---|
| **macOS** | [`bubbleTranslate.dmg`](https://github.com/pelamx/bubbleTranslate/raw/main/bubbleTranslate.dmg) (7 MB) | An app bundle to drag into Applications |
| **Linux** | [`bubbleTranslate-linux-x86_64`](https://github.com/pelamx/bubbleTranslate/raw/main/bubbleTranslate-linux-x86_64) (15 MB) | One executable to `chmod +x` and run |

The two never collide, so this repo carries both. Neither download needs a Rust
toolchain; building from source is covered under each and is the better route
on Linux if your distribution is not a recent one — see the note on glibc.

### macOS — from the DMG

Open the DMG and drag **bubbleTranslate.app** onto the Applications folder.

The app is ad-hoc signed rather than notarized, so macOS blocks the first
launch with *"Apple could not verify bubbleTranslate is free of malware."*
That is Gatekeeper reacting to the missing Apple Developer signature, not to
anything the app does. To get past it:

1. Double-click the app and dismiss the warning.
2. Open **System Settings › Privacy & Security**, scroll to Security, and click
   **Open Anyway** next to the message about bubbleTranslate.
3. Confirm once more when the app launches.

On macOS 15 and later, Control-clicking the app and choosing *Open* no longer
bypasses this — Open Anyway is the route. If you would rather not go through
Settings, stripping the quarantine flag has the same effect:

```sh
xattr -dr com.apple.quarantine /Applications/bubbleTranslate.app
```

This step is only needed once, and only for a build downloaded from the
internet. Building from source skips it entirely.

Then grant Accessibility — the one permission the app needs, and the subject of
[the next section](#granting-accessibility-macos). Without it macOS will not
tell the app what is selected, and no bubble ever appears.

### macOS — from source

```sh
./bundle.sh
open bubbleTranslate.app
```

`bundle.sh` builds and assembles `bubbleTranslate.app` in place. Run
`./setup-signing.sh` first if you expect to rebuild often; it is what keeps the
Accessibility grant from going stale, explained below.

### Linux — from the binary

One statically-named, dynamically-linked x86-64 executable. Nothing is signed
or quarantined on Linux, so there is no Gatekeeper equivalent to get past and
no permission to grant — the desktop publishes the selection itself. It just
needs the executable bit:

```sh
chmod +x bubbleTranslate-linux-x86_64
./bubbleTranslate-linux-x86_64 --check     # confirms the backends answer
./bubbleTranslate-linux-x86_64
```

`--check` runs one translation through each provider and prints the result
without opening a window, which separates "the app is broken" from "the network
is" on a first run.

**It needs a recent distribution.** The binary is built on Arch and links
against glibc 2.43 or newer — `atan2f@GLIBC_2.43` and friends, pulled in by the
maths in the bubble's layout. On anything older the loader refuses it outright:

```
version `GLIBC_2.43' not found (required by ./bubbleTranslate-linux-x86_64)
```

Ubuntu 24.04 (glibc 2.39) and Debian 13 (2.41) are both below that line. If you
see that error, build from source instead — it takes a couple of minutes and
produces a binary matched to your own system.

Everything else it needs is already on any desktop that can run a GUI, and is
loaded at runtime rather than linked: `libGL`, `libxkbcommon`, and
`libwayland-client` on a Wayland session.

To install it properly — on your `PATH`, in the application menu, with an icon
— take the launcher and icon from this repo alongside it:

```sh
install -Dm755 bubbleTranslate-linux-x86_64 ~/.local/bin/bubbleTranslate
install -Dm644 linux/bubbleTranslate.desktop ~/.local/share/applications/bubbleTranslate.desktop
install -Dm644 linux/bubbleTranslate.svg ~/.local/share/icons/hicolor/scalable/apps/bubbleTranslate.svg
```

No root, nothing outside the XDG user directories. Uninstalling is deleting
those three files. To update later, download the executable again and repeat
the first line.

### Linux — from source

The recommended route on any distribution that the prebuilt binary refuses, and
the one that keeps working as your system moves:

```sh
./linux/install.sh
```

Builds with `cargo`, then installs exactly the three files above into
`~/.local`. Run it with `bubbleTranslate`, or find it in the application menu.
It also checks your session first and says which selection backend you will
get, which is worth reading — see [How it reads the
selection](#how-it-reads-the-selection).

### What Linux needs

- **XWayland**, on a Wayland session. The interface is drawn as an X11 client
  on every desktop, because a Wayland window is not allowed to choose its own
  position and the bubble has to appear at the cursor. Reading the selection
  does *not* go through X11 — see below.
- **A tray**, if you want to close the window without stopping the translator.
  Any StatusNotifierItem host will do, which is what waybar, quickshell, KDE
  and XFCE all expose a tray through; GNOME needs the AppIndicator extension.
  Without one the app still works, but the main window *is* the app and closing
  it quits — see [Running with no window](#running-with-no-window).
- A font with coverage past Latin, if you translate into Chinese, Japanese,
  Korean, Arabic or Cyrillic. Any Noto CJK package will do; `fc-match` is asked
  where it went.

### Granting Accessibility (macOS)

Either route, the first launch asks for **Accessibility** permission. Grant it
in System Settings › Privacy & Security › Accessibility, then **quit and
relaunch** — the event tap is installed at startup, so it needs a restart to
take effect.

macOS ties this permission to the app's exact signature. An ad-hoc signature —
what `codesign --sign -` produces — has no stable identity, so its designated
requirement is the binary's own hash:

```
designated => cdhash H"e39883dc..."
```

Every rebuild changes that hash, macOS sees an unrelated program, and the grant
stops applying **while the switch still reads "on"** — a confusing failure,
because nothing looks wrong.

Run `./setup-signing.sh` once to stop this. It creates a self-signed
certificate and `bundle.sh` signs with it from then on, which pins the
requirement to the identity instead:

```
designated => identifier "com.pelamx.bubbleTranslate"
              and certificate root = H"c0a427ea..."
```

Later builds keep satisfying that, so the grant survives rebuilds. Grant
Accessibility once after the switch and you are done. Without the certificate
the app still builds and runs — it just costs a re-grant every time.

The certificate is self-signed, so **Gatekeeper is unaffected**: a downloaded
DMG still shows the "could not verify" warning. Removing that needs a paid
Developer ID, covered under [Building an installer](#building-an-installer).
The private key lives in `~/.config/bubbletranslate` (mode 600, outside the
repo). Back it up — losing it means a new certificate, and one more re-grant.

If a grant has already gone stale, clearing the entry is quicker than hunting
the toggle:

```sh
tccutil reset Accessibility com.pelamx.bubbleTranslate
```

The app shows a Dock icon only while the main window is open. That is not
cosmetic: macOS will not bring a background-only app to the front, so the app
becomes a regular one for as long as it has a window, and drops back to
background when you close it — which is what keeps the bubble from stealing
focus from whatever you are reading.

## Running with no window

A translator spends nearly all its time waiting, so the window is somewhere to
visit rather than somewhere to live. Both platforms put a globe in the desktop's
own strip of indicators — the **menu bar** (🌐) on macOS, the **tray** on Linux
— and that icon is what the app is when its window is closed.

- **Closing the window does not quit.** It puts the interface away; the
  translator carries on watching selections. Close it however your desktop
  closes windows — the red button, `Super+W`, whatever you use.
- **Clicking the icon brings the window back.** On macOS, launching the app
  again does the same.
- **Quit lives in the icon's menu**, and nowhere else. That is deliberate: a
  window you can dismiss by reflex should not also be the thing that stops the
  app.

To skip the window entirely at startup, tick **Start without the window** in
the main window's Behaviour section, or pass the flag:

```sh
bubbleTranslate --background
```

The setting is for making it the habit; the flag is for an autostart entry that
should not depend on the config agreeing. Add `--background` to `Exec=` in
`bubbleTranslate.desktop` and the app comes up as nothing but its tray icon.

There is one safeguard. On Linux the tray is a negotiation with the desktop and
it can fail — a session with no StatusNotifierItem host has nowhere to put the
icon. Where that happens the app says so on stderr and falls back to the older
arrangement: the main window *is* the app, and closing it quits. An app you have
started invisibly and cannot reach is worse than an unwanted window, so
`--background` opens one rather than leaving you with neither.

## The main window

- **Translate** — a scratch box for text you paste in, using the same chain
- **Languages** — target and source language
- **Providers** — reorder the chain with ↑↓, set the DeepL key and MyMemory
  email, and **Test providers** to see which backends answer right now
- **Behaviour** — bubble text size, auto-hide delay, settle delay, length cap
- **Recent** — what the bubble has translated this session, with copy buttons

Every change saves immediately to the config file.

## Building an installer

```sh
./release.sh          # -> bubbleTranslate.dmg
```

Drag-to-Applications layout. Without an Apple developer account the app is
ad-hoc signed, so the DMG installs fine but Gatekeeper blocks the first launch
and the user has to clear it once — see [macOS — from the DMG](#macos--from-the-dmg).

With a `Developer ID Application` certificate ($99/year Apple Developer
Program) the same script produces a release that opens with no warning:

```sh
xcrun notarytool store-credentials bubbleTranslate-notary \
  --apple-id you@example.com --team-id TEAMID --password <app-specific-password>

SIGN_IDENTITY="Developer ID Application: Your Name (TEAMID)" \
NOTARY_PROFILE=bubbleTranslate-notary \
./release.sh
```

**This app cannot ship on the Mac App Store.** Store apps must run in the App
Sandbox, which forbids all three of its capture mechanisms — Accessibility
reads of other apps, system-wide event taps, and synthesized keystrokes. A
sandboxed build would launch and capture nothing. Direct distribution is the
route for a tool that reads other applications.

## How it reads the selection

This is the part that is genuinely different per platform, and the differences
are worth knowing about because they decide which applications work.

### Linux

Nothing is captured and nothing is synthesized: selecting text already
publishes it, and the desktop is asked for it.

| Session | How | Works? |
|---|---|---|
| X11, any desktop | The primary selection, watched with XFixes | Yes |
| Wayland — Hyprland, sway, KDE, COSMIC, river | `wlr-data-control` | Yes |
| Wayland — GNOME | Nothing available | No |

Wayland ties clipboard access to keyboard focus on purpose, and a translator
that watches selections in *other* windows is never the focused client. The
`wlr-data-control` protocol exists for exactly this case — it is what clipboard
managers use — and every compositor above implements it except GNOME's. On
GNOME's Wayland session no application can read another's selection at all;
bubbleTranslate says so in its window rather than appearing to work, and the
translate box and the command line still do.

Note that the session type decides this, not the toolkit: running as an X11
client under XWayland does *not* work around it, because the compositor guards
the bridged selection behind the same focus rule.

**Applications that publish nothing.** GTK, Qt, Firefox, Chromium, Electron,
terminals and PDF viewers all publish a primary selection. A few programs draw
their own text and publish none — bubbleTranslate's own window among them. For
those, turn on **Also translate on copy (Ctrl+C)** under Behaviour: copying is
the one gesture that always reaches the desktop.

**Workspaces.** The bubble asks to appear on every workspace with
`_NET_WM_STATE_STICKY`, which X11 window managers honour. wlroots compositors
do not implement it for X11 clients: there the equivalent is a *pin*, a state
the compositor holds rather than a property a window can set on itself. So on
Hyprland the app asks over `hyprctl` instead, once per time the bubble is shown
— the pin does not survive the window being hidden, and a bubble that only
appears on the workspace it was first shown on is the kind of failure that
looks like the translator has simply stopped working.

Nothing to configure; it happens on its own. If it ever cannot — the trace says
so with `BUBBLETRANSLATE_DEBUG=1` — the same thing in config form is:

```
windowrule = pin, class:^(bubbleTranslate)$, floating:1
```

**Scaling.** On a scaled display the compositor and the X server describe the
same screen differently — 2× against 2.33× on the laptop this was written on —
which would make the text larger than every other window and put the bubble
somewhere other than the cursor. The two are measured and reconciled at
startup. **Interface scale** in the main window is the dial on top of that, for
desktops that simply run denser than macOS.

### macOS

Two strategies, in order:

1. **Accessibility API** — asks the focused element for `AXSelectedText`.
   Instant, and it never touches your clipboard. Works in native apps, Safari,
   and most text fields.
2. **Synthetic Cmd+C** — posts a command-C and watches the pasteboard's change
   count. Terminal.app, PDF viewers and Electron apps expose nothing over AX
   but copy fine, so this is what makes "anywhere" literal. Your previous
   clipboard **text** is restored afterwards; a copied image or file reference
   is not.

Set `clipboard_fallback = false` in the config to disable strategy 2.

Only gestures that actually finish a selection trigger a capture: a drag longer
than a few points, a double/triple click, shift+arrow navigation, or Cmd+A. A
plain click never does — otherwise strategy 2 would fire a copy on every click
in the OS.

## Translation backends

Three, tried in order until one answers:

| # | Provider | Key needed | Notes |
|---|----------|-----------|-------|
| 1 | Google   | no  | Unofficial `translate_a` endpoint. Fast and free; rate-limits per IP and can answer a burst with an HTML captcha page. Falls back to a mirror host and one retry before giving up. |
| 2 | MyMemory | no  | Detects the source itself (`Autodetect`). ~5k chars/day anonymous, ~50k with `mymemory_email` set. Rejects text over 500 bytes, so long selections skip straight past it. |
| 3 | DeepL    | yes | Best quality where it has the language. Skipped entirely unless `deepl_api_key` is set. Free keys end in `:fx` and are routed to `api-free.deepl.com` automatically. |

When every provider refuses, the bubble lists each one's reason rather than a
generic failure.

## Checking the backends

```sh
./target/release/bubbleTranslate --check                # probe all three
./target/release/bubbleTranslate --translate "merhaba"  # run the chain once
```

Neither opens a window or needs any permission, which is how you tell a backend
problem apart from a capture problem. The same probe is available in the main
window under **Providers › Test providers**.

To trace the whole pipeline stage by stage:

```sh
pkill bubbleTranslate
BUBBLETRANSLATE_DEBUG=1 ./bubbleTranslate.app/Contents/MacOS/bubbleTranslate
```

Each stage logs one line (`mouse-up`, `capture`, `translate`), so a selection
that produces no bubble can be traced to the gesture filter, the capture, or
the provider chain.

## Config

`~/Library/Application Support/bubbleTranslate/config.toml` on macOS,
`~/.config/bubbleTranslate/config.toml` on Linux. Written on first run.

```toml
target_lang = "en"          # what to translate into
source_lang = "auto"        # or a fixed code
providers = ["google", "mymemory", "deepl"]
deepl_api_key = ""
mymemory_email = ""         # raises the MyMemory quota
auto_translate = true       # bubble on selection
min_chars = 2
max_chars = 4000            # keeps a stray Cmd+A out of the queue
debounce_ms = 180           # settle time before reading the selection
clipboard_fallback = true   # macOS only: synthesize Cmd+C when AX is empty
watch_clipboard = false     # also translate on copy, for apps with no selection
auto_hide_secs = 12         # 0 = stay until closed; pauses while hovered
font_size = 15.0
ui_scale = 1.0              # whole-interface scale, on top of the display's
```

Target language, auto-translate and the DeepL key are also editable from the
bubble's ⚙ menu. Changing the language re-translates the text already captured.

## Known limits

- GNOME's Wayland session cannot be watched at all; see above.
- The prebuilt Linux binary needs glibc 2.43 or newer, which rules out the
  current Debian and Ubuntu releases. Build from source there.
- A Linux session with no StatusNotifierItem host gets no tray icon, and there
  the main window is the only way back to the app, so closing it quits.
- Restoring the clipboard after a synthetic copy only preserves text.
- The bubble never takes keyboard focus (by design — otherwise the source app
  would drop its selection), so it cannot be dismissed with Esc. Close it with
  ✕, let it auto-hide, or just select something else.
- Keyboard-initiated selections anchor the bubble at the mouse pointer, not at
  the caret.
