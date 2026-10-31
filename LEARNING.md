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

**Where:** `game/src/sim/mod.rs` (`GameState::advance`) and `game/src/sim/fighter.rs`. The test `same_inputs_give_same_state` is a first, tiny determinism check; G5's `SyncTestSession` is the real one.

---

## "Just pressed" vs "held" (why the state keeps last tick's inputs)

**What:** `GameState.inputs` stores the inputs used on the most recent tick.

**Why:** An attack should start when the button is *pressed*, not repeat every tick it's held. "Pressed" means "held now, not held last tick", which needs last tick's input. It lives *in the game state* (not in the input layer) so that saving and loading the state for rollback also restores it; otherwise a replayed tick could see the wrong "last input".

**Where:** `game/src/sim/mod.rs`, `GameState.inputs` and the `pressed0`/`pressed1` lines in `GameState::advance`: `pressed = held_now & !held_last_tick`. `Fighter::ground_control` starts attacks only on `pressed`. Test: `holding_the_button_does_not_start_an_attack`.

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

---

## Fixed-point numbers (integer subpixels)

**What:** Positions and speeds are `i32` counts of *subpixels*, where 1 pixel = 256 subpixels. A walk speed of 4 px/tick is stored as 1024; gravity of 0.75 px/tick² is stored as 192. The decimal point is "fixed" at 8 bits from the right, so this is called *fixed-point*.

**Why:** Floats can give slightly different answers on different CPUs, compilers or WASM, depending on rounding and instruction choices. In a rollback game a one-bit difference grows into a full desync. Integer add, subtract and compare are exact and identical everywhere. 256 subpixels per pixel gives smooth fractional speeds while leaving `i32` plenty of range (about 8 million pixels).

**Where:** `game/src/sim/fighter.rs` (`SUBPIXELS` and every constant built from it). `game/src/render/mod.rs`, `px()`, is the one place subpixels become floats, and only for drawing.

---

## Stage coordinates (y down, feet anchor)

**What:** x grows to the right, y grows *downward* (like the screen), so "up" is negative. A fighter's `(x, y)` is the centre of its feet. The floor is `FLOOR_Y` = 640 px, and the stage is 1280 px wide.

**Why:** Matching screen coordinates means the renderer needs no flip. Anchoring at the feet means standing and crouching only change the height, not the position, and "on the floor" is simply `y == FLOOR_Y`.

**Where:** `game/src/sim/fighter.rs` (`FLOOR_Y`, `STAGE_WIDTH`, `Fighter.x/.y`). `game/src/render/mod.rs` (`draw_fighter` draws the box upward from the feet).

---

## Velocity, gravity and the jump arc

**What:** Each tick: position += velocity, then (in the air) velocity.y += gravity. Takeoff sets `vy = JUMP_VELOCITY` (−15 px/tick). Gravity adds 0.75 px/tick each tick, so the fighter slows, stops at the peak, and falls back. When `y` reaches the floor it is clamped and the fighter lands.

**Why:** Constant acceleration gives a parabola, which looks and feels natural. Doing it with integers means the arc is *exactly* the same every time: the jump lasts exactly 41 ticks and peaks at exactly 157.5 px (height after n ticks = 3840n − 96n(n−1) subpixels). The tests check those exact numbers. "Move, then add gravity" is the order chosen; the other order gives a slightly different (also exact) arc, so the order must never change silently.

**Where:** `game/src/sim/fighter.rs`, `Fighter::step`, `JUMP_VELOCITY`, `GRAVITY`. Test: `jump_leaves_floor_and_lands_after_jump_ticks`.

---

## Ground control vs no air control

**What:** Input is read only while on the ground. UP jumps (with the horizontal speed fixed by LEFT/RIGHT at takeoff), DOWN crouches (no walking), LEFT/RIGHT walk, nothing held means idle. Once airborne, input is ignored until landing.

**Why:** This is how classic fighting games work: a jump is a commitment. The opponent can see where you'll land and punish it, which is a core part of the game's strategy. Walking backward is slower than forward (3 vs 4 px/tick) so retreating has a cost.

**Where:** `game/src/sim/fighter.rs`, `Fighter::ground_control`. Tests: `no_air_control`, `walking_forward_is_faster_than_back`, `crouch_stops_walking`.

---

## SOCD cleaning (left + right at once)

**What:** SOCD means "simultaneous opposing cardinal directions": holding LEFT and RIGHT (or UP and DOWN) at the same time. We treat each opposite pair as neutral: `horizontal = right − left`, which is 0 when both are held.

**Why:** On a keyboard it's easy to hold both. Without a rule, the result would depend on which `if` happens to be checked first, which is an accident rather than a decision. Neutral is a common, explicit choice.

**Where:** `game/src/sim/fighter.rs`, `horizontal` and `vertical`. Test: `left_and_right_together_is_neutral`.

---

## Walls (clamping)

**What:** After moving, `x` is clamped to `[half width, stage width − half width]`, so the fighter's box never leaves the stage.

**Why:** Clamping (instead of bouncing) stops the fighter dead at the wall, which is what fighting games do. It also matters later: being "in the corner" is a key situation in G3, because knockback can't push you further.

**Where:** `game/src/sim/fighter.rs`, the end of `Fighter::step`. Test: `walls_stop_fighters_at_both_edges`.

---

## Facing, and why update order must not matter

**What:** On the ground, each fighter turns to face the opponent's x. In the air facing is frozen, so jumping over someone doesn't flip you mid-jump. If both x values are equal, the old facing is kept.

**Order independence:** `GameState::advance` reads *both* positions first, turns both fighters, and only then moves them. If P1 moved first and P2 then turned based on P1's new position, P1 would always see slightly older information than P2. Reading everything at the start of the tick makes the result the same whichever player is updated first.

**Where:** `game/src/sim/fighter.rs`, `Fighter::face`. `game/src/sim/mod.rs`, `GameState::advance`. Tests: `faces_opponent_on_ground_only`, `fighters_turn_around_after_crossing`.

---

## Action state machine

**What:** A fighter is always in exactly one `Action`: Idle, Walk, Crouch, Jump, Light, Heavy or Hitstun. Each tick, `Fighter::step` first advances the current action's timer (an attack's `action_frame` counts up, `hitstun` counts down). If that timer has run out the fighter goes back to Idle, and only an *actionable* fighter (Idle/Walk/Crouch, on the floor) reads input.

**Why:** It makes "what can this fighter do right now?" a single question with one answer. Mid-attack you can't walk, jump or start another attack, so committing to a move has a cost. A hit replaces whatever action was running with Hitstun, which is how a faster attack interrupts a slower one. Without one explicit state, these rules turn into a tangle of booleans that disagree with each other.

**Where:** `game/src/sim/fighter.rs`: `Action`, `Fighter::step`, `tick_action`, `is_actionable`, `ground_control`, `start_attack`.

---

## Frame data (startup, active, recovery)

**What:** Each attack is described by numbers of ticks ("frames"):
- **startup:** before the hitbox appears (you're committed but harmless),
- **active:** the hitbox exists and can hit,
- **recovery:** the hitbox is gone but you can't act yet.

The light attack is 4 / 3 / 8 (15 total); the heavy is 10 / 4 / 20 (34 total). `action_frame` is 0 on the tick the attack starts, so the light's hitbox exists on frames 4, 5 and 6.

**Why:** These numbers *are* the game design. Short startup means the attack beats slower ones. Long recovery means a miss can be punished. Because the sim runs at a fixed 60 ticks/s, a frame count is an exact duration on every machine.

**Where:** `game/src/sim/attack.rs`: `Attack`, `LIGHT_ATTACK`, `HEAVY_ATTACK`, `Attack::is_active`, `Attack::total`. Tests: `active_only_between_startup_and_recovery`, `attack_lasts_exactly_its_total_frames`.

---

## AABB overlap (axis-aligned bounding boxes)

**What:** A rectangle that never rotates, stored as its four edges. Two such boxes overlap exactly when they overlap on x *and* on y:
`a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom`.
We use strict `<`, so boxes that only touch along an edge don't count.

**Why:** It's four integer comparisons, with no square roots or angles, and it's exact and deterministic. Fighting games have used rectangles for hit detection since the arcade era for exactly this reason.

**Where:** `game/src/sim/rect.rs`, `Rect::overlaps`, with tests for overlap, separation on each axis and touching edges.

---

## Hitbox vs hurtbox

**What:** A **hurtbox** is where a fighter *can be hit* (here, their whole body; shorter when crouching). A **hitbox** is where an attack *hits*, and it exists only during the attack's active frames. A hit happens when the attacker's hitbox overlaps the defender's hurtbox. The hitbox is stored relative to the attacker's feet as "forward" reach and "upward" height, and mirrored when the attacker faces left.

**Why:** Keeping the two separate lets attacks have their own shape and timing. The light attack hits high (130–160 px up), so it passes over a crouching opponent (120 px tall); the heavy hits mid (60–110 px), so it reaches crouchers but is slow. That gives crouching a purpose without any extra code.

**Where:** `Fighter::hurtbox`, `Fighter::active_hitbox` (`game/src/sim/fighter.rs`); `Attack::hitbox` (`game/src/sim/attack.rs`). The placeholder view draws the sim's own boxes (hitbox outlined in red), so what you see is what the code checks. Tests: `hitbox_mirrors_with_facing`, `light_whiffs_over_crouch_but_heavy_hits_it`.

---

## One hit per swing

**What:** `Fighter.attack_connected` is set when an attack lands and reset when a new attack starts. While it's set, the remaining active frames can't hit again.

**Why:** The light attack is active for 3 ticks. Without this flag, a hitbox overlapping the opponent for all 3 would deal damage 3 times.

**Where:** `game/src/sim/mod.rs`, `check_hit` and the hit-applying lines in `GameState::advance`. Test: `one_swing_hits_only_once`.

---

## Resolving hits without favouring player 1 (trades)

**What:** After both fighters move, `advance` checks P1-hits-P2 *and* P2-hits-P1 using the same snapshot of both fighters, and only then applies the results. If both hit on the same tick, both take damage (a *trade*).

**Why:** If we checked and applied P1's hit first, P2 would already be in hitstun with no hitbox, and their hit would vanish, so P1 would win every tie. It's the same principle as facing in G2: read everything first, then write.

**Where:** `game/src/sim/mod.rs`, `GameState::advance`. Tests: `simultaneous_hits_trade`, `faster_attack_interrupts_slower_one`.

---

## Damage, hitstun and knockback

**What:** On a hit the defender:
- loses `damage` health, never below 0,
- enters `Hitstun` for `hitstun` ticks and can't act,
- gets pushed away from the attacker at `knockback` subpixels/tick.

During hitstun the knockback speed shrinks by `KNOCKBACK_FRICTION` each tick, so the push slows to a stop. A fighter hit in the air stays in Hitstun but keeps falling, because "airborne" is now decided by position (`y < FLOOR_Y`), not by action.

**Why:** Hitstun is what makes a hit matter beyond the damage: it's time the opponent can't use. Knockback creates space so attacks don't stack forever, and walls stop it, which makes the corner dangerous.

**Where:** `Fighter::take_hit`, `tick_action`, `toward_zero` (`game/src/sim/fighter.rs`). Tests: `hitstun_blocks_input_then_ends`, `knockback_slows_down_during_hitstun`, `knockback_pushes_the_defender_away`, `hit_in_the_air_still_falls`.

---

## Frame advantage

**What:** After an attack connects, who gets to act first? Compare the defender's hitstun with how long the attacker still has to wait (rest of active + recovery). If the attacker recovers first, the attack is *plus on hit*. The light here hits on frame 4 (tick T). The attacker finishes frames 5–14 and is free on tick T+11. The defender's 15 ticks of hitstun run out on tick T+15. So the attacker acts 4 ticks first: the light is **+4 on hit**. The heavy, by the same sums, is −3 on hit (free on T+24 vs T+21), so even landing it hands the opponent the initiative.

**Why:** Frame advantage decides who has the initiative after every exchange, which is the core of fighting-game strategy. It falls out of the frame data, which is why the numbers in `attack.rs` matter more than they look.

**Where:** the frame data in `game/src/sim/attack.rs`. Test: `light_on_hit_leaves_attacker_free_first` in `game/src/sim/mod.rs`.
