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
| G4: rounds, health bars, KO, reset, PRNG + FNV checksum | Aryaman | Not started |
| G5: `RollbackGame` + `SyncTestSession` | Aryaman | Not started (needs R2) |
| G6: game feel (render-only) | Ram | Partly done: flash + shake (hitstop visuals and frame-data tuning not done, see below) |
| G7: menu, WASM build | Aryaman | Not started |
| R1: crate skeleton, `api.rs`, `InputQueue` | Ram | Not started |
| R2: `SyncTestSession` + `boxes` example | Ram | Not started |
| R3: `P2PSession` | Aryaman | Not started (needs R1) |
| R4: fake-network transport + convergence tests | Aryaman | Not started |
| R5: checksums, desync, stall | Ram | Not started (needs R3) |
| R6: WebSocket transport | Ram | Not started |
| S1, S2: relay server | Ram | Not started |
| I1–I3: online play, browser, deploy | Together | Not started |

**Tests:** 54 unit tests, all passing (`cargo test --workspace`). fmt and clippy clean.

---

## How to try what exists

```bash
cargo run -p riposte-game
```

| | Move | Jump | Crouch | Light | Heavy |
|---|---|---|---|---|---|
| P1 | A / D | W | S | F | G |
| P2 | ← / → | ↑ | ↓ | K | L |

The screen is a placeholder debug view: grey boxes are the fighters' hurtboxes, a black notch shows facing, a red outline is an active hitbox, and each label shows the current action and health. Real visuals will come from third-party art.

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

**Not built yet:** blocking, air attacks, pushboxes (fighters can walk through each other), KO and rounds (G4).

---

## G6: game feel (render-only)

**Gist:** A hit now flashes the defender red and shakes the view, harder and longer for bigger hits (light: 5 px for 3 ticks; heavy: 12 px for 8 ticks). `Effects` only reads `GameState`; it never writes to it.

**Files:** `game/src/render/effects.rs` (new), plus small wiring in `game/src/render/mod.rs` (`draw` takes `&mut Effects`, the view takes a shake offset, `flash_tint`) and `game/src/main.rs` (creates the `Effects`). The `mod.rs` and `main.rs` edits touch Aryaman's files, as DESIGN.md says G6 is "called from render/mod.rs".

**Decisions:**
- Hits are detected by a health drop between two draws, so the sim needed no changes and no hit event.
- Timers count sim ticks (frame delta), so they don't depend on frame rate.
- Frame going backwards or health rising resets the baseline without firing effects.

**Tests:** 9 in `effects.rs`: no hit, defender-only flash, fade in ticks, redraw doesn't fade, heavy > light, shake jitter and end, small hit doesn't cut a big shake, restart not mistaken for damage, health rising.

**Not done:**
- **Hitstop visuals:** the sim has no hitstop, so freezing the picture in render would show something the sim doesn't do. If wanted, hitstop should be added to the sim first (Aryaman's files).
- **Health-bar animation:** health bars don't exist yet (G4).
- **Frame-data tuning:** needs edits to `sim/attack.rs`, which is Aryaman's. Needs a playtest and agreement first.

---

## Open questions

- **Blocking** isn't in any milestone. Suggestion: hold back to block, added in G4.
- **Pushboxes** (body collision): fighters can currently overlap. G4, or a small step of its own?
- **Air attacks:** pressing an attack mid-jump does nothing today.
- **Tuning:** all movement and frame-data numbers are first guesses. Ram's G6 includes frame-data tuning.
- **Art:** 2D sprites or 3D models? This affects G7 and the WASM build.
- **Main loop floats:** `main.rs` converts macroquad's `f64` time to integers once. The "floats only in `render/`" rule is read as applying to the sim; say if it should be stricter.
- **R3 timing:** R3 needs Ram's R1 (`rewind` crate + `api.rs`), which isn't on `main` yet.
