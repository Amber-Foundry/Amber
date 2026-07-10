# Amber Project Rules for Agents

Behavioral guidelines to reduce common LLM coding mistakes. Merge with project-specific instructions as needed.

## Architecture Summary
- This is a desktop application built with Tauri.
- Backend: Rust (`core/src/`), using an embedded SQLite database (`db/migrations/`).
- Frontend: React + TypeScript + Vite (`ui/`).

## Operational Commands
- To install frontend dependencies: `npm install` then `npm audit fix`
- To check formatting/linting: `npm run lint` or `cargo fmt --all`
- Running a dev server: `npm run tauri dev`
- Complete check of all tests: `npm run test`
- Always run preflight checks before finishing the task: `npm run preflight:fix`

# Rules for Code Generation

## 1. Separation of Concerns
Never invoke database logic directly from React components. All database access or local LLM context handling must go through a command handler in Rust (`core/src/`), which is then invoked via the strongly typed TS services in `ui/services/`. NEVER delete databases nor edit without asking explicit permission first.

## 2. Privacy Protocols 
Ensure all modifications handling personal user data respect the encryption structures in `core/src/privacy.rs` and `core/src/redacted.rs`.

## 3. PR Suggestions Protocol
When evaluating PR suggestions, always first check if the suggestions are valid. If they are valid, justify why; if they are not, explain why. This step must never be skipped.

## 4. Think Before Coding
**Don't assume. Don't hide confusion. Surface tradeoffs.**

Before implementing:
- State your assumptions explicitly. If uncertain, ask.
- If multiple interpretations exist, present them - don't pick silently.
- If a simpler approach exists, say so. Push back when warranted.
- If something is unclear, stop. Name what's confusing. Ask.

## 5. Simplicity First

**Minimum code that solves the problem. Nothing speculative.**

- No features beyond what was asked.
- No abstractions for single-use code.
- No "flexibility" or "configurability" that wasn't requested.
- No error handling for impossible scenarios.
- If you write 200 lines and it could be 50, rewrite it.

Ask yourself: "Would a senior engineer say this is overcomplicated?" If yes, simplify.

## 6. Surgical Changes
**Touch only what you must. Clean up only your own mess.**

When editing existing code:
- Don't "improve" adjacent code, comments, or formatting.
- Don't refactor things that aren't broken.
- Match existing style, even if you'd do it differently.
- If you notice unrelated dead code, mention it - don't delete it.

When your changes create orphans:
- Remove imports/variables/functions that YOUR changes made unused.
- Don't remove pre-existing dead code unless asked.

The test: Every changed line should trace directly to the user's request.

## 7. Goal-Driven Execution

**Define success criteria. Loop until verified.**

Transform tasks into verifiable goals:
- "Add validation" → "Write tests for invalid inputs, then make them pass"
- "Fix the bug" → "Write a test that reproduces it, then make it pass"
- "Refactor X" → "Ensure tests pass before and after"

For multi-step tasks, state a brief plan:
```
1. [Step] → verify: [check]
2. [Step] → verify: [check]
3. [Step] → verify: [check]
```

Strong success criteria let you loop independently. Weak criteria ("make it work") require constant clarification.

## Cursor Cloud specific instructions

Dependency refresh (`npm ci`, `cargo fetch`) runs automatically on VM startup. System deps (WebKitGTK + Tauri libs), Node, and the Rust toolchain are already baked into the VM image. Notes below are for running/testing, not initial setup.

### Running the desktop app (headless VM)
- Amber is a Tauri desktop app; there is no separate backend server. SQLite is embedded in-process and `db/migrations/` are applied automatically at startup.
- Start it with `npm run tauri dev` (it auto-starts the Vite dev server on port `1420`, then builds/launches the native window). The first run recompiles the Rust core (`core/`) and can take several minutes; subsequent runs are fast.
- A display is available at `DISPLAY=:1`. Export it in the shell before launching (`export DISPLAY=:1`) since the window needs an X server. `libEGL`/`DRI3` warnings in the log are harmless software-rendering fallback, not errors.

### First run / app data
- On first launch the app shows a first-run onboarding wizard. Model/LLM setup steps can be skipped ("I'll set this up later" / "Skip onboarding") to reach the main workspace; local-LLM/embedding features stay optional (they need Ollama, a cloud API key, or downloaded ONNX models).
- Persistent app data (SQLite DB, downloaded models) lives under `~/.amber/`. Delete `~/.amber/` to reset back to the onboarding/first-run state.

### Toolchain caveats
- The Rust core needs Rust ≥ 1.85 (a transitive dep uses edition2024); the image is on current stable.
- `cc` resolves to gcc-14, so `libstdc++-14-dev` must be present for the Rust link step to find `-lstdc++` (already installed in the image).

### Commands
- Standard lint/test/build/run commands are in `README.md` and `package.json` scripts (`npm run lint`, `npm test`, `npm run test:ui`, `npm run build`, `npm run tauri dev`). Rust checks: `cargo test` / `cargo clippy` / `cargo fmt` in `core/`. `npm run preflight` runs the full CI-parity gate (includes cargo fmt/clippy/test).