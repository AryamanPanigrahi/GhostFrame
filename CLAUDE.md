# CLAUDE.md — Riposte

## Project
Riposte is a 2-player online fighting game in Rust with rollback netcode. It's a Cargo workspace with three crates:
- `rewind/`: rollback netcode library (crate `rewind`), game-agnostic
- `game/`: the fighting game (crate `riposte-game`), built on macroquad
- `server/`: WebSocket relay (crate `riposte-server`), built on tokio + axum

**Read DESIGN.md before starting any task.** It holds the architecture, the frozen interface, the protocol, the milestones (R1–R6, S1–S2, G1–G7, I1–I3) and who owns what. **PROGRESS.md** says what's already built and what's still open.

## Ownership
Ownership is by milestone and file, not by crate.

**Aryaman**
- R3: `P2PSession` (`rewind/src/session/p2p.rs`, `rewind/src/snapshot.rs`)
- R4: fake-network transport + convergence tests (`rewind/src/transport/fake.rs` and its tests)
- G1, G2, G3, G4, G5, G7 (`game/`, including placeholder drawing and art integration in `game/src/render/mod.rs`, but not Ram's G6 files)
- Integration, WASM, deployment

**Ram**
- R1: crate skeleton, `api.rs` traits, `InputQueue` (`rewind/src/api.rs`, `rewind/src/input_queue.rs`, `rewind/src/lib.rs`)
- R2: `SyncTestSession` + `examples/boxes.rs`
- R5: checksum exchange, desync event, stall (builds on R3)
- R6: WebSocket transport (`rewind/src/transport/websocket.rs`)
- S1, S2: relay server (`server/`)
- G6: game feel, render-only (new files under `game/src/render/`, e.g. `render/effects.rs`, called from `render/mod.rs`)

Rules:
- Only edit files owned by the person running this session, unless the task explicitly says otherwise.
- Never change `rewind/src/api.rs` or the protocol enums unless the task says both people agreed. If a change seems necessary, stop and explain why instead.

## Learning rules (highest priority)
This project exists so both authors can understand and explain every line in interviews.
1. **Setup is never invisible.** The first time a new dependency, tool or setup step is introduced, explain what it is, why it's needed, and the exact command, and record it in SETUP.md so the human can run it by hand locally. Don't silently install, configure or start things the human hasn't seen yet. Building and running tests inside your own sandbox to verify your work is fine.
2. Every new concept gets an entry in LEARNING.md: name, first-principles explanation, and the file/function where it lives.
3. Prefer simple, explicit code over clever code. Avoid macros and heavy generics unless clearly needed. Comments explain *why*, not *what*.
4. Commit after each milestone with a message explaining what changed and why.
5. End every session with a summary: what was built, which files to read first and in what order, and open questions.
6. **Keep the docs in step with the code.** At the end of every milestone, in the same commit:
   - PROGRESS.md: status table, the milestone's gist, files, numbers, decisions, tests, open questions.
   - DESIGN.md: repository layout, "state as built", and the ✅ on the milestone.
   - SETUP.md: any new setup step, command or control.
   - LEARNING.md: new concepts.

   Anything a doc says that the code no longer does is a bug.

## Visuals: not Claude's job
- Don't design graphics, art, sprites, colour schemes or the visual style. The humans source third-party models/art and decide how the game looks.
- Focus on the game physics, simulation, netcode and software.
- Drawing code is placeholder only: plain shapes and debug text, just enough to see and test the mechanics. Keep it minimal and easy to replace.
- Help load or wire in third-party assets only when the task explicitly asks for it.
- Art never changes gameplay: hitboxes, hurtboxes and positions come from the sim, and art is drawn on top of them.

## Determinism rules (everything inside game/src/sim/ and anything rewind resimulates)
- No `f32`/`f64` in simulation state or logic. Use integer subpixels (1 px = 256 units, `i32`).
- No `HashMap`/`HashSet` iteration. Use `Vec` or arrays.
- No system time and no `thread_rng`. Randomness only from the seeded xorshift PRNG stored in `GameState`.
- The sim never reads window, render or frame-time data. Fixed 60 ticks per second.
- Floats are allowed only in `render/`, converting at the boundary.
- The game must pass rewind's `SyncTestSession` before any online work is merged.

## Code standards
- Rust stable, edition 2021. `cargo fmt` clean, `cargo clippy --workspace -- -D warnings` clean.
- Unit tests for all rewind logic and for sim mechanics (movement, hit detection, frame data).
- `thiserror` in library crates, `anyhow` in binaries.
- Only use dependencies listed in DESIGN.md section 9. Justify any addition in the session summary.

## Credit discipline
- One milestone per session. Don't start the next milestone unprompted.
- If a build or test fails 3 times in a row on the same issue, stop and explain the problem and options instead of retrying.
- Don't regenerate large files that already exist. Edit them.

## Commands
- Build all: `cargo build --workspace`
- Test all: `cargo test --workspace`
- Local 2-player: `cargo run -p riposte-game`
- Relay server: `cargo run -p riposte-server`
- Toy rewind example: `cargo run -p rewind --example boxes`
- WASM build: `./scripts/build-web.sh`, then serve `web/` (SETUP.md section 9)

## Handoff notes (read before starting a session)
Current as of Oct 31. Update this when it changes; detailed status lives in PROGRESS.md.

- **On `main`:** R1, R2, R5, R6, S1, S2 and G6 (Ram), plus workspace, G1-G5, G7, R3 and R4 (Aryaman), Practice vs lag (`game/src/practice.rs`), and **I1 online play** (`game/src/netplay.rs`: menu Create/Join → `Lobby` → `OnlineScreen` over `WebSocketTransport`; server address from `RIPOSTE_SERVER`, default `ws://127.0.0.1:8080/ws`).
- **R3 is done** (`rewind/src/session/p2p.rs`). **R5 is done** (merged to `main`): `tick` now stalls when `max_prediction` frames ahead (`Event::Stalled`), and exchanges confirmed-frame checksums every `Config::checksum_interval` frames (`Event::Desync`). `Config` has a new field, so build it with `..Config::default()`.
- **Aryaman's files edited by Ram's Claude, with Ram's go-ahead because Aryaman isn't pushing actively:** R5 edited `rewind/src/session/p2p.rs` (R3) and `rewind/tests/convergence.rs` (R4: the lag test now expects a stall, not a panic). Aryaman's Claude: pull `main` before touching those files, and expect them to differ from your last copy.
- **R6 is done** (merged to `main`): `rewind::WebSocketTransport` (`connect(url)`, `create_room`, `join_room`, `lobby_events`, plus `Transport`). Desktop uses tungstenite and the browser uses quad-net (see SETUP.md section 10). `web/mq_js_bundle.js` has a one-line fix so the quad-net plugin registers; keep it if you replace the file. Desktop speaks both `ws://` and `wss://` (tungstenite's rustls feature with the `ring` backend and built-in Mozilla roots; see SETUP.md section 10).
- **G5 is done:** Riposte passes the sync test (`game/src/sim/rollback.rs`). For I1 the game still needs R6 (WebSocket transport).
- **R4 is done:** `rewind/src/transport/fake.rs` and `rewind/tests/convergence.rs`. Sessions converge over bad networks within their envelope; R5's stall now handles lag beyond it (`lag_beyond_max_prediction_stalls_instead_of_desyncing`).
- **G6 touches two of Aryaman's files, with Aryaman's approval:** `render/mod.rs` (`draw` takes `&mut Effects`, `View::for_window` takes a shake offset, `flash_tint`, `pub mod effects;`) and `main.rs` (each `LocalMatch` owns an `Effects`). The effects code itself is in `game/src/render/effects.rs` and only reads `GameState`.
- **G6 is partly done:** no hitstop visuals (the sim has no hitstop), no health-bar animation (G4 is on `main` now, so this is unblocked), no frame-data tuning (`sim/attack.rs` is Aryaman's).
- **I1 touched Ram's files lightly:** `pub type DefaultSocket` in `rewind/src/transport/websocket.rs` (was private; the game needs to name it). `web/mq_js_bundle.js` now has three quad-net fixes: Ram's plugin registration, plus two from I1 (global `consume_js_object`/`js_object`, and binary messages pushed as raw bytes). Keep all three if the bundle is ever replaced. `.cargo/config.toml` adds `--allow-undefined` for the wasm target.
- **I2/I3 prep touched Ram's server files, additively:** new `server/src/web.rs` (serves 3 fixed files when `WEB_DIR` is set), `app_with_web` in `lib.rs` (`app()` is unchanged), `WEB_DIR` in `main.rs`, tokio's `fs` feature. The browser picks its relay address in `web/index.html` (`relayUrl`). Desktop `wss://` is done (`net/wss-desktop`). Open: choosing a host, the README, and an untested Dockerfile (no Docker daemon in the sandbox).
- **README, `docs/frame-walkthrough.md` and `docs/exercises.md` are written.** The README's public link is a `TODO` until a host is chosen and deployed. If code changes the path described in the walkthrough (tick order, `P2PSession::tick` steps, relay envelope), update it in the same commit.
