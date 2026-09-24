# Codex Runtime Watch contributor guide

The product scope is intentionally narrow: observe model evidence, not usage. Do not add token,
cost, quota, account, prompt, or project dashboards unless explicitly requested. Never perform an
automatic backend probe. Runtime is local execution evidence and **must never** be equated with
Provider; absent explicit server evidence, Provider remains `Not observed`.

Keep Codex schema knowledge inside `src-tauri/src/codex/`. Model and reasoning-effort strings are
open-ended upstream values. The intended maintenance pattern is: **Update Codex adapters when
upstream interfaces change.** Preserve privacy: never collect prompts, responses, source, project
files, credentials, or telemetry, and never log secrets.

Use vanilla TypeScript for this small UI. Keep SQLite to factual records in the two existing tables;
do not persist redundant derived status. Run formatting, clippy, Rust tests, TypeScript typechecking,
and the frontend build before committing. Supported release targets are Windows 11 x64 and macOS
Apple Silicon only; macOS Intel and Linux packaging are not requirements.

Normal monitoring is passive. The manual probe is the sole exception: only an explicit user action
may send the literal `hi`, and authentication must remain inside Rust with no token logging or UI exposure.
