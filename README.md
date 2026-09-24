# Codex Runtime Watch

Codex Runtime Watch is a lightweight local desktop utility that records the model and reasoning
effort Codex selects, runs, and—when explicitly exposed—reports from the provider. It watches normal
Codex activity without generating extra requests and presents a compact current-turn view and
searchable local history.

> If provider identity is not exposed by Codex, the application reports **Not observed** instead of
> guessing.

Version 0.1.0 supports **Windows 11 x64** and **macOS Apple Silicon**.

## Evidence model

| Label | Meaning | Accepted evidence |
|---|---|---|
| **Selected** | The model/effort selected for this turn. | `turn_context.payload.collaboration_mode.settings` first; nested `thread_settings_applied.payload.thread_settings` is the thread fallback. |
| **Runtime** | The local execution configuration recorded for a turn. | `turn_context.payload.model` and `turn_context.payload.effort` (including compatible field aliases). |
| **Provider** | A model explicitly named by server/provider evidence. | Structured `model/rerouted` or server-model events only. |

Runtime is strong evidence about local execution configuration, but it does **not** prove which model
a remote service ultimately served. The absence of a reroute event proves nothing about provider
identity. Runtime is therefore never copied into Provider.

Each rollout file is an isolated parsing scope. Its `session_meta.payload.id` supplies the canonical
thread identity when available; otherwise a deterministic, non-path file identity plus the turn ID
prevents files from mixing or duplicating. Settings are scoped per file/thread; subagent parent metadata is retained rather than attributed to its root agent. Unknown or malformed
events are skipped without stopping monitoring. Raw model and effort values are preserved, including
future values Codex Runtime Watch has never seen.

## What works

* Event-driven recursive watching under the Codex session directory, incremental JSONL reads, durable
  byte cursors plus per-rollout identity/settings context, partial-line recovery, truncation recovery,
  restart deduplication, and a bounded recent initial scan.
* Every useful normal turn is stored in SQLite, newest first, with All, Mismatches, Runtime, and
  Probes filters; records can be copied, deleted, or cleared.
* A compact vanilla TypeScript UI with system/light/dark themes and factual mismatch results.
* A Windows system tray/macOS menu-bar item that opens the app or Verify page, controls OS login
  startup, and quits explicitly. Closing the window hides it while monitoring continues.
* Native notifications for each newly scanned runtime mismatch or explicit provider reroute when
  enabled; probes remain in-app only.
* Manual **Verify Backend**, which sends exactly `hi` to the Codex Responses backend only after a
  click using the existing Codex login. It parses structured SSE `response.created.response.model`
  (or the explicit `OpenAI-Model` response header), records a separate Probe row, and classifies
  auth, network, capacity, and protocol failures without calling them mismatches.

Codex currently does not guarantee provider-model metadata in ordinary local rollouts or CLI JSON
output. Consequently, Provider will commonly remain **Not observed**. This is intended behavior.

## Privacy

There is no telemetry, analytics, tracking SDK, crash upload, remote database, or localhost server.
Normal monitoring makes zero additional OpenAI requests. It stores only model/configuration evidence and
identifiers; it does not store prompts, responses, conversation text, source code, project files, or
credentials. Manual Verify is the only feature that creates traffic. Authentication is read and
used only inside Rust for that request; tokens are never returned to the UI, logged, or persisted.

## Install and run

Download an artifact from a tagged release, or build on the target platform:

```sh
npm ci
npm run generate-icons
npm run tauri build -- --features desktop
```

Unsigned builds may trigger Windows SmartScreen. macOS distribution outside local development needs
an Apple Developer ID, hardened runtime signing, notarization, and stapling; CI intentionally does
not require those secrets. Windows production distribution similarly benefits from a trusted
code-signing certificate. macOS Intel and Linux packages are not produced.

For development:

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
short busy timeout.

## Troubleshooting

* **No observations:** Confirm Codex is installed, run a normal Codex turn, and verify the Codex home
  path. A missing `.codex` directory is harmless; saving a corrected path reconfigures the watcher immediately.
* **Provider says Not observed:** This normally means Codex did not persist explicit provider
  identity. It is not an error and is not evidence of a match.
* **Probe failed:** Run `codex login` and retry. Offline, expired-auth, unavailable-model, capacity,
  and changed-protocol errors remain classified, isolated Probe rows.
* **Watcher warning/no updates:** Save the corrected Codex home. Durable offsets resume complete
  lines and reset safely if a rollout is replaced or truncated.
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

* Provider evidence is only as available as Codex's persisted structured events; raw network traces
  are neither required nor enabled.
* Manual Verify depends on the current Codex web authentication/backend protocol. If explicit model
  evidence is absent, the probe is a protocol failure rather than guessed verification.
* Signing and notarization are release-operator responsibilities.

## License

[MIT](LICENSE)
