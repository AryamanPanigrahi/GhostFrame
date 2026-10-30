# LEARNING.md

Every concept in Riposte, explained from first principles, with where it lives in the code. Read top to bottom: later entries build on earlier ones.

---

## Cargo workspace

**What:** A folder whose `Cargo.toml` has a `[workspace]` section listing several crates (`members`). They share one `Cargo.lock` and one `target/` folder.

**Why:** Riposte is three crates (`rewind`, `riposte-game`, `riposte-server`) that must agree on shared dependency versions (`serde`, `bincode`) and be tested together. A workspace lets `cargo test --workspace` build and test everything in one go, and stops each crate from compiling its own copy of the same dependency.

**Where:** `Cargo.toml` (root). Each crate still has its own `Cargo.toml` for its own dependencies, e.g. `game/Cargo.toml`.

---

## Render frames vs simulation ticks

**What:** A *frame* is one picture drawn on screen. A *tick* is one step of the game simulation. They are different things and run at different rates.

**Why:** Frame rate depends on the monitor and the machine (30, 60, 144, 240 Hz, with occasional hitches). If the game advanced once per frame, a player on a 144 Hz monitor would move 2.4× faster than one on 60 Hz. Worse, for online play both machines must run *exactly* the same sequence of steps, and rollback must be able to re-run "tick 812" and get the same answer. So the sim advances in fixed 1/60 s ticks no matter how often we draw. In the smoke test the game rendered at ~210 fps while the sim advanced exactly 60 ticks per second.

**Where:** `game/src/main.rs` (`main` loop: run due ticks, then draw once).

---

## Fixed timestep with an accumulator

**What:** Each frame, add the real time that passed to a "bank". While the bank holds at least one tick's worth (1/60 s), withdraw it and run one tick. Whatever is left over (less than one tick) stays in the bank for next frame.

**Why:** It turns irregular real time into a regular count of ticks. At 144 fps most frames run 0 ticks and some run 1. At 30 fps every frame runs 2. Over one real second it's always 60 in total.

**The spiral-of-death cap:** If the game freezes for 5 seconds (window dragged, laptop asleep), the bank would hold 300 ticks and the game would fast-forward through them. Worse, if ticks are ever slower to compute than real time, the bank grows forever. So we run at most `MAX_TICKS_PER_FRAME` (4) per frame and throw the excess away.

**Where:** `game/src/clock.rs`, `FixedClock::advance`. Tests in the same file check 60 ticks per second at 30/60/75/144/240 fps.

---

## Integer time units (avoiding 1/60's fraction)

**What:** 1/60 s is 16,666.666… microseconds, not a whole number. Instead of using floats, the clock counts in units of 1/60,000,000 s (microseconds × 60). In those units one tick is exactly 1,000,000.

**Why:** Rounding 16,666.67 down to 16,666 µs per tick would make the game run slightly fast and drift over time. Floats would also work, but the project keeps time and game logic in integers wherever it can, so there's never a question of float rounding differing between machines. Scaling by 60 makes the division exact.

**Where:** `game/src/clock.rs`, the `banked` field. `game/src/main.rs`, `now_micros`, which is the single place macroquad's `f64` seconds are converted to whole microseconds.

---

## Input bitmask and bitwise operations

**What:** Each button is one bit of a `u16`: LEFT = bit 0 (`1 << 0` = 1), RIGHT = bit 1 (`1 << 1` = 2), UP = 4, DOWN = 8, LIGHT = 16, HEAVY = 32. A player's whole input for one tick is these bits OR'd together. Holding LEFT and LIGHT is `1 | 16 = 17`.
- `input |= button` sets a bit (press).
- `input & button != 0` tests a bit (is it held?).

**Why:** It's 2 bytes per player per tick, which matters because inputs are the *only* thing sent over the network. Comparing two whole inputs is a single integer compare, which rollback does constantly ("was my prediction right?"). And it's `Copy`, `PartialEq` and `Default` (0 = nothing pressed), which is exactly what rewind's `RollbackGame::Input` requires.

**Where:** `game/src/sim/mod.rs` (`Input`, the button constants, `is_held`). `game/src/input.rs` (`input_from_keys` builds the mask from the keyboard).

---

## Keyboard is outside the sim

**What:** `input.rs` is the only file that knows about keys. It turns key state into an `Input` bitmask, and the sim only ever sees bits.

**Why:** Online, player 2's input doesn't come from this keyboard at all; it comes from the network. In rollback, old inputs are replayed from memory. If the sim read the keyboard directly, none of that could work. The sim must be a pure function of (state, inputs).

**Testing trick:** macroquad's `is_key_down` needs a real window, so `input_from_keys` takes the "is this key down?" check as a plain function pointer (`fn(KeyCode) -> bool`). The game passes `is_key_down`, and tests pass a fake that returns `true` for chosen keys.

**Where:** `game/src/input.rs` (`P1_KEYS`, `P2_KEYS`, `read_input`, `input_from_keys`).

---

## Deterministic simulation (the core rule)

**What:** Given the same starting state and the same inputs, `GameState::advance` must produce bit-for-bit the same result on every machine and every run.

**Why:** Each client runs the full game and only inputs are exchanged. If the two sims ever compute different results from the same inputs, the players see different games (a *desync*). Rollback also re-runs past ticks and assumes it gets the same answer as before. Floats, hash-map iteration order, system time and unseeded randomness can all differ between machines or runs, so they're banned inside `sim/` (see CLAUDE.md).

**Where:** `game/src/sim/mod.rs`. For now `GameState` only holds `frame` and the last `inputs`; fighters arrive in G2. The test `same_inputs_give_same_state` is a first, tiny determinism check; G5's `SyncTestSession` is the real one.

---

## "Just pressed" vs "held" (why the state keeps last tick's inputs)

**What:** `GameState.inputs` stores the inputs used on the most recent tick.

**Why:** An attack should start when the button is *pressed*, not repeat every tick it's held. "Pressed" means "held now, not held last tick", which needs last tick's input. It lives *in the game state* (not in the input layer) so that saving and loading the state for rollback also restores it; otherwise a replayed tick could see the wrong "last input".

**Where:** `game/src/sim/mod.rs`, `GameState.inputs`. Used for real in G3.

---

## Session (LocalSession)

**What:** The object the main loop hands inputs to and asks to "tick". `LocalSession` is the simplest version: it takes both players' inputs from one keyboard and advances the game once per tick, with no prediction or rollback.

**Why:** It has the same shape as rewind's `P2PSession` (`add_local_input`, then `tick(&mut game)`). For online play (G5, I1) we swap in rewind's session and the main loop barely changes. Also, inputs are read once per *frame* but applied once per *tick*, so if a frame runs 2 ticks both get the same held buttons.

**Where:** `game/src/session.rs` (`LocalSession`). Called from `game/src/main.rs`.

---

## Logical resolution and letterboxing

**What:** The game world is always 1280×720 *logical* pixels. At draw time we compute one scale factor, `min(window_w / 1280, window_h / 720)`, so the whole stage fits, and centre it with black bars on the leftover sides.

**Why:** Players with different window sizes must see the same stage (same reach, same walls); only the picture gets bigger or smaller. Taking the *minimum* of the two ratios keeps the 16:9 shape instead of stretching it.

**Where:** `game/src/render/mod.rs`, `View::for_window`. The drawing itself is a placeholder; the real visuals will come from third-party art.

---

## macroquad's async main loop

**What:** macroquad's `main` is an `async fn`, and each frame ends with `next_frame().await`. The `#[macroquad::main(window_conf)]` attribute creates the window from `window_conf()` and runs that async function.

**Why:** In a browser (WASM) a program can't run an endless `loop`; it has to return control to the browser every frame or the page freezes. `await` is the point where macroquad hands control back and resumes us on the next frame. Natively it works the same way, so one loop works for both targets (G7).

**Where:** `game/src/main.rs`, `main` and `window_conf`.
