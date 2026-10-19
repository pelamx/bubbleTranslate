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

## Unreleased

**The daily usage ping says a little more, and says it about whole days.**
The once-a-day "in use" ping now also carries how many requests went to Claude
on your own Anthropic key and how many tokens they used, how many screen
readings you made and whether you ran out of the free ones, and whether this
copy came from the website or from a store. As before, it is only counts:
never the text, the picture, the languages or your key. Claude requests go
from your computer straight to Anthropic, so until now there was no way to
know what reading the screen through Claude costs people, or whether it is
failing for them. Counts are now sent for each finished day, so they are
complete. Before, the ping went out as the day began and carried almost
nothing. The ping can still be turned off the same way as before, and that
stops all of it.

## 0.4.19 — 2026-10-19

**A single word is translated in the sentence it came from, if you have an
Anthropic key. (macOS, Windows)** Until now a selected word was translated on
its own. Most words have several meanings, and the translators picked the
commonest one, so "bank" came out as the building in a sentence about a river.
Now the word goes to Claude Haiku with the sentence around it, and the
translation is the meaning that sentence uses. The word's other meanings are
still listed underneath. This works where the app says what text surrounds a
selection: text fields, Mail, Notes, TextEdit, Safari and Chrome on a Mac, and
most Windows apps. Elsewhere, and without a key, a word is translated as
before. A word costs about a hundredth of a cent, billed to your own
Anthropic account. You can turn it off in Settings, under the Anthropic key.

## 0.4.18 — 2026-10-18

**Claude reads and translates the screen, if you have an Anthropic key.** A
box (Ctrl+Shift+E, ⌘⇧E on a Mac) and the whole screen (Ctrl+Shift+F, ⌘⇧F) are
now sent to Claude Haiku as a picture. Claude reads the text itself and
translates it. The computer's own reader gets small text wrong: "rn" for "m", a
dropped accent, an 8 that was a 3. Until now the translators were handed those
mistakes and translated them as written, one paragraph at a time. Now a wrong
number comes out right. A box in a language this computer cannot read is still
translated. A menu word comes out as a command rather than a verb, because
Claude sees the whole screen at once.

The picture leaves your computer when this is on, and the window says so. A
screen costs about a tenth of a cent, billed to your own Anthropic account.
Without a key nothing changes. If Claude cannot be reached, the screen is read
and translated as before. You can turn it off in Settings, under the Anthropic
key.

## 0.4.17 — 2026-10-16

**A contents page is translated whole, every entry alike.** On a book's
contents page each line is set in two typefaces — "CHAPTER 3" in small bold
capitals, the title in a book face — and no one installed typeface matches
both, so some entries were taken for a misreading and left in English, some
chapters lost their number, and the entries that were translated came out a
size larger or smaller than one another. A line the reader is sure of is now
translated even when no one typeface fits all of it, and the entries of a list
are set in one typeface and one size, as they were printed. (Windows's own
reader does not say how sure it is, so there a line in two typefaces is still
left as it is, unless the screen is read by Google Vision.)

**Buttons and icons are no longer painted over.** In Google Drive's PDF viewer
the row of buttons above the page — "Open with Google Docs", "Share" — and the
page-number box could be taken for a heading in spaced-out letters and painted
over with their translation, wiping the buttons out; and a browser's reload
arrow, read as the letter C, came back as "Ç". Spaced-out headings are now
recognised only in capitals, which is how they are set, and a letter standing
on its own is left as it is.

**A menu under a title stays a menu.** In Google Drive's PDF viewer the
file's name sits right above the "File" menu, and the first menu item was read
as the last word of the title — "Dark Territory….pdf File" — and translated
together with it. A row of short items side by side — a menu, tabs, a row of
buttons — is now kept apart, each item on its own. Lines of a tightly set page
also come back whole more often: a line the reader split down an imaginary
column, or whose box reached into the line above, is no longer left in English
or set a size too large.

**Everything listed under 0.4.16 comes to Linux with this version. (Linux)**
0.4.16 was released for Windows first. The paragraphs of a printed page kept
whole, titles in large type and in spaced-out capitals, translation in context
and the bubble that says when nothing on the screen could be read all arrive
on Linux now.

## 0.4.16 — 2026-10-16

**A heading set in spaced-out capitals is translated now, not left behind.**
On a printed page — a PDF in Google Drive's viewer, a book cover — a chapter
title or headline is often set in wide-spaced letters. Its letters sit so far
apart that no installed typeface lined up with them, so the whole-screen
translation judged it unreadable and left it in the original language, even
while the paragraphs under it were translated. It is now recognised as a
spaced title and translated like the rest, wherever the paper behind it is
plain.

**Translating the whole screen reads the interface, not only your language.
(macOS)** When you have set a source language rather than leaving it
automatic, ⌘⇧F used to read only that language and leave the English interface
around the content — menus, buttons, a sidebar — untranslated. It now reads
English alongside your language, so the whole screen comes back rather than
only the part in the one language.

**A translated screen is now translated in context.** Each paragraph is
translated as part of the whole page rather than on its own: with a DeepL key
the rest of the screen is sent along as context — DeepL reads it to choose
between meanings but does not translate it — so a word that could go two ways
is settled by what surrounds it. The other providers already translate each
paragraph whole.

**The screen is now read in the language you are translating from. (Windows)**
Windows reads the screen one language at a time, and it was always asked for
the language Windows itself is in — never for the language on the screen. Ask
it for the wrong one and it does not refuse: it guesses, and a page comes back
as letters that were never on it, which is why some lines translated and most
did not. It is now asked for the language you translate from, and falls back
to the computer's own only when you have left the source on "detect it".

**The window says which languages your computer can read, and what to do when
yours is not one of them. (Windows)** Reading the screen needs a recognition
pack per language, a fresh Windows often has only the one it was installed in,
and until now nothing anywhere said so — the screen simply came back wrong.
"Read the screen" now lists what Windows reads, says plainly when the language
you read from is missing, and gives the path in Settings that adds it.

**Small text on the screen is read better. (Windows)** A book page or a PDF at
its own size puts letters about eight pixels tall on an ordinary display,
which is under what the reader can make out. The picture is now enlarged
before it is read — on a display already magnifying everything, where the
letters arrive big enough, nothing changes.

**When nothing on the screen can be read, it says so.** Pressing the key and
getting no answer at all is indistinguishable from the app being broken. Now a
bubble says the screen could not be read and where to look, which on Windows
is nearly always a missing recognition pack.

**Translating the whole screen keeps a printed page's paragraphs whole.** On a
page of print — a PDF in Google's viewer, a book — a line was sometimes read
as two halves, cut down the middle of the page, and the halves were translated
apart: the sentence came out in pieces, half in one typeface and half in
another. They are now put back together first, so each paragraph is translated
as one, in the typeface and size it was printed in, inside its own column; and
a paragraph that starts with an indent stays a paragraph of its own instead of
running on from the one above. A short word at the start of a line, like
"By", and a label with an icon in front of it, like a "Comment" button, are no
longer left untranslated, and words like "eSignature" or "iPhone" are no longer
mistaken for code.

**Titles and headlines set in large type are translated too, as one
sentence.** A title page or a poster — "THIS IS / HOW THEY / TELL ME / THE
WORLD / ENDS", in tall narrow capitals — came back with words missing, or not
at all: letters that large are hard to read at the size of the screen, and no
installed typeface is that narrow. Large type is now read a second time at
half the size, the closest typeface is narrowed or widened to the original's
shape, and a title broken over several lines is translated as the one sentence
it is, in capitals when it was in capitals. (Linux reads the large type again;
the rest is the same on every system.)

## 0.4.15 — 2026-10-16

**The translate box is first in the window, and the shortcuts are said
once.** Typing or pasting something to translate is what the window is most
often opened for, so the box now sits at the top, above "How to use it". That
section now shows only the selection key and Esc: the screen-reading
shortcuts were listed both there and in "Read the screen" just below, and now
appear once, with their explanation.

## 0.4.14 — 2026-10-16

**Translate the whole screen at once with ⌘⇧F. (macOS)** Every paragraph on
the screen is replaced by its translation, in place — in the same font, size
and colour it was printed in, on the lines it was on, the way it already works
on Linux and Windows. It works on anything the screen shows: a PDF, a
terminal, a game, an app that was never translated; code and text on a picture
are left as they are. Press any key or click to put it away. The screen is
read on your own Mac by the recogniser the system ships, so nothing of it
leaves the machine. It is also in the window, under "Translate text you cannot
select".

**Translate the whole screen at once with Ctrl+Shift+F. (Windows)** Every
paragraph on the screen is replaced by its translation, in place — in the same
font, size and colour it was printed in, on the lines it was on, the way a
translated web page looks. It works on anything the screen shows: a PDF, a
terminal, a game, an app that was never translated. Code, file names and
commands are left as they are, and so is anything that could not be replaced
cleanly. Press any key or click to put it away. It arrived on Linux in 0.4.13
and does the same thing here; until now, a page on Windows meant drawing a box
with Ctrl+Shift+E over each part of it in turn. It is also in the window, under
"Translate text you cannot select".

The screen is read by the text recognition built into Windows, so nothing
leaves the computer and there is nothing to install — on a machine that reads
only one language, that is the one language it will read, which is the same
list Ctrl+Shift+E has always used.

**A Google Vision key now works on Windows. (Windows)** The settings have
offered one for a while and the window said the picture was sent to Google to
be read; on Windows it was not, and the key did nothing. Now it is used for
both ways of reading the screen — the box and the whole of it — and faint text,
small text and scripts Windows has no pack for come out right. Without a key
nothing changes and nothing leaves the computer.

## 0.4.13 — 2026-10-16

**The window now says how to use it, at the top.** Hold Alt — Option on a
Mac — and select text or double-click a word; Ctrl+Shift+E for text you cannot
select; Ctrl+Shift+F for the whole screen (on Linux); Esc to put either away —
each with its key, and the key is whichever one you chose in the settings.
Below them, a few tips: what the bubble's buttons do, how the language you
read from is found, and what the free version includes. The app shows nothing
until you ask it to, so until now the gestures were easy to miss.

**Translate the whole screen at once with Ctrl+Shift+F. (Linux)** Every
paragraph on the screen is replaced by its translation, in place — in the same
font, size and colour it was printed in, on the lines it was on, the way a
translated web page looks. It works on anything the screen shows: a PDF, a
terminal, a game, an app that was never translated. Code, file names and
commands are left as they are, and so is text on a picture that could not be
replaced cleanly. Press any key or click to put it away. Until now a page
meant selecting it piece by piece, or drawing a box with Ctrl+Shift+E for each
part. It is also in the window, under "Translate text you cannot select".

**Reading the screen can use Google Vision, with a key of your own. (Linux)** Enter a
Google Cloud Vision API key under Settings and the screen — a box or the whole
of it — is read by Google instead of on your computer: faint and small text,
accented letters and every script come out right, in about a second. The
picture of the screen is sent to Google to do it, and the window says so while
the key is set. Without a key nothing changes and nothing leaves your
computer; if Google cannot be reached, the screen is read on your computer as
before.

**Reading the screen has its own daily allowance: three a day on the free
version.** A box drawn with Ctrl+Shift+E and the whole screen with
Ctrl+Shift+F count together, and no longer use up the ten translations of
selected text, which stay ten. Both come back at midnight, and Pro has no limit
on either. The bubble says how many screen readings are left, and when they
run out it says so instead of opening the overlay for nothing.

## 0.4.12 — 2026-10-15

**When the free translations run out, the bubble shows what the yearly plan
saves.** It now reads "$20/year (2 months free) or $2/month", yearly first.
Someone who uses up the day's translations is using the app every day, which
is exactly who the yearly plan is cheaper for — and the bubble never said so.

**Cancelling asks why, if you want to say.** The cancel form on the account
page now has an optional question and a box for a few words. Nothing is
required: the cancellation goes through whatever you answer, or if you answer
nothing. What people say is how we find out what to fix.

## 0.4.11 — 2026-10-15

**When only a few free translations are left, the bubble offers Pro beside the
count.** The "3 free translations left today" note now has a Go Pro link next
to it. Until now the way past the daily limit only appeared after the last
translation was gone, when the bubble had nothing else to give you.

**The window shows bubbleTranslate's icon in its corner. (Windows)** The
window that opens from the tray had an empty space at the top left where every
other program has its icon. It now carries the speech bubble with the globe.

**bubbleTranslate has its own icon on the Mac. (macOS)** Until now the app
had none, so in System Settings — under Accessibility and Screen Recording,
exactly where you are asked to find it and switch it on — it showed up as a
blank square that was easy to miss. It now shows the speech bubble with the
globe, the same mark as on Windows and Linux.

**The icon in the title bar and the taskbar is the bubble again, with the
world inside it (Windows).** At those sizes it had been the globe on its own:
the bubble around it was dropped because shrinking the full mark left four grey
smudges and a globe too small to recognise. Drawing it for the small grid
rather than shrinking it onto one gets both — the bubble reaches the edges of
the icon, the tail is wide enough to survive, and the globe keeps the three
lines that still read at sixteen pixels.

## 0.4.10 — 2026-10-14

**New copies use Alt to translate — Option on a Mac — instead of Shift.** Hold
Alt and select, or hold it and double-click a word, and the bubble appears.
Shift had a catch: it is also the key that extends a selection, so a
double-click with it held often selected everything from the last selection up
to the new word, and translated all of that instead of the word. Alt leaves
the selection exactly as you made it. If you already use bubbleTranslate,
nothing changes: it keeps the key you have, and the window's Behaviour section
is where you switch to Alt.

## 0.4.9 — 2026-10-14

**The Mac download opens without a warning (macOS).** Until now the first
launch was blocked with "Apple could not verify bubbleTranslate is free of
malware", and the only way past it was a trip to System Settings to press Open
Anyway — enough to make a careful person give up, or wonder whether the
download was safe. bubbleTranslate is now signed with an Apple Developer ID and
checked by Apple before it is published, so it opens like any other app.

Because the app is signed differently, macOS asks for the Accessibility
permission once more after this update. If bubbleTranslate already shows as on
in System Settings › Privacy & Security › Accessibility but no bubble appears,
remove it from that list with the minus button and turn it on again. Updates
after this one keep the permission.

## 0.4.8 — 2026-10-08

**The ten free translations a day now actually stop at ten.** Once the day's
ten were spent, anything translated earlier that day could be translated again
without limit — so a free copy used on the same handful of phrases was never
asked to stop, and the daily allowance had no end to it in practice. Switching
the target language got around it the same way. Re-reading a sentence is still
free, as it always was, but only while there is something left in the day;
after the tenth translation the bubble says so and offers Pro, whether the text
is new or not.

## 0.4.7 — 2026-10-08

**The icon is back in the Start menu and on the taskbar (Windows, Microsoft
Store).** The version installed from the Store arrived without one: a blank
square in the Start menu, on the taskbar and in Alt+Tab, which made it hard to
find and made a working app look broken. The pictures were in it all along —
nothing told Windows where to look for them. The copy downloaded from the
website was never affected and is unchanged.

## 0.4.6 — 2026-10-06

**The download installs itself (Linux).** Until now the Linux download was
only a file: it ran from wherever the browser saved it, never showed up in the
application menu, and had to be found again in Downloads every time. Now the
first time you run it, bubbleTranslate puts itself in your home folder,
adds itself to the application menu with its icon, and starts from there,
including at login. Running a newer download later updates that copy and
replaces the one that is running, so updating by hand is just downloading and
running the new file. Nothing needs a password.

## 0.4.5 — 2026-10-06

**Update with one click (Windows, Linux).** When a new version is out, the
window now has an "Update now" button, and the bubble an "Update" link, where
before there was only a download. One click fetches the new version, checks
that it arrived intact, swaps it in and restarts bubbleTranslate, with nothing
to unzip or replace by hand. Your licence, settings and today's free
translations stay as they are. If it cannot be done from there — no
connection, or the app sits in a folder it may not write to — the download
button comes back instead. This works from the next update after this one:
getting this version is still the usual download. On macOS the download stays
as it is for now.

**DeepL now steps in when Google and MyMemory both fail, with no key needed.**
Until now, when both free translators were down or had blocked you for too
many requests, the bubble showed an error unless you had entered a DeepL key
of your own. Now it asks DeepL on bubbleTranslate's behalf instead, so a
translation still arrives. It is a backup, so it is limited: 50 translations
a day per computer, selections up to 1,500 characters, and a shared daily
total for everyone. The text goes through bubbleTranslate's server to DeepL
for that one translation and is not kept. If you have your own DeepL key, it
is used exactly as before.

## 0.4.4 — 2026-10-06

**A short "still open" signal while the app runs.** Besides the anonymous
daily ping, a running copy now says it is still open every five minutes,
carrying only the same anonymous install id — no text, no languages, not even
the version. The daily count shows how many people opened the app on a given
day, but not how many are using it at this moment, which is what tells us
whether a change is being felt right away. `usage_ping = false` in the config
file turns this off along with the daily ping.

## 0.4.3 — 2026-10-05

**The tray icon is back, and closing the window no longer quits (Linux).**
In 0.4.2 closing the window stopped bubbleTranslate completely, so there was
no bubble until you opened it again. Now closing the window leaves the
translator running in the background with its icon on the bar: click the icon
to bring the window back, and right-click it and choose Quit to stop it.
Opening bubbleTranslate from your launcher while it is already running also
brings the window back, instead of starting a second copy.

## 0.4.2 — 2026-10-05

**No more tray icon (Linux).** The icon in the system tray is gone. On some
bars it showed no menu at all, which left no way to quit the app. Now the
window is the app: closing it quits bubbleTranslate, and opening it again from
your launcher starts it back up.

**The Omarchy bar widget is gone (Linux).** Like the tray icon, it stayed on
the bar with no way to close it, and showed "not running" whenever the app
was closed. If an earlier version added it to your bar, starting this version
takes it off again.

**The daily usage ping survives a sleeping computer.** The anonymous "this
copy is running" signal is still sent at most once a day, but it is now sent
on the first day it has not gone out yet, rather than 24 hours after the last
one. A computer put to sleep every night stopped that 24-hour clock, so a copy
used every day could go uncounted for days at a time. Nothing more is sent
than before, and `usage_ping = false` in the config file still turns it off.

## 0.4.1 — 2026-10-01

**A single word shows its other meanings.** Select one word and, under the
translation, the bubble lists what else it can mean, grouped by part of
speech — "run" comes back as *koşmak*, with the verb's other senses, the noun
and the adjective underneath. One word rarely has one translation, and the
one picked is often not the one the sentence around it meant; now you can
see the alternatives without opening a dictionary. It only appears for a
single word, only when Google did the translating, and it can be turned off
in the window's Behaviour section.

## 0.4.0 — 2026-10-01

**Esc closes the bubble.** Press Esc and the bubble goes away, whichever
window you are in. Until now the only ways to get rid of it early were
reaching for the ✕ or waiting for it to fade. Esc still reaches the window you
are working in, so a dialog or a menu there closes as it always did. On Linux
it works on X11, on Hyprland, and wherever the app can read the keyboard (the
`input` group); on other Wayland desktops, ✕ and the auto-hide remain.

**The bubble can match your Omarchy theme.** (Linux) On Omarchy, the theme
list has a new first choice, "Omarchy (follows the desktop)". With it, the
bubble takes its colours from whatever theme the desktop is using and changes
when you switch themes. Until now you had to pick the nearest built-in palette
by hand and pick again after every theme change.

**A bubbleTranslate button for the Omarchy bar.** (Linux) Run
`bubbleTranslate --omarchy-install` once and a translate icon appears on the
bar. Click it to stop bubbles from popping up on their own, for a presentation
or a screen share, and click again to bring them back. The icon dims while
they are off, and hovering it shows how many free translations are left today.
`bubbleTranslate --omarchy-remove` takes it away again.

## 0.3.9 — 2026-09-30

**The Pro page opens in your language.** Pressing "Upgrade to Pro" always
opened the purchase page in English, even when you were using the app in
Spanish or Turkish, right when you were deciding whether to pay. The page now
follows your browser's language already, without an update. From this version
on, it opens in whatever language the app is set to.

**A copied password no longer ends up in clipboard history.** (Windows,
macOS) Sometimes the app has to copy your selection to read it, and it then
puts back what you had on the clipboard. That restore was not marked as
private, so Windows clipboard history (Win+V), cloud clipboard sync or a
clipboard manager on the Mac could record it — including a password you had
just copied from a password manager. The restore now carries the same "keep
this private" markers password managers use.

**Your settings file is private to you.** (macOS, Linux) It holds your licence
key and any DeepL or Anthropic key you entered. Until now other accounts on
the same computer could read it. It is now readable by you alone, from the
next time the settings are saved.

**The update notice only links to our own downloads.** It used to accept any
secure web address. It now only accepts a download from our releases page, so
a tampered update file cannot point you somewhere else.

## 0.3.8 — 2026-09-28

**The bubble can read the translation aloud.** A Listen button sits next to
Copy. It uses a voice that is already on your computer, so it helps with
pronunciation, and it lets you listen instead of reading. Pressing it again
stops the voice, and the bubble does not close by itself while it is reading.
The button only appears when a voice for that language is installed, so it
never reads Turkish in an English voice. (Linux) It uses Piper when a Piper
voice for that language is installed, and Piper sounds far more natural than
the others. Otherwise it uses Speech Dispatcher or eSpeak, which many desktops
already have. (Windows) Windows cannot tell in
advance which voices exist, so for a language with no voice the first press
does nothing and the button then goes away.

**Buying Pro now emails you the licence key.** Until now the key was shown
once, on the page you land on after paying, and anyone who closed that page
before copying it had to write to support to get it back. It now also arrives
by email from support@bubbletranslate.app, so it is in your inbox for the next
machine. Replying to that email reaches support.

## 0.3.7 — 2026-09-25

**The daily usage ping now also says when the free allowance runs out.** On
the free plan, the first time in a day that the ten translations are used up,
the app sends one more ping saying so — the same anonymous values as the daily
one, nothing about what you translated. It is how we learn whether ten a day is
the right number, which is worth knowing before changing it for everybody.
Setting `usage_ping = false` turns this off along with the rest. The privacy
policy lists it, along with the country the service now keeps for each
installation.

## 0.3.6 — 2026-09-25

**It starts by itself when you log in.** (Windows, Linux) Until now the app
stopped with every restart and stayed stopped until you remembered to open it
again, which most people did not — so the next time there was something to
translate, nothing happened. It now starts in the background at login, with
just its tray icon. "Start when I log in" in the window turns it off.

**The bubble tells you when a new version is out.** The notice used to be only
in the window, and someone who uses the app from the tray never opens it — so
they stayed on an old version without knowing. Once per session, the first
bubble now carries a line with the new version and a download link.

**You see the free allowance running out before it does.** With three or fewer
free translations left for the day, the bubble says how many remain, so the
limit is no longer a surprise in the middle of reading.

**A first run shows how to use it.** The app does nothing visible until you
select text somewhere else, which made a first run look like one that did not
work. Until your first translation, the window opens with a short card saying
what to do.

**Selecting a word in your own language now translates it into the other one.**
Until now, if you translate into Turkish and selected Turkish text, the bubble
handed you back the same words — the providers were asked for Turkish-to-Turkish
and answered, correctly and uselessly, with the input. Worse, it counted against
the ten free translations a day, so reading in two languages quietly spent the
allowance on nothing.

There is now a second language, set in the window under the target language:
*…but if it is already that language*. Set it to English and a Turkish selection
comes back in English, while English selections keep coming back in Turkish. It
starts as English, so this works without changing anything.

Set it to *Leave it alone* and the old behaviour returns — except that the
round trip is no longer charged for, because nothing was translated.

**Claude can now do the translating, with your own API key.** The three
backends that ship are phrase translators, and they are weakest exactly where
this app now spends much of its time: idiom, a sentence whose meaning depends on
the one before it, and text read off the screen with its accents missing, which
a phrase translator renders confidently wrong. A language model reads the
sentence instead.

Paste an Anthropic key into the window and Claude joins the list. It sits last,
so it only answers when the free ones fail — including when a selection is too
long for MyMemory, which until now left long text with no fallback at all. Move
it to the top with the arrows to have it answer first. Nothing is sent anywhere
new without a key, and the cost is billed to your own account, not through
bubbleTranslate.

**Providers added in an update now reach people who already had the app.** The
list of backends is stored in full, so a settings file written by an earlier
version pinned the old set — and since the window only offers what the list
holds, a newly added backend would have been invisible to everyone but fresh
installs. Anything missing is now added to the end of the list, leaving an order
you chose yourself alone.

**Long selections have a fallback again.** MyMemory refuses anything over 500
characters in one go, so a long selection had Google in front of it and nothing
behind: if Google would not answer, there was no translation at all. Long text is
now sent to MyMemory in sentence-sized pieces and joined back together. Line
breaks inside the selection are not preserved that way, which is the right trade
for a fallback — the alternative was nothing.

**The app now keeps a record of which backend answered.** `bubbleTranslate
--health` prints, for today, how many translations each backend produced and how
many times it was asked and could not answer. It is the counterpart to `--check`:
that one asks how the backends are right now, and says nothing about the ones
that were failing all morning.

The same counts ride along on the daily usage ping, which the privacy policy now
describes. They are counts only — never the text, never the languages — and they
are added into a daily total across all installations rather than stored against
yours. They exist because the backend the app leads with is an undocumented
endpoint that can begin refusing at any time, and without them a day when it
breaks for everybody looks exactly like a quiet day. `usage_ping = false` turns
the whole ping off as before, and `--health` keeps working either way.

**The provider test no longer reports MyMemory as broken when you translate into
Turkish.** It probed with a Turkish phrase whatever the target language was, so
for Turkish it was asking for Turkish-to-Turkish — which MyMemory refuses,
correctly, and the status panel and `--check` then showed a permanent failure on
a backend that was working fine. The phrase is now chosen to differ from the
language being translated into.

**The downloads have moved.** Every release so far was published under a second
account, which could no longer be published to; they now live alongside
everything else, and this is the first release in the new place. The buttons on
the website point at it, so a fresh install notices nothing.

A copy already running 0.3.5 or older is the one to watch: it looks for new
versions at the old address. If yours goes quiet and the website shows a
version newer than the one you have, download it once from there — after that
the notices come through as usual.

## 0.3.5 — 2026-09-24

**The headings in the window are bold now.** They were meant to be, and on
every system they came out as slightly paler grey instead: the interface
toolkit has no bold of its own, so asking for it only changed the colour. The
app now loads a bold face from the system at startup and the headings use it,
which makes the sections easier to tell apart at a glance.

Where a system has no such face — some Linux installs — nothing breaks: the
headings stay exactly as they were.

**The window now shows how to read text off the screen. (Linux)**
Ctrl+Shift+E was only described on the website, so anyone who had not read it
there had no way to know it existed. A new *Read the screen* section, right
under the translate box, says what it is for and how to use it in three
steps, with a button to try it straight away.

**Reading the screen works on Linux with nothing installed. (Linux)** Until
now Ctrl+Shift+E needed Tesseract, and without it the key silently did
nothing. The first time you use it, a small built-in reader (12 MB) is now
downloaded and used — the picture itself is still read on your computer and
never sent anywhere. The window shows its progress, with a button to fetch it
ahead of time.

**…but install Tesseract for it to read properly. (Linux)** The built-in
reader only knows letters without accents: ç, ğ, ş, é and ü come out as plain
c, g, s, e and u, and Cyrillic, Arabic or Chinese is not read at all — and a
word missing its letters can translate as a different word. The window says
so, and shows the one command that installs Tesseract with your language for
your distribution, with a button to copy it. Once it is installed Tesseract
does the reading instead, and the window notices by itself — no restart
needed.

**Ctrl+Shift+E now works in windows running as administrator. (Windows)**
Pressing it in Visual Studio, an admin terminal or any other program started
"as administrator" did nothing: Windows keeps an ordinary program from seeing
keys typed into those windows. The screen now dims whatever window is in
front.

**The selection no longer leaves a smear behind it. (Windows)** Dragging a box
could leave stale pieces of the desktop inside it — most visibly a ghost of
the pointer, left where it had been when the key was pressed. The bright
cut-out that caused it is gone; the box is marked by its frame, which looks
the same on every machine and leaves nothing behind.

## 0.3.4 — 2026-09-24

**The Linux download now starts on Ubuntu, Debian, Fedora and the rest.
(Linux)** Until now it only ran on the newest rolling-release distributions:
on Ubuntu 24.04, Debian 13 or anything older it refused to start with a
message about `GLIBC_2.43`, and the only way round it was building from
source. It is now built to run on any distribution from Debian 10 and Ubuntu
20.04 onwards — one download for all of them, and a smaller one, 16 MB rather
than 21.

**Reading the screen works on GNOME and KDE too. (Linux)** On their Wayland
sessions Ctrl+Shift+E used to do nothing, because those desktops do not let an
application take a picture of the screen by itself. It now asks the desktop
for one, the way screenshot tools there do. The first time, your desktop may
ask whether bubbleTranslate is allowed to take screenshots; if it will not
hand one over quietly, its own screenshot tool opens instead, and you choose
the region there.

**A rectangle dragged tightly around a line reads it better. (Linux)** Letters
that touched the edge of the rectangle were sometimes misread or missed.

## 0.3.3 — 2026-09-24

**The crosshair is there the moment you press the key. (Windows)** Ctrl+Shift+E
dimmed the screen but left the pointer as the spinning "busy" ring until you
moved the mouse — which says the app is working on something, when what it is
actually doing is waiting for you to draw a rectangle. It is the crosshair
straight away now, before anything has moved.

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
