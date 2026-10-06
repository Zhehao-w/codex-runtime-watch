<p align="center">
  <img src="src-tauri/icon-source.svg" width="156" alt="Codex Runtime Watch icon">
</p>

<h1 align="center">Codex Runtime Watch</h1>

<p align="center">
  A lightweight local desktop monitor for Codex model, reasoning-effort, and provider-routing evidence.
</p>

<p align="center">
  <a href="https://github.com/Zhehao-w/codex-runtime-watch/releases/latest"><img alt="Release" src="https://img.shields.io/github/v/release/Zhehao-w/codex-runtime-watch?display_name=tag&sort=semver&style=flat-square"></a>
  <a href="https://github.com/Zhehao-w/codex-runtime-watch/actions/workflows/ci.yml"><img alt="CI" src="https://img.shields.io/github/actions/workflow/status/Zhehao-w/codex-runtime-watch/ci.yml?branch=main&style=flat-square&label=CI"></a>
  <a href="LICENSE"><img alt="License" src="https://img.shields.io/github/license/Zhehao-w/codex-runtime-watch?style=flat-square"></a>
  <img alt="Windows 11 x64" src="https://img.shields.io/badge/Windows_11-x64-0078D4?style=flat-square&logo=windows11&logoColor=white">
  <img alt="macOS Apple Silicon" src="https://img.shields.io/badge/macOS-Apple_Silicon-111827?style=flat-square&logo=apple&logoColor=white">
</p>

<p align="center">
  <a href="https://github.com/Zhehao-w/codex-runtime-watch/releases/latest"><strong>Download the latest release →</strong></a>
</p>

---

Codex Runtime Watch records the model and reasoning effort Codex selects, runs, and—when explicitly
exposed—reports from the provider. Normal monitoring stays local and passive: it watches Codex activity
without generating extra requests, then presents a compact current-turn view and filterable local history.

> **Provider is evidence-only.** If Codex does not expose provider identity, the application reports
> **Not observed** instead of guessing or copying Runtime into Provider.

Version 0.1.0 supports **Windows 11 x64** and **macOS Apple Silicon**.

## Highlights

- **Selected / Runtime / Provider stay separate.** Runtime is local execution configuration; Provider
  only appears when explicit server/provider evidence exists.
- **Passive by default.** Normal watching is local, incremental, event-driven, and creates no extra
  OpenAI requests.
- **Manual Verify is explicit.** It runs only after a click, sends literal `hi`, and records a separate
  Probe observation.
- **Mismatch and reroute history.** SQLite-backed history keeps factual state transitions, supports
  filtering, and avoids duplicate notifications.
- **Small native desktop footprint.** Tauri 2 + Rust + vanilla TypeScript, with Windows tray/macOS
  menu-bar behavior, themes, notifications, and start-at-login support.
- **No telemetry.** No analytics SDK, remote database, localhost server, prompt capture, or response
  capture.

## Install

Download the current build from the [latest GitHub Release](https://github.com/Zhehao-w/codex-runtime-watch/releases/latest).

| Platform | Package |
|---|---|
| Windows 11 x64 | NSIS `.exe` or MSI `.msi` |
| macOS Apple Silicon | `.dmg` |

Windows builds are currently unsigned and may trigger SmartScreen. Tagged macOS releases are built
with Developer ID signing, hardened runtime, notarization, and stapling; ordinary PR/main CI remains
unsigned and does not require Apple credentials. Windows production distribution similarly benefits
from a trusted code-signing certificate. macOS Intel and Linux packages are not produced.

To build locally on a supported target platform:

```sh
npm ci
npm run generate-icons
npm run tauri build -- --features desktop
```

## Evidence model

| Label | Meaning | Accepted evidence |
|---|---|---|
| **Selected** | The model/effort selected for this turn. | `turn_context.payload.collaboration_mode.settings` first; nested `thread_settings_applied.payload.thread_settings` is the thread fallback. |
| **Runtime** | The local execution configuration recorded for a turn. | `turn_context.payload.model` and `turn_context.payload.effort` (including compatible field aliases). |
| **Provider** | A model explicitly named by server/provider evidence. | Structured provider fields that are actually present in a watched/persisted source, or Manual Verify. |

Runtime is strong evidence about local execution configuration, but it does **not** prove which model
a remote service ultimately served. The app does not subscribe to the Codex app-server notification
protocol, where current `model/rerouted` notifications are exposed. It only adapts compatible records
if they are actually persisted in a watched rollout. The absence of Provider evidence proves nothing
about provider identity. Runtime is therefore never copied into Provider.

Each rollout file is an isolated parsing scope. Its `session_meta.payload.id` supplies the canonical
thread identity when available; otherwise a deterministic file-scope identity derived from the rollout
path plus the turn ID prevents files from mixing or duplicating. Settings are scoped per file/thread;
subagent parent metadata is retained rather than attributed to its root agent. Unknown or malformed
events are skipped without stopping monitoring. Raw model and effort values are preserved, including
future values Codex Runtime Watch has never seen.

## What works

* Event-driven recursive watching under the Codex session directory, incremental JSONL reads, durable
  byte cursors plus per-rollout identity/settings context and a bounded-prefix fingerprint, partial-line
  recovery, truncation/replacement recovery, restart deduplication, and a bounded recent initial scan.
* Every useful normal turn is stored in SQLite, newest first, with All, Mismatches, Runtime, and
  Probes filters; records can be copied, deleted, or cleared.
* A compact vanilla TypeScript UI with system/light/dark themes and factual mismatch results.
* A Windows system tray/macOS menu-bar item that opens the app or Verify page, controls OS login
  startup, and quits explicitly. Closing the window hides it while monitoring continues.
* Native notifications for each newly observed runtime mismatch or explicit provider reroute when
  enabled and OS permission is granted. Provisional provider evidence that only has a thread-level
  Selected fallback is not announced as a provider mismatch; notification decisions use factual state
  transitions so later evidence does not repeat an already-reported mismatch.
* Manual **Verify Backend**, which sends exactly `hi` to the Codex Responses backend only after a
  click using the existing Codex login. Provider is populated only from explicit `OpenAI-Model` or
  `X-OpenAI-Model` evidence in the HTTP response or structured SSE metadata. Generic response `model`
  fields are intentionally ignored; if explicit provider evidence is absent, Verify records a protocol
  failure instead of guessing. Probe rows still classify auth, network, capacity, and protocol failures
  without calling them mismatches.

Codex currently exposes `model/rerouted` through its app-server notification protocol, which this
application does not actively connect to or subscribe to. Normal Provider is populated only when
explicit structured provider evidence is actually available in a watched/persisted source.
Consequently, Provider will commonly remain **Not observed**. This is intended behavior.

## Privacy

There is no telemetry, analytics, tracking SDK, crash upload, remote database, or localhost server.
Normal monitoring makes zero additional OpenAI requests. It stores only model/configuration evidence and
identifiers; it does not store prompts, responses, conversation text, source code, project files, or
credentials. Manual Verify is the only feature that creates traffic. Authentication is read and
used only inside Rust for that request; tokens are never returned to the UI, logged, or persisted.

## Development

```sh
npm ci
npm run generate-icons
npm run tauri dev -- --features desktop
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
npm run typecheck
npm run build
```

### macOS release signing

The tag-only release workflow requires these GitHub Actions secrets:

- `APPLE_CERTIFICATE`: base64-encoded Developer ID Application `.p12`
- `APPLE_CERTIFICATE_PASSWORD`: password used when exporting that `.p12`
- `APPLE_ID`: Apple account email used for notarization
- `APPLE_PASSWORD`: app-specific password for that Apple account
- `APPLE_TEAM_ID`: Apple Developer Team ID

Create the certificate secret with:

```sh
openssl base64 -A -in /path/to/developer-id-application.p12 -out certificate-base64.txt
```

The release runner imports the certificate into a temporary keychain, derives the Developer ID signing
identity, builds with Tauri notarization enabled, then verifies the app signature, Gatekeeper
assessment, and stapled app ticket before uploading the DMG. The temporary keychain password is generated on the runner and is not a
repository secret. Never commit Apple credentials to the repository.

## Storage and settings

Codex is discovered at `%USERPROFILE%\.codex` on Windows and `~/.codex` on macOS. A custom path can
be set in Settings. Application data uses the operating system application-data directory for the
identifier `dev.codexruntimewatch.app` and contains:

* `settings.json` — human-readable settings (`codex_home`, notifications, start-at-login preference,
  theme, and initial scan days).
* `runtime-watch.sqlite` — exactly two logical tables: factual `observations` and scanner
  `scan_state` cursors.

The default startup window is seven days. Existing files outside that window are not imported;
subsequent changes are event-driven. History is read in bounded pages and SQLite uses WAL with a
short busy timeout. Notification permission is controlled by Windows/macOS; denial is respected
without repeated background prompts and does not affect recording.

## Troubleshooting

* **No observations:** Confirm Codex is installed, run a normal Codex turn, and verify the Codex home
  path. If the Codex home or `sessions` directory does not exist yet, the watcher waits and retries
  periodically, then attaches automatically when Codex creates the directory; saving a corrected
  custom path also reconfigures the watcher immediately.
* **Provider says Not observed:** This normally means Codex did not persist explicit provider
  identity. It is not an error and is not evidence of a match.
* **Probe failed:** Run Codex once so its normal authentication flow can refresh credentials; if that
  does not help, run `codex login` and retry. Offline, expired-auth, unavailable-model, capacity, and
  changed-protocol errors remain classified, isolated Probe rows.
* **Watcher warning/no updates:** A transient file rename/delete race is ignored, but persistent file,
  fingerprint, or database scan failures surface as **Watcher warning** instead of being reported as
  healthy. Saving the corrected Codex home reconfigures the watcher.
* **Database temporarily busy:** The application retries SQLite locks briefly. Close other programs
  that hold the database and retry the operation.

## Architecture and compatibility

Data flows as: filesystem notification → incremental reader → Codex adapter → per-thread correlator
→ factual SQLite observation → Tauri IPC/event API UI refresh. Schema-specific code lives under
`src-tauri/src/codex/`; persistence and settings remain independent. The highest compatibility risks
are upstream rollout event names/paths, provider-notification shapes, and Responses SSE metadata.
The app does not inspect `logs_2.sqlite`. These adapters deliberately tolerate unknown fields and fail
closed for provider identity.

The implementation was researched against current OpenAI Codex source: rollout items include turn
context/configuration records, while provider/server model handling evolves independently and is not
guaranteed to be persisted for every turn. The conservative rule is permanent: only an explicit,
structured provider field populates Provider. Updating those adapter modules and fixtures should be
the normal response to upstream format changes.

## Known limitations

* Provider evidence is only as available as Codex's persisted structured events; live app-server
  notifications and raw network traces are not subscribed or enabled. Manual Verify is the explicit
  independent provider check.
* Manual Verify currently reads file-backed Codex ChatGPT credentials from `auth.json`; it does not
  independently run Codex's OAuth refresh flow or read credentials stored only in an OS keyring. If
  authentication is stale, run Codex normally to let it refresh, or sign in again, then retry Verify.
* Manual Verify depends on the current Codex web authentication/backend protocol. If explicit model
  evidence is absent, the probe is a protocol failure rather than guessed verification.
* Signing and notarization are release-operator responsibilities.

## License

[MIT](LICENSE)
