# Feature analytics, errors and remote configuration

The existing `/admin` dashboard includes **Feature analytics**, **Error
monitoring**, and **Remote configuration**. This is part of the existing
Cloudflare Worker and D1 service; no third-party analytics credentials are
needed.

## Rollout

Apply `schema.sql` before deploying the Worker, then ship a desktop build
containing these changes:

```sh
cd service
npm run schema:remote
npm run deploy
```

These commands change production. Tests apply the schema to their isolated
local database automatically. Neither the Worker nor the desktop release is
published by making the source changes. Existing desktop releases continue
using their old reporting and update behavior; the new metrics cannot be
reconstructed retrospectively. The desktop version must be bumped as part of
the next normal release.

## What is counted

New desktop clients retain 31 days of daily counters in `metrics.json`, next
to `config.toml`. They report cumulative snapshots every five minutes and on normal shutdown via
`/v1/ping`. The server takes the maximum for each install/day/version/channel/
feature/outcome, so duplicate, reordered, and retried snapshots do not inflate
the new metrics. Existing provider/Claude/screen legacy delta reports are
unchanged and retain their existing delivery semantics.

The panel shows successful translations, echoed text (not counted as success),
quota refusals, and failures for selection, clipboard capture, manual input,
OCR regions, full-screen OCR, and retranslations. A full-screen translation
counts as one page, not one request per paragraph. Copy actions and speech
starts are counted separately. Selection attempts include gestures that may
not produce text; they should not be added to clipboard attempts to calculate
total requests. Per-install translation counts and repeat usage across at
least two days are shown. Repeat usage is not a cohort retention rate and
does not prove usefulness or why someone stopped using the app.

Fixed error codes cover capture readiness, monitor startup, empty selection,
OCR capture/recognition, partial screen results, provider-chain failure,
renderer initialization, speech startup, and Rust panics. Failed translations
also report distinct network, HTTP, rate-limit, response-format and unavailable
provider reason codes. These reason rows describe the same failed requests;
they must not be added to the translation failure totals. OCR cancellation
and unavailable capture share a separate code because the platform API does
not distinguish them. Neither is presented as a confirmed crash.

Panic counters are best effort, persisted locally, and sent on a subsequent
launch. Native crashes, forced termination, and power loss are not captured.
There are no stack traces, selected/translated text, screenshots, clipboard
contents, raw exception strings, API keys, or language choices in the new
metrics. Set `usage_ping = false` to disable collection and transmission of
these new counters. Remote settings and update checks continue to work.
The service retains the new metrics for 90 days; dashboard views show 30 days
and exclude the operator's ignored installs.

## Remote controls

Use the authenticated remote configuration form in `/admin`. Select a
platform and channel, enter a JSON object, and save. Each save replaces that
scope's object and adds an audit entry. `{}` clears its overrides.

```json
{
  "debounce_ms": 180,
  "translation_timeout_ms": 10000,
  "disabled_providers": ["mymemory"],
  "dictionary_enabled": false,
  "update_message": "Please update to the latest version.",
  "update_before_version": "0.4.22"
}
```

Supported controls:

- `debounce_ms`: integer 0–2000, selection settling delay.
- `translation_timeout_ms`: integer 2000–30000, default provider request
  timeout. Full-screen Claude requests retain their explicit longer timeout.
- `disabled_providers`: names from `google`, `mymemory`, `deepl`, `claude`.
  Disabling Google also disables Google dictionary lookups. Disabling Claude
  disables its special word/context and screen paths. Disabling every active
  provider yields the ordinary no-provider error; it does not silently enable
  a disabled provider.
- `dictionary_enabled`: `false` temporarily disables dictionary lookups;
  `true` preserves the user's local preference.
- `update_message`: plain text up to 300 characters, shown in the main window.
- `update_before_version`: optional numeric `major.minor.patch` threshold;
  the message is shown only on older builds. It does not force an update or
  change the trusted download destination.

Precedence is global, platform, channel, then platform+channel. Controls are
fetched from `/v1/config?os=linux&channel=download` every five minutes, even
when analytics is disabled. They are held in memory, expire after 24 hours
without a valid response, and never overwrite `config.toml`. A fresh offline
launch uses local settings. The client validates types, ranges, response size,
and allowed fields as well as the server.

The existing version distribution, update history, GitHub release check and
update UI remain the update delivery mechanism. Microsoft Store builds keep
Store-managed updates. No remote executable code or arbitrary download URL is
accepted. Cross-site writes to the configuration form are rejected using the
same origin checks as the other admin actions.
