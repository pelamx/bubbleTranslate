# What changed

Every release of bubbleTranslate, in plain language, newest first. This file
is written for the people using the app rather than for the people building
it: it says what is different and why it was worth changing, not which files
moved.

A release is dated on the day it was published. "Unreleased" is what is
already built and waiting for the next one — if you are reading this on
GitHub, it is not in the download yet.

Where a change only affects one system, the entry says so. macOS, Windows and
Linux are released separately, so a version number can appear on one of them
days before the others.

## 0.3.2 — 2026-09-24

**Translate text that cannot be selected, by drawing a box around it.
(Linux)** What Windows and macOS gained in 0.3.0 comes to Linux. Press
Ctrl+Shift+E, drag a rectangle over anything on the screen — a screenshot, a
video, a scanned page, a game — and what is inside it is read and translated.
Escape, or the right mouse button, cancels.

It needs Tesseract, which does the reading, and a language pack for each
language you read from: `tesseract` and `tesseract-data-eng` on Arch,
`tesseract-ocr` and `tesseract-ocr-eng` on Debian and Ubuntu. Tesseract is a
separate package rather than part of the download because each language adds
tens of megabytes, and this way you only install the ones you read. The
reading happens on your own computer and the picture is never sent anywhere.

It works on X11 and on Hyprland, sway and the other wlroots desktops. GNOME
and KDE on Wayland do not let applications take a picture of the screen this
way, so there the key does nothing yet. On Hyprland the key is bound in the
compositor, so the window you are reading does not also react to it. Anywhere
else, `bubbleTranslate --read-screen` does the same thing from a keybinding of
your own.

## 0.3.1 — 2026-09-24

**Reading the screen works every time, not just the first. (Windows)** Pressing
Ctrl+Shift+E a second time did nothing at all — no dimmed screen, no crosshair,
nothing — and the only way to get it back was to quit bubbleTranslate and start
it again. One reading per launch is not something anyone can use, and it was
there from the moment 0.3.0 was published.

## 0.3.0 — 2026-09-24

**Translate text that cannot be selected, by drawing a box around it.
(Windows)** Press Ctrl+Shift+E, drag a rectangle over anything on the screen,
and what is inside it is read and translated. A screenshot somebody sent you,
a still from a video, a scanned page, a game, a remote desktop — until now
those were the one thing bubbleTranslate could not help with at all. There is
text right there on the screen, but nothing to select, so there was nothing
to ask any application for. Escape, or the right mouse button, cancels
without translating.

The reading happens on your own computer and the picture is never sent
anywhere, not even to us. Which languages can be read depends on what
Windows has installed: it has to recognise the language you are translating
*from*, so pointing it at a language your Windows was never set up for gives
confident nonsense rather than an error. Windows Settings, under Time &
language, is where those are added.

**The same, on a Mac, with ⌘⇧E. (macOS)** It puts up the crosshair the Mac's
own screenshot tool uses, so multi-display, window mode on the space bar and
Escape-to-cancel all come with it. The translation appears under the box you
drew rather than over it, so you can compare the two.

The first time you press it, macOS asks whether bubbleTranslate may record the
screen, and opens the right settings page. Nothing is captured until you say
yes, and nothing is asked during installation — if you never press ⌘⇧E you are
never asked at all. macOS only grants that to a freshly started app, so quit
bubbleTranslate from the menu bar and open it again afterwards.

Reading happens on the Mac itself, and the picture is deleted the moment the
words have been taken out of it. Before it is read the picture is doubled in
size and its colour drained to plain contrast, which sounds like a detail and
is not: on a poster title in slanted hand-lettering it is the difference
between two broken fragments and every word coming back. macOS reads far more
languages than it is set up for, Turkish among them, so unlike Windows there
is nothing to install first.

## 0.2.9 — 2026-09-23

**A Bluetooth mouse keeps working after the laptop sleeps. (Linux)** A
wireless mouse that reconnected after sleep, or was switched off and on, went
unnoticed until bubbleTranslate was restarted. Ordinary selections still
translated, so it looked fine, but pages like Google Drive's PDF preview, which
wait for the mouse button to come up, never produced a bubble. A mouse or
keyboard that connects while the app is running is now picked up within a
couple of seconds.

**Tapping on the trackpad now works in Google Drive's PDF preview. (Linux)**
With tap-to-click on, double-tapping a word, or tap-and-dragging over a
sentence, while holding the trigger key brought up no bubble there. Only a
physical press of the pad worked. Taps now work the same as clicks, so you can
pick out a single word the way you would with a mouse.

**Copies of 0.2.7 and older say when an update is out again.** When the
downloads moved to their new home, the file those versions check for updates
went with them, and they quietly stopped noticing new versions. It is back
where they look, so the "a new version is available" notice appears again and
leads to the current download.

## 0.2.8 — 2026-09-23

**Text in Google Drive's PDF preview translates even when a screenshot is on
the clipboard. (Linux)** Pages like that one only hand over text when it is
copied, so bubbleTranslate briefly borrows the clipboard and then puts back
what was there. It used to refuse whenever the clipboard held an image, and the
bubble never appeared. Now it saves whatever the clipboard holds, images
included, and restores it exactly.

**The bubble draws on every machine now, rather than coming up as a black box.
(Windows)** On some machines — virtual machines, remote desktops, and PCs whose
graphics driver was never properly installed — every translation appeared as an
empty black rectangle: the text had been fetched and there was simply nothing
readable on screen. On the worst of them the app stopped responding and could
not be closed either, so the next launch started as a second copy and dropped
its own black bubble on top of the first.

The cause was how the app asked to be drawn. It used OpenGL, which is the part
of a graphics driver most likely to be missing or half-finished on exactly
those machines. It now draws the way Windows draws the desktop itself, and
falls back to software rendering where there is no graphics card worth the
name. On machines where the bubble already worked it looks exactly as it did.
This was not new in 0.2.7; it needed a Windows machine of the wrong kind to
show up.

**The downloads have moved, and this version knows where.** They are published
from their own place now rather than from the same page as the source code, so
every download link on the website points somewhere new. Nothing about
installing or updating changes for you: this version checks the new address
for newer builds, and the old links keep working. A copy older than this one
will stop noticing new versions once the old page closes, so it is worth
taking this update.

## 0.2.7 — 2026-09-22

**Everyone on the free version gets ten translations a day, and only Pro is
unlimited.** Copies installed before the daily limit existed used to keep
unlimited use, but the app could only tell them apart by a file on the
computer, and deleting or editing that file turned any copy into an unlimited
one. That no longer works: if the file is deleted or has been changed, the app
counts that day's ten as used, and a fresh ten arrives at midnight as usual.
Nothing changes for anyone who left the file alone.

## 0.2.6 — 2026-09-21

- The app's own interface now speaks English, Turkish and Spanish. Three
  buttons in the top-right corner of the main window — EN, TR, ES, the same as
  on the website — switch the window and the bubble in one click, and the
  choice is remembered. English stays the default.

## 0.2.5 — 2026-09-21

**The bubble waits for you to finish selecting. (Linux)** On a Wayland desktop
nothing tells an app that a sweep of text has ended, so the translator had to
guess from a short pause — and a pause in the middle of selecting, or a double
click that you then dragged into a longer selection, could throw the bubble up
before you were done, over half a phrase or over the wrong text. Where the
mouse can be read it now holds off until you let the button go, the way it
already did under X11, so the bubble arrives at the end of the selection and on
what you actually picked. On a session where the mouse cannot be read it falls
back to the old timing, and the settings window says so.

## 0.2.4 — 2026-09-20

**The feedback box names Apple Mail on a Mac.** The first choice beside "Write
the mail" opens whatever your system treats as its mail program — on a Mac
that is Apple Mail unless you have changed it — but calling it "my mail app"
hid a perfectly good answer behind a vague one. It now says what it will
actually open, on each system.

## 0.2.3 — 2026-09-20
_0.2.3 was published for Linux only; macOS and Windows go straight from 0.2.2
to 0.2.4._


**There is a way to write in from inside the app.** A "Send feedback" box at
the bottom of the window takes a complaint or a suggestion and opens it, already
addressed to pelamx@bubbletranslate.app, in **your mail app, Gmail or
Outlook.com** — whichever you pick, and it remembers. If you read your mail on
the web, pick Gmail or Outlook: a browser that is not the system's mail handler
answers the mail app option with an empty tab, which looks like a broken button
and is not.

Either way the message goes out through your own mail, not through us —
nothing is sent from the app itself, you see it before it leaves, and you keep
a copy in your sent mail. The version and system are added at the end so a
reply can make sense, and you can delete them. One button copies the address if
you would rather write from somewhere else entirely.

**Single words now translate in Google Drive's document preview.** (Linux)
Drive draws its own text instead of handing it to the desktop, so
bubbleTranslate copies the selection itself to read it — and it only did that
after a drag. Picking a word out with a double click, which is how most people
select one word, produced nothing at all. Sentences worked, single words did
not. Double clicks and triple clicks now count, and still only with Shift held.

**Installing a new version over a running one works.** (Windows)
Launching a second copy of bubbleTranslate brings the first one's window
forward instead of starting a rival — which was also what happened when you
downloaded a newer version and double-clicked it. The window appeared, nothing
had changed, and the update notice was still there. The two copies now compare
versions: the older one quits and the newer one takes over.

**The Windows download is a zip.** A browser given a bare, unsigned `.exe`
calls it uncommon and throws it away unless you dig it back out of the
warning. The same program inside a zip arrives normally, and at 7 MB instead of
17. The `.exe` is still on the release page for anyone who wants it.

**The name is written bubbleTranslate everywhere**, in the app and on the
website — lowercase `b`, capital `T`.

## 0.2.2 — 2026-09-19

**The app says which version it is running**, in its settings window. Until
now the only way to tell was to check the file you downloaded.

**The first launch explains itself before it acts.** Starting the app for the
first time says what it is about to ask for and why — the accessibility
permission on macOS, the input group on Linux — rather than opening a system
dialog with no context.

## 0.2.1 — 2026-09-19

**Buying Pro is checked more carefully.** The payment webhook now verifies the
amount that was actually charged against the one that was asked for, and the
signature check and the stored token were both tightened. Nothing about this
is visible while it works; it is here because it changed.

## 0.2.0 — 2026-09-18

The first release with Pro: 10 translations a day free, unlimited on a licence,
on up to three machines.

---

Entries before this line were written after the fact, from the history, when
this file was started on 2026-09-20. Everything from "Unreleased" down to
0.2.0 above is accurate but summarised; from the next release onward each entry
is written as the change is made.
