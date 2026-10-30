# CLAUDE.md — Riposte

## Project
Riposte is a 2-player online fighting game in Rust with rollback netcode. It's a Cargo workspace with three crates:
- `rewind/`: rollback netcode library (crate `rewind`), game-agnostic
- `game/`: the fighting game (crate `riposte-game`), built on macroquad
- `server/`: WebSocket relay (crate `riposte-server`), built on tokio + axum

**Read DESIGN.md before starting any task.** It holds the architecture, the frozen interface, the protocol, the milestones (R1–R6, S1–S2, G1–G7, I1–I3) and who owns what.

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
- WASM build: see SETUP.md (added in milestone G7)
