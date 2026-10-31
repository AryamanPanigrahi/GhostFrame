# PROGRESS.md

What's built so far, milestone by milestone: the gist, the files, the tests and the decisions made along the way. Updated at the end of every milestone. For *why* each concept works the way it does, see LEARNING.md; for the plan, see DESIGN.md.

---

## Status

| Milestone | Owner | Status |
|---|---|---|
| Workspace setup | Aryaman | ✅ Done |
| G1: window, 60 Hz loop, input bitmask, LocalSession | Aryaman | ✅ Done |
| G2: fixed-point movement | Aryaman | ✅ Done |
| G3: attacks, frame data, hitboxes, damage, hitstun, knockback | Aryaman | ✅ Done |
| G4: rounds, health bars, KO, reset, PRNG + FNV checksum | Aryaman | ✅ Done |
| G5: `RollbackGame` + `SyncTestSession` | Aryaman | Not started (unblocked: R2 is on `main`) |
| G6: game feel (render-only) | Ram | Partly done: flash + shake (no hitstop visuals, health-bar animation or frame-data tuning yet, see below) |
| G7: menu, WASM build | Aryaman | ✅ Done (online items wait for I1) |
| R1: crate skeleton, `api.rs`, `InputQueue` | Ram | ✅ Done |
| R2: `SyncTestSession` + `boxes` example | Ram | ✅ Done |
| R3: `P2PSession` | Aryaman | Not started (unblocked: R1 is on `main`) |
| R4: fake-network transport + convergence tests | Aryaman | Not started |
| R5: checksums, desync, stall | Ram | Not started (needs R3) |
| R6: WebSocket transport | Ram | Not started |
| S1, S2: relay server | Ram | ✅ Done |
| I1–I3: online play, browser, deploy | Together | Not started |

**Tests:** 118 tests, all passing (`cargo test --workspace`): 22 in `rewind`, 79 in `riposte-game`, 17 in `riposte-server` (6 of them over real WebSockets). fmt and clippy clean.

---

## How to try what exists

Desktop:

```bash
cargo run -p riposte-game
```

Browser (one-time setup and details in SETUP.md section 9):

```bash
./scripts/build-web.sh
```

```bash
python3 -m http.server 8000 --directory web
```

Then open http://localhost:8000. Either way, the game starts at the main menu: Up/Down and Enter, then "Local 2-player". Esc goes back to the menu.

| | Move | Jump | Crouch | Light | Heavy |
|---|---|---|---|---|---|
| P1 | A / D | W | S | F | G |
| P2 | ← / → | ↑ | ↓ | K | L |

The screen is a placeholder debug view. Real visuals will come from third-party art.
- Grey boxes are the fighters' hurtboxes, and a black notch shows facing.
- A red outline is an active hitbox.
- Each fighter's label shows its current action and health.
- Along the top are health bars, round wins and the round number, with a banner for READY, K.O. and the match winner.
- The bottom line shows the frame number and the state checksum.

---

## Workspace setup

**Gist:** One Cargo workspace so every crate shares one `Cargo.lock` and one `target/`. Only `game` exists so far; Ram adds `rewind` (R1) and `server` (S1) to `members` in the root `Cargo.toml`.

**Files:** `Cargo.toml`, `game/Cargo.toml`, `.gitignore`, `SETUP.md`.

**Decisions:**
- macroquad 0.4.16 with default features off, so no audio, and Linux doesn't need ALSA.
- `Cargo.lock` is committed so both people build identical versions.

---

## G1: window, fixed 60 Hz loop, input bitmask, LocalSession

**Gist:** The game opens a 1280×720 window. Real time is banked and spent in exact 1/60 s ticks, so the sim runs at 60 ticks/s whatever the monitor's refresh rate. The smoke test drew ~210 fps while ticking at 60. Each player's keys become a `u16` bitmask, and `LocalSession` feeds both into the sim, shaped like rewind's future `P2PSession`.

**Files:** `game/src/main.rs` (loop), `game/src/clock.rs` (`FixedClock`), `game/src/input.rs` (keys → bits), `game/src/session.rs` (`LocalSession`), `game/src/sim/mod.rs` (`Input`, button bits, `GameState`), `game/src/render/mod.rs` (placeholder view, 16:9 letterboxing).

**Decisions:**
- Time is counted in microseconds × 60, so one tick is exactly 1,000,000 units: no rounding drift and no floats.
- At most 4 ticks per frame; after a long pause the extra time is dropped, so the game doesn't fast-forward.
- The `f64` → microseconds conversion happens once, in `main.rs`, outside the sim.

**Tests:** clock (60 ticks/s at 30–240 fps, banking, cap), key mapping, button bits, session routing.

---

## G2: fixed-point movement

**Gist:** Each fighter walks, crouches and jumps, with gravity, a floor, walls at both stage edges, and facing toward the opponent. Everything is integer subpixels (1 px = 256).

**Files:** `game/src/sim/fighter.rs` (`Fighter`, movement constants, `step`, `ground_control`, `face`).

**Numbers:**
- Walk: 4 px/tick forward, 3 px/tick back.
- Jump: −15 px/tick at takeoff, gravity 0.75 px/tick². That gives an exact 41-tick jump peaking at 157.5 px.
- Fighter: 80 px wide, 200 px tall standing, 120 px crouching.
- Floor: 640 px. Stage: 1280 px wide.

**Decisions:**
- y grows downward; position is the centre of the feet.
- No air control: the jump direction is fixed at takeoff.
- Left + right (or up + down) together count as neutral (SOCD cleaning).
- Facing only updates while actionable on the ground, using both positions from the start of the tick, so update order can't favour P1.

**Tests:** walk speeds, stop on release, SOCD, no gravity on the floor, exact jump arc, no air control, crouch, both walls, facing rules, crossing over.

---

## G3: attacks, frame data, hitboxes, damage, hitstun, knockback

**Gist:**
- Two attacks with real frame data. An attack starts on a fresh press, roots the fighter, and hits if its hitbox overlaps the opponent's hurtbox during its active frames.
- A hit deals damage, puts the defender in hitstun and knocks them back.
- Both hits are checked before either is applied, so simultaneous hits trade.

**Files:** `game/src/sim/attack.rs` (`Attack`, `LIGHT_ATTACK`, `HEAVY_ATTACK`), `game/src/sim/rect.rs` (`Rect::overlaps`), `game/src/sim/fighter.rs` (state machine: `tick_action`, `start_attack`, `take_hit`, `hurtbox`, `active_hitbox`), `game/src/sim/mod.rs` (`check_hit`, hit resolution in `advance`).

**Frame data:**

| | Startup | Active | Recovery | Total | Damage | Hitstun | Knockback | Hits | On hit |
|---|---|---|---|---|---|---|---|---|---|
| Light | 4 | 3 | 8 | 15 | 50 | 15 | 3 px/tick | high (whiffs on crouch) | +4 |
| Heavy | 10 | 4 | 20 | 34 | 120 | 20 | 6 px/tick | mid (hits crouch) | −4 |

Health is 1000. Knockback slows by 0.25 px/tick each tick of hitstun.

**Decisions:**
- One hit per swing (`attack_connected`).
- A hit interrupts anything, including the defender's own attack, so the faster attack wins.
- "Airborne" is decided by position, so a fighter hit mid-jump keeps falling.
- Facing is frozen during attacks and hitstun, so a hitbox can't swap sides mid-swing.

**Tests:** AABB overlap, active frames, mirroring, press-not-hold, attack duration, hitstun countdown, knockback, air hit, health floor, first-active-frame hit, one hit per swing, whiff, crouch vs light/heavy, trade, interrupt, exact frame advantage (+4 / −4).

**Not built yet:** blocking, air attacks, pushboxes (fighters can walk through each other).

---

## G4: rounds, health bars, KO, reset, PRNG + FNV checksum

**Gist:**
- A match is best of 3. Each round opens with a 1 s READY pause (input ignored), then the fight.
- At 0 health the loser is knocked down and the winner scores. After 2 s the next round starts with fighters reset and the score kept.
- At 2 wins the match winner shows for 4 s, then a fresh match starts. A double KO is a draw and the round is replayed.
- The state now holds a seeded xorshift64 generator and has an FNV-1a checksum over every field, ready for rewind's sync test (G5) and desync detection.

**Files:** `game/src/sim/round.rs` (`Phase`, `Round`, timings), `game/src/sim/rng.rs` (`Rng`), `game/src/sim/checksum.rs` (`Fnv1a`), `game/src/sim/mod.rs` (`GameState::new(seed)`, input gating, `check_ko`, `tick_round`, `checksum`), `game/src/sim/fighter.rs` (`Action::Knockdown`, `knock_down`, `hash_into`), `game/src/render/mod.rs` (placeholder health bars, score, banners, checksum), `game/src/main.rs` (`LOCAL_SEED`).

**Numbers:**
- Pauses: READY 60 ticks (1 s), round over 120 (2 s), match over 240 (4 s).
- 2 wins take the match. Health is 1000.

**Decisions:**
- Hits are only checked during `Fight`, so an attack still active after a KO can't hit the downed fighter.
- Physics keeps running in pauses, so a fighter KO'd in the air still lands.
- A new round resets fighters but keeps the frame counter, the random generator and the score.
- The checksum names every field through a destructuring `let`, so forgetting a new field won't compile. Enums use explicit tags and numbers are little-endian, so hashes are identical on every platform.
- Nothing uses the random generator yet. It's in the state now so checksums and rollback already cover it.

**Tests:** xorshift (same seed same sequence, zero seed, known value), FNV-1a (official test vectors, byte order), ready pause, KO and scoring, input ignored after KO, round reset, pause lengths, match win and new match, double KO, knockdown, checksum equality, and that changing any field or swapping the players changes the checksum.

**Not built yet:** round timer (time-out), blocking, pushboxes, air attacks.

---

## G7: main menu, WASM build

**Gist:**
- The game opens on a main menu: Local 2-player, Create online room, Join online room. Joining asks for a 4-letter room code.
- Local play works fully, and Esc returns to the menu. The two online items show an honest "not connected yet" notice, because the relay server (S1), the WebSocket transport (R6) and the online wiring (I1) don't exist yet. The menu already produces `CreateRoom` / `JoinRoom { code }` for I1 to hook up.
- The same game builds for the browser: `scripts/build-web.sh` produces `web/riposte-game.wasm`, served next to `index.html` and macroquad's JS loader.

**Files:**
- `game/src/menu.rs` (`Menu`, `Screen`, `MenuKey`, `MenuAction`).
- `game/src/input.rs` (`read_menu_keys`, `back_pressed`, `held_or_tapped`).
- `game/src/main.rs` (`App`, `LocalMatch`, input buffering).
- `game/src/render/mod.rs` (`draw_menu`, placeholder).
- `web/index.html`, `web/mq_js_bundle.js`, `scripts/build-web.sh`.

**Decisions:**
- Room codes are 4 letters, case-insensitive (stored uppercase).
- The JS loader is copied from macroquad 0.4.16 itself, so it always matches the Rust side; the `.wasm` is built, not committed.
- Quick taps are never lost:
  - A key counts if it was down *or* pressed during the frame.
  - Input is buffered across frames until a tick consumes it.
  - Both came from testing the browser build, where a fast tap was dropped.

**Verified in a real browser:** headless Chromium in the sandbox loaded the WASM build. It went through the menu, typed a room code ("ab9cd" became "ABCD"), saw the online notice, and started a local match. Held keys walked both fighters, an instant tap started an attack, and Esc returned to the menu.

**Tests:** menu navigation and wrap-around, local/create/join actions, code entry (letters only, uppercase, max 4, backspace, no short codes), back from code entry and notices, menu key mapping.

**Found while testing:** macroquad's `mq_js_bundle.js` throws `register_plugin is not defined` on load. Its bundled quad-net WebSocket plugin assigns an undeclared variable in strict mode. It's harmless now, but quad-net is what R6 uses for WebSockets in the browser, so this is likely the first thing R6's WASM spike hits (see open questions).

---

## R1: rewind crate skeleton, `api.rs`, `InputQueue` (Ram)

**Gist:**
- The `rewind` crate exists and `api.rs` holds `RollbackGame` and `Transport` exactly as in DESIGN.md section 4, plus two shared definitions: `type Frame = u32` and the `Event` enum (Desync, Stalled, Disconnected).
- `InputQueue` stores one player's inputs in frame order. It predicts unknown frames as the last confirmed input, and when the real input arrives it records the earliest wrong frame, which is where a session rolls back to.

**Files:** `rewind/src/api.rs`, `rewind/src/input_queue.rs`, `rewind/src/lib.rs`, `rewind/Cargo.toml`.

**Decisions:**
- Inputs must arrive in order: gaps are rejected, and repeats from redundant resends are ignored.
- `reset_prediction` is for the session to call on rollback.
- `discard_before` keeps memory bounded.
- `serde` is the only dependency.

**Tests:** 15 unit tests.

**Open for R3 (from Ram):** `input_delay` isn't handled by `InputQueue`. The session should fill the local queue's first `input_delay` frames with default inputs.

---

## R2: `SyncTestSession` and the boxes toy game (Ram)

**Gist:**
- Every tick, `SyncTestSession` advances one frame and records the state before it, the inputs and the checksum after it.
- It then loads the state from `check_distance` frames back, replays them, and fails with a `SyncTestError` (frame, expected, actual) on the first mismatching checksum.
- `examples/boxes.rs` is a tiny deterministic game that passes it, rolling back 8 frames every tick for 600 frames. `-- --broken` shows a failure.

**Files:** `rewind/src/session/synctest.rs`, `rewind/src/session/mod.rs`, `rewind/examples/boxes.rs`.

**Decisions:**
- It returns a `Result` instead of `Event::Desync`: a failing sync test is a bug to stop on, not a runtime event.
- It keeps only the last `check_distance` records, so memory stays constant.

**Tests:** 7 unit tests, including a game whose `save()` forgets a field, caught at frame 0.

```bash
cargo run -p rewind --example boxes
```

---

## S1, S2: relay server (Ram)

**Gist:**
- **Pairing:** `riposte-server` serves a WebSocket route `/ws` that pairs two players by a 4-letter room code. It sends both `MatchStart` (player index 0/1 and one shared random seed), then forwards `Relay` bytes unchanged.
- **Leaving:** a disconnect sends `PeerLeft` and deletes the room, and rooms idle for 5 minutes are swept.
- **Bad messages close the connection:** an unknown or full room, undecodable bytes, messages over 16 KiB, or creating a room twice. The frozen `ServerMsg` has no error variant to report them with.
- **Running it:** logs via `tracing` (`RUST_LOG`), port from `PORT`, graceful shutdown, and a two-stage Dockerfile.

**Files:** `server/src/rooms.rs` (room logic, no sockets), `server/src/lib.rs` (axum handler, one task per connection), `server/src/protocol.rs`, `server/src/main.rs`, `server/tests/relay.rs`, `Dockerfile`, `.dockerignore`.

**Dependencies:** `tokio`, `axum` (ws), `serde` + `bincode`, `tracing` + `tracing-subscriber`, `anyhow`. Plus the dev-only `tokio-tungstenite` and `futures-util` for the real-socket tests, justified in SETUP.md section 6.

**Tests:** 11 unit tests on the room logic, plus 6 integration tests that drive a real server over real WebSockets.

```bash
cargo run -p riposte-server
```

---

## G6: game feel (render-only)

**Gist:** A hit now flashes the defender red and shakes the view, harder and longer for bigger hits (light: 5 px for 3 ticks; heavy: 12 px for 8 ticks). `Effects` only reads `GameState`; it never writes to it.

**Files:** `game/src/render/effects.rs` (new), plus small wiring in `game/src/render/mod.rs` (`draw` takes `&mut Effects`, the view takes a shake offset, `flash_tint`) and `game/src/main.rs` (each `LocalMatch` owns an `Effects`, so a new match starts without a leftover shake). The `mod.rs` and `main.rs` edits touch Aryaman's files, as DESIGN.md says G6 is "called from render/mod.rs".

**Decisions:**
- Hits are detected by a health drop between two draws, so the sim needed no changes and no hit event.
- Timers count sim ticks (frame delta), so they don't depend on frame rate.
- Frame going backwards or health rising resets the baseline without firing effects.

**Tests:** 9 in `effects.rs`: no hit, defender-only flash, fade in ticks, redraw doesn't fade, heavy > light, shake jitter and end, small hit doesn't cut a big shake, restart not mistaken for damage, health rising.

**Not done:**
- **Hitstop visuals:** the sim has no hitstop, so freezing the picture in render would show something the sim doesn't do. If wanted, hitstop should be added to the sim first (Aryaman's files).
- **Health-bar animation:** unblocked now that G4's health bars are on `main`.
- **Frame-data tuning:** needs edits to `sim/attack.rs`, which is Aryaman's. Needs a playtest and agreement first.

---

## Open questions

- **Blocking** isn't in any milestone, so G4 doesn't have it. Suggestion: hold back to block, as a small step before G5.
- **Round timer:** there's no time-out yet, so a round only ends by KO. Should rounds have one (e.g. 99 s, most health wins)?
- **Pushboxes** (body collision): fighters can currently overlap. A small step of its own?
- **Air attacks:** pressing an attack mid-jump does nothing today.
- **Tuning:** all movement and frame-data numbers are first guesses. Ram's G6 includes frame-data tuning.
- **Art:** 2D sprites or 3D models? This affects G7 and the WASM build.
- **Main loop floats:** `main.rs` converts macroquad's `f64` time to integers once. The "floats only in `render/`" rule is read as applying to the sim; say if it should be stricter.
- **R3:** unblocked now that R1 is on `main`. Ram's note: `InputQueue` doesn't handle `input_delay`, so `P2PSession` should fill the first `input_delay` local frames with default inputs.
- **quad-net in the browser (for Ram, R6):** the JS bundle's quad-net plugin fails to register (`register_plugin is not defined`). R6's WASM spike should start by checking how quad-net expects its JS to be loaded with this macroquad version. Native WebSockets aren't affected.
