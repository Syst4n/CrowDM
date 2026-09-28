# Contributing to CrowDM

Thanks for helping. CrowDM should stay fast, dependable and easy to
understand; changes are judged on those three things.

## Getting set up

See "Building from source" in the README. Then read
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) and
[docs/DECISIONS.md](docs/DECISIONS.md) before changing the engine or the
browser hand-off.

## Where things live

- Transfer behaviour: `crates/engine/src/transfer.rs` (one download) and
  `engine.rs` (jobs, queue, persistence).
- Browser hand-off: `extensions/src/background.js`, `crates/host`,
  `apps/desktop/src-tauri/src/{pipe,captures}.rs`, protocol types in
  `crates/ipc`.
- UI: `apps/desktop/src/components`, styles and design tokens in
  `apps/desktop/src/styles.css`.

## Rules of thumb

- Every engine behaviour change gets a test in `crates/engine/tests` using
  the controllable test server (`crates/testserver`). Add a server option
  there if you need a new kind of misbehaving server.
- Changes to the hand-off must keep the guarantee that a browser download is
  only cancelled after CrowDM has taken it, and is resumed on every failure.
  Run `tools/e2e/app-capture.mjs` and `tools/e2e/browser-capture.mjs`.
- Do not claim speedups without measurements; put numbers and conditions in
  `docs/PERFORMANCE.md`.
- UI copy is plain English, short, and says what happens. The UI uses one
  accent colour (teal), 6 px radius for controls and 10 px for panels, and
  must work in light and dark themes.
- Run before pushing: `cargo fmt --all`, `cargo test --workspace`,
  `pnpm --dir apps/desktop typecheck`.

## Commits and pull requests

Small, focused commits with a descriptive message. Describe what a user sees
before and after, and how you tested it.
