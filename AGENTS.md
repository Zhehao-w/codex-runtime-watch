# Codex Runtime Watch contributor guide

The product scope is intentionally narrow: observe model evidence, not usage. Do not add token,
cost, quota, account, prompt, or project dashboards unless explicitly requested. Never perform an automatic backend probe except for the separately opt-in macOS Window Scheduler; that feature must stay disabled by default, fail closed on missing account/usage data, never use API-key billing, and never alter normal passive monitoring. Runtime is local execution evidence and **must never** be equated with
Provider; absent explicit server evidence, Provider remains `Not observed`.

Keep Codex schema knowledge inside `src-tauri/src/codex/`. Model and reasoning-effort strings are
open-ended upstream values. The intended maintenance pattern is: **Update Codex adapters when
upstream interfaces change.** Preserve privacy: never collect prompts, responses, source, project
files, credentials, or telemetry, and never log secrets.

Use vanilla TypeScript for this small UI. Keep SQLite to factual records in the two existing tables;
do not persist redundant derived status. Run formatting, clippy, Rust tests, TypeScript typechecking,
and the frontend build before committing. Supported release targets are Windows 11 x64 and macOS
Apple Silicon only; macOS Intel and Linux packaging are not requirements.

Normal monitoring is passive. Manual Verify requires an explicit click. The opt-in
macOS Window Scheduler is the only separately authorized exception, and may send a
minimal CLI request only after validating the ChatGPT account, real limits, and
weekly safety threshold. Never automatically spend banked resets or API key credit.
Authentication must remain in Rust; never log or expose tokens.

The optional macOS Wake Manager is privileged **only** for its own fixed
fallback and bounded server-reset pmset wake events. Its administrator-approved root script must remain root-owned, static, and
incapable of evaluating user-supplied code or reading Codex credentials. Never
overwrite other pmset schedules. Install and uninstall only after an explicit user
action; no global sudoers rules or permanent sleep-setting modifications.
