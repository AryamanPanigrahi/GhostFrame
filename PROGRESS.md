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
| G5: `RollbackGame` + `SyncTestSession` | Aryaman | ✅ Done: passes the sync test |
| G6: game feel (render-only) | Ram | Partly done: flash + shake (no hitstop visuals, health-bar animation or frame-data tuning yet, see below) |
| G7: menu, WASM build | Aryaman | ✅ Done (online items wait for I1) |
| R1: crate skeleton, `api.rs`, `InputQueue` | Ram | ✅ Done |
| R2: `SyncTestSession` + `boxes` example | Ram | ✅ Done |
| R3: `P2PSession` | Aryaman | ✅ Done |
| R4: fake-network transport + convergence tests | Aryaman | ✅ Done |
| R5: checksums, desync, stall | Ram | ✅ Done |
| R6: WebSocket transport | Ram | ✅ Done (browser end-to-end waits for I2) |
| S1, S2: relay server | Ram | ✅ Done |
| I1 online play | Together | ◐ Code done, browser vs browser verified; the two-laptop test is yours |
| I2 online in the browser | Aryaman | ✅ Done in the sandbox: the page picks its own relay address; two browsers played via the relay serving the page |
| I3 deploy + public link | Together | ◐ Prep done (relay serves the web build; Dockerfile includes it). Untested Dockerfile, no host chosen, no README yet |

**Tests:** 185 tests, all passing (`cargo test --workspace`): 63 in `rewind`, 98 in `riposte-game`, 24 in `riposte-server` (including 4 for serving the web build). fmt and clippy clean.

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

## I2 + I3 prep: browser picks its relay, relay hosts the page (Aryaman)

**Gist:**
- **I2:** the page decides the relay address (`relayUrl` in `web/index.html`): `?server=` override, else `ws://<host>:8080/ws` for a page on another dev port, else the page's own address (`wss://` on HTTPS). The game gets it through two functions the page adds to the module (`server_url()` for wasm in `netplay.rs`).
- **I3 prep:** with `WEB_DIR` set, the relay also serves `index.html`, `mq_js_bundle.js` and `riposte-game.wasm`, so one deployed app is the whole game. The `Dockerfile` builds the web version into the image and sets `WEB_DIR`.

**Verified in the sandbox:**
- Relay with `WEB_DIR=web` on :8080; two headless browsers opened it with **no configuration**, created and joined a room, and fought (760 / 900 hp, 89 frames rolled back, 48 stalled ticks, no desync).
- The four address cases (HTTPS page, relay page, dev port, `?server=`) all return the right URL.
- 4 server tests: the right content types, only the 3 fixed files (path traversal gets 404), a missing file is a 404, and without `WEB_DIR` `/` is still the health check.

**Not verified:** the `Dockerfile` (no Docker daemon in the sandbox), and anything over a real HTTPS host.

**Files:** `server/src/web.rs` (new), `app_with_web` in `server/src/lib.rs`, `WEB_DIR` handling in `server/src/main.rs`, `tokio`'s `fs` feature in `server/Cargo.toml`, `server/tests/web.rs`, `web/index.html`, `game/src/netplay.rs`, `Dockerfile`, `.dockerignore`. The server files are Ram's; the change is small and additive, and `app()` behaves exactly as before.

**Still open for I3:**
- **Desktop `wss://`:** `rewind` links tungstenite without TLS, so desktop can only reach `ws://` servers. Adding it means a TLS feature on tungstenite in `rewind/Cargo.toml` (Ram's R6 file) plus a justified new dependency. Browser players don't need it.
- Choose a host, deploy, and replace the `TODO` public link in `README.md`.
- Free hosts usually sleep idle apps, so the first connection can be slow, and rooms don't survive a restart.

---

## I1: online play through the relay (Aryaman)

**Gist:**
- **Create online room** connects to the relay and shows the 4-letter code. **Join online room** connects and joins.
- On `MatchStart` the game starts an `OnlineMatch` with the player index and seed.
- During the match an overlay shows your player number, frames rolled back and ticks stalled, plus a loud DESYNC warning if R5 ever reports one.
- If the other player leaves, the match stops with a message.
- The server address defaults to `ws://127.0.0.1:8080/ws`. On desktop, `RIPOSTE_SERVER` overrides it.

**Verified in the sandbox:** the real relay server plus two separate headless Chromium browsers. One created a room, the other joined with the code from the server log, and both fought for a few seconds.
- Both showed the same match: same health (640 / 950) and positions.
- Rollbacks happened (41 and 63 frames, deepest 7) and stalls kicked in (the software renderer ran at ~17 fps).
- There was no desync. Closing one browser showed "The other player left" in the other.
- **Not verified here:** native vs native through the menu. Ram's `server/tests/rewind_client.rs` covers the native transport plus sessions over real sockets, but the two-laptop test (I1's definition) is for you and Ram.

**Found and fixed on the way (browser build):**
1. The WASM link failed with "undefined symbol: ws_connect". The fix is `.cargo/config.toml` with `--allow-undefined` for the wasm target. It only appeared once the game actually called the WebSocket code.
2. `consume_js_object is not defined`: the minified bundle hid the `sapp_jsutils` helpers that quad-net calls by their global names. One-line fix in `web/mq_js_bundle.js`.
3. Nothing was ever received: quad-net 0.1.2's Rust side reads binary messages from the wrong JavaScript object, so they arrived empty. One-line fix in `web/mq_js_bundle.js`.

**Files:**
- `game/src/netplay.rs` (`Lobby`, `OnlineScreen`, `server_url`).
- `App::Lobby` / `App::Online` in `game/src/main.rs`, and `draw_text_screen` in `render/mod.rs`.
- `OnlineMatch::transport_mut` / `stalled_ticks`.
- `P2PSession::transport_mut` (R3 file).
- `pub type DefaultSocket` in Ram's `rewind/src/transport/websocket.rs`: a one-word visibility change so the game can name the platform's socket type.
- `.cargo/config.toml` and `web/mq_js_bundle.js`.
- `serde` + `bincode` as test-only dev-dependencies of the game.

**Tests:** 6 new (fake socket playing the relay's side):
- create shows the code, then starts as P1;
- join starts as P2;
- join with no answer times out after 5 s;
- the opponent leaving before the start fails the lobby;
- the opponent leaving mid-match stops it;
- peer inputs arriving through the relay are used without guesses.

**Still open for I1/I2:**
- The two-laptop test.
- Errors don't say *why* a native connection failed beyond the error text.
- ~~In the browser the server address is fixed~~ Fixed by I2 below; it was:
- "WebSocket is already closing" console noise after the other player leaves (harmless).

---

## I1 prep: online match plumbing and "Practice vs lag" (Aryaman)

**Gist:**
- `OnlineMatch` is the game plus a `P2PSession` over any transport. When R6 lands, I1 is: connect, wait for `MatchStart`, then create an `OnlineMatch` with that player number and seed over the WebSocket.
- A new menu mode, **Practice vs lag**, plays you against the bot through the real netcode. The bot runs its own session and game copy, joined to yours by R4's fake network, and keys 1-4 change the lag mid-match. An overlay shows frames rolled back.
- `TickDriver` now drives every match: the fixed 60 Hz clock plus input buffering, previously copied inside `LocalMatch`.

**Files:**
- `game/src/online.rs` (`OnlineMatch`), `game/src/practice.rs` (`Practice`, `LAG_PRESETS`), `game/src/driver.rs` (`TickDriver`).
- `game/src/bot.rs` (`Bot`, moved out of the G5 tests).
- Menu item and `MenuAction::StartPractice` in `menu.rs`; `read_either_player` and `lag_preset_pressed` in `input.rs`.
- Placeholder `draw_overlay` in `render/mod.rs`; `App::Practice` in `main.rs`.
- `FakeNetwork::set_config` in `rewind/src/transport/fake.rs`.

**Lag presets (one-way):** none; 33 ms + 17 ms jitter; 67 ms + jitter, 5% loss; 100 ms + jitter, 10% loss. All in-order, like a WebSocket, and all inside the prediction envelope.

**Verified in a browser:** headless Chromium ran practice mode on the worst preset. Both fighters landed hits, it rolled back 227 frames (deepest 6), and nothing broke.

**Tests:** 9 new:
- tick driver (2), bot (2), menu practice item (1);
- two online matches identical every tick without lag (1);
- practice: worst preset rolls back without crashing, no lag means both sides agree every tick, switching lag mid-match is safe (3).

**What I1 still needed:** done in the I1 section above.

(A separate relay client was written in `game/src/relay.rs` before R6 landed, then dropped unmerged: R6's `WebSocketTransport` already does the lobby and the relay wrapping, and Ram's `server/tests/rewind_client.rs` tests the same path over real sockets.)

---

## R4: fake network and convergence tests (Aryaman)

**Gist:**
- `FakeNetwork` is a deterministic, tick-driven fake network. Each direction has latency, jitter, packet loss (capped by a maximum losing streak) and in-order (TCP-like) or reordering (UDP-like) delivery.
- 200 seeded runs of two `P2PSession`s over random bad networks all converge: both peers end identical to each other and to a run where every input was known.
- Riposte itself converges too, bot vs bot over a lossy, jittery network.

**Files:**
- `rewind/src/transport/fake.rs` (`FakeConfig`, `FakeNetwork`, `FakeTransport`, `LinkStats`).
- `rewind/tests/convergence.rs`.
- `riposte_converges_over_a_bad_network` in `game/src/sim/rollback.rs`.
- `P2PSession::deepest_rollback()` added in `rewind/src/session/p2p.rs`.
- Re-export lines in Ram's `rewind/src/lib.rs`.

**Numbers (200-seed run):**
- Networks: latency 0–3 ticks, jitter 0–2, loss 0–30% with at most 2 in a row, half in-order and half reordering.
- About 30,000 frames re-simulated, over 20,000 messages dropped, deepest rollback 5 frames.

**Decisions:**
- Time and randomness are fully deterministic, so a failing seed fails the same way every run.
- Messages due on the same tick come out in sending order.
- The test networks stay inside the envelope the session can absorb without a stall. One test shows what happens past it: 20 ticks of latency panics with "further than max_prediction".

**Tests:** 12 new: 7 for the fake network, 4 convergence tests, 1 Riposte-over-network test. Breaking resending on purpose makes all 4 convergence tests fail.

**Learned:** the snapshot ring holds `max_prediction + 2` = 10 states, so with 2 frames of input delay up to about 10 ticks of lateness is survivable, slightly more than `max_prediction` suggests. 12 ticks of latency still converged; 20 did not.

---

## G5: `RollbackGame` and the sync test (Aryaman)

**Gist:**
- `GameState` implements rewind's `RollbackGame`. The snapshot is a full clone of the state, `advance` and `checksum` are the game's own, and `Input` is the `u16` bitmask.
- Riposte passes rewind's `SyncTestSession`: bot-vs-bot for 6000 frames on each of 4 seeds, rolling back 8 frames every tick, with every checksum matching. **This is the gate DESIGN.md sets before any online work.**

**Files:** `game/src/sim/rollback.rs` (the impl and the tests); `game/Cargo.toml` (`rewind` as a path dependency).

**Coverage per seed:** about 100 hits, 30–45 jumps, 4 KOs, 4 round ends, 1–2 full matches. The test fails if any of these is too low.

**Checked that it catches bugs:** two bugs were planted on purpose and removed afterwards.
- `load` forgetting last tick's inputs: desync at frame 60.
- A hidden global counter changing knockback: desync at frame 163.

**Tests:** 3 new (sync test with coverage, save/load round trip, trait methods equal the game's).

```bash
cargo test -p riposte-game sync -- --nocapture
```

**Not covered by this test:** native vs WASM producing identical results. The sim is integer-only, so it should, but that is first truly checked in I2, when a browser plays a native client.

---

## R3: `P2PSession` (Aryaman)

**Gist:**
- The online session. Local inputs apply `input_delay` frames late, and the first `input_delay` frames are "nothing pressed".
- Every tick it receives the peer's inputs. If a guess about them was wrong, it loads the snapshot from the first wrong frame and re-runs every frame since with the real inputs.
- It then sends every local input the peer hasn't acknowledged, snapshots, and advances.
- Old inputs are forgotten once no rollback or resend can need them.

**Files:**
- `rewind/src/session/p2p.rs` (`P2PSession`, `Config`).
- `rewind/src/snapshot.rs` (`SnapshotBuffer`).
- `rewind/src/protocol.rs` (`PeerMsg`, exactly as DESIGN.md section 6).
- `rewind/src/transport/memory.rs` (`memory_pair`, for tests).
- Wiring lines in Ram's `rewind/src/lib.rs` and `rewind/src/session/mod.rs`.
- `serde` derive + `bincode` in `rewind/Cargo.toml`.

**Numbers:** `input_delay` 2, `max_prediction` 8, snapshot ring of 10.

**Decisions:**
- `ack_frame` means "the first of your frames I don't have yet", so 0 means nothing received.
- Receive happens before send, so the acknowledgement we send is as fresh as possible.
- Undecodable messages are ignored, because the peer resends everything unacknowledged anyway.
- A gap in a message's frames stops reading that message.
- Rolling back further than the snapshot ring panics with a clear message. R5's stall is what keeps the session within `max_prediction`, so **R5 is needed before playing over a real network**.

**Tests:** 17 new.
- `SnapshotBuffer` (4) and the memory transport (3).
- The session (10): both peers match a reference run (lockstep, no input delay, bursts of late inputs, lost messages, duplicated messages), input delay hiding one tick of lag with zero rollbacks, constant input never mispredicted, exact resend/ack contents, garbage bytes ignored, and rolling back past the snapshots caught.
- Removing the rollback call makes 5 of the 10 session tests fail, so they really test it.

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

## Learning material (Aryaman)

**Done:** `README.md`, `docs/frame-walkthrough.md` and `docs/exercises.md`, written from the code as it is now. LEARNING.md and SETUP.md are kept current milestone by milestone.

- **Walkthrough:** one light attack traced from the key press to the other player's screen, naming the file and function at every step, plus a worked rollback with numbers.
- **Exercises:** warm-ups, rollback experiments, determinism, new features, and ten explain-it-aloud questions. **Every "you should see" claim in sections A and C was run and confirmed:**
  - Walk speed 4→6: all tests pass.
  - Jump velocity −15→−12: `jump_leaves_floor_and_lands_after_jump_ticks` fails.
  - Heavy hitstun 20→24: `frame_advantage_on_hit` fails.
  - A float in `Fighter::step`: the sync test still **passes** (the exercise explains why).
  - An unhashed new `Fighter` field: compile error in `hash_into`.
- **Correction to DESIGN.md's suggestion:** "introduce a float and watch the sync test catch it" is wrong for a single machine, since a float in the saved state is deterministic there. The exercise instead hides state *outside* `GameState` (caught) and shows the float case (not caught), with the reason.
- **Ram's parts:** the walkthrough covers the whole path, but Ram's own LEARNING.md and docs for R1/R2/R5/R6/S1/S2 are his to keep current.

## Open questions

- **One home for `ClientMsg`/`ServerMsg`:** there are two copies, in `server/src/protocol.rs` and `rewind/src/transport/websocket.rs`. Ram's end-to-end test (`server/tests/rewind_client.rs`) would catch a mismatch, but one shared definition (e.g. in `rewind/src/protocol.rs`, imported by the server) would remove the risk. It moves frozen-protocol code, so it needs both of you to agree.

- **Blocking** isn't in any milestone, so G4 doesn't have it. Suggestion: hold back to block, as a small step before G5.
- **Round timer:** there's no time-out yet, so a round only ends by KO. Should rounds have one (e.g. 99 s, most health wins)?
- **Pushboxes** (body collision): fighters can currently overlap. A small step of its own?
- **Air attacks:** pressing an attack mid-jump does nothing today.
- **Tuning:** all movement and frame-data numbers are first guesses. Ram's G6 includes frame-data tuning.
- **Art:** 2D sprites or 3D models? This affects G7 and the WASM build.
- **Main loop floats:** `main.rs` converts macroquad's `f64` time to integers once. The "floats only in `render/`" rule is read as applying to the sim; say if it should be stricter.
- **`protocol.rs` ownership:** it isn't listed for either of us in CLAUDE.md. R3 created it exactly as DESIGN.md section 6 freezes it; R5 uses its `Checksum` variant.
- **quad-net in the browser (for Ram, R6):** the JS bundle's quad-net plugin fails to register (`register_plugin is not defined`). R6's WASM spike should start by checking how quad-net expects its JS to be loaded with this macroquad version. Native WebSockets aren't affected.
