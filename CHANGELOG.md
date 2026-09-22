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

## 0.2.8 — 2026-09-22

**The bubble is no longer a black rectangle. (Windows)** On some machines —
virtual machines and remote desktops especially — every translation came up as
an empty black box: the text had been fetched and there was simply nothing
readable on screen. The bubble has no title bar and no border, and asking for a
window that way turns out to be what some graphics drivers decline to draw at
all. It now asks for an ordinary window, which is always drawn, and trims the
frame off itself, so it looks exactly as it did before on the machines where it
already worked. This was not new in 0.2.7; it needed a Windows machine of the
wrong kind to show up.

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
