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

**What:** After an attack connects, who gets to act first? Compare the defender's hitstun with how long the attacker still has to wait (rest of active + recovery). If the attacker recovers first, the attack is *plus on hit*. The light here hits on frame 4 (tick T). The attacker finishes frames 5–14 and is free on tick T+11. The defender's 15 ticks of hitstun run out on tick T+15. So the attacker acts 4 ticks first: the light is **+4 on hit**. The heavy, by the same sums, is −4 on hit (attacker free on T+24, defender on T+20), so even landing it hands the opponent the initiative.

**Why:** Frame advantage decides who has the initiative after every exchange, which is the core of fighting-game strategy. It falls out of the frame data, which is why the numbers in `attack.rs` matter more than they look.

**Where:** the frame data in `game/src/sim/attack.rs`. Test: `frame_advantage_on_hit` in `game/src/sim/mod.rs` checks both numbers.

---

## Round flow (a second state machine)

**What:** On top of each fighter's `Action`, the game has a `Phase`:
- **Ready** (1 s): fighters placed, input ignored.
- **Fight.**
- **RoundOver** (2 s) after a KO.
- **MatchOver** (4 s) once someone has 2 round wins. Then a fresh match starts.

Each pause has a `timer` that counts down once per tick; at 0, `tick_round` moves to the next phase.

**Why:** Fighting games need clean breaks: nobody should be able to attack during the countdown or hit a KO'd opponent. Gating input on the phase (`Round::accepts_input`) does that in one place instead of sprinkling checks around. Physics still runs during pauses, so a fighter KO'd mid-air still falls and lands.

**Where:** `game/src/sim/round.rs` (`Phase`, `Round`, the tick constants). `game/src/sim/mod.rs`: `GameState::advance` (input gating), `check_ko`, `tick_round`, `start_round`, `end_match`. Tests: `match_starts_in_ready_and_ignores_input`, `ko_ends_the_round_and_scores`, `next_round_resets_fighters_but_keeps_score`, `second_win_takes_the_match_then_a_new_match_starts`.

---

## KO, knockdown and the double KO

**What:** When health hits 0, the fighter goes into `Knockdown` (no input, no turning, slides to a stop) and the other player scores a round win. Hits are only checked during `Fight`, so a swing that's still active after the KO can't knock the fighter out of Knockdown. If a trade takes *both* fighters to 0 on the same tick, it's a draw: nobody scores and the round is replayed.

**Why:** The double KO falls straight out of G3's "check both hits, then apply both" rule; we just had to decide what it means. A draw avoids the strange case of both players reaching 2 wins at once.

**Where:** `GameState::check_ko` (`game/src/sim/mod.rs`), `Fighter::knock_down` (`game/src/sim/fighter.rs`). Tests: `input_is_ignored_after_ko`, `double_ko_is_a_draw_and_replays_the_round`, `knocked_down_fighter_cannot_act_or_turn`.

---

## Seeded PRNG (xorshift64)

**What:** A pseudo-random number generator is a function that turns one number (the *state*) into the next, in a sequence that *looks* random. xorshift64 does it with three steps: `x ^= x << 13; x ^= x >> 7; x ^= x << 17`. The starting state is the *seed*. Zero is the one seed that never changes (0 shifted and xored is still 0), so it's replaced with a fixed non-zero constant.

**Why:** OS randomness (`thread_rng`) differs per machine, which would desync online play instantly. A generator whose state lives in `GameState` gives the same numbers on both machines (same seed from the server's `MatchStart`). It's also saved and restored with everything else, so rollback replays the same "random" numbers. Nothing in the sim *uses* randomness yet; it's in the state now so the checksum and rollback already cover it.

**Where:** `game/src/sim/rng.rs` (`Rng::new`, `Rng::next_u64`). `GameState.rng`, seeded by `GameState::new(seed)`; locally `LOCAL_SEED` in `game/src/main.rs`. Tests: same seed same sequence, zero seed not stuck, a known first value.

---

## FNV-1a checksum

**What:** A hash function turns any amount of data into a fixed-size number (here 64 bits) that changes if any input bit changes. FNV-1a starts at a fixed "offset basis" and, for each byte, XORs the byte in and multiplies by a fixed prime. The multiply smears each byte's effect across all 64 bits.

**Why:** Online, the two machines can't compare whole game states every few frames (too big), but they can compare one `u64`. If the checksums for the same frame differ, the games have desynced (DESIGN.md section 5, step 7). We write FNV-1a ourselves because Rust's built-in hasher is allowed to change between versions and platforms, and ours must be identical on native and WASM. Tests check it against the official FNV test vectors.

**Where:** `game/src/sim/checksum.rs` (`Fnv1a`). `GameState::checksum` (`game/src/sim/mod.rs`). Shown on screen in the placeholder debug line.

---

## Making the checksum complete and stable

**Every field:** `GameState::checksum` and `Fighter::hash_into` start with a destructuring `let` that names every field (`let Fighter { x, y, ... } = *self;`). If someone adds a field and forgets to hash it, Rust refuses to compile, because the pattern no longer lists every field. A missing field would otherwise be a silent hole: two machines could disagree on it and the checksum would never notice.

**Same bytes everywhere:** numbers are fed in little-endian order (`to_le_bytes`), so the bytes are identical on every CPU. Enums are hashed through explicit tags (`Action::tag`, the `match` in `hash_round`) rather than `as u8`, so reordering variants in the source can't quietly change every hash.

**Order matters:** players are hashed P1 then P2, so swapping them gives a different checksum.

**Where:** `game/src/sim/mod.rs` (`checksum`, `hash_round`), `game/src/sim/fighter.rs` (`hash_into`, `Action::tag`). Tests: `changing_any_part_of_the_state_changes_the_checksum`, `swapping_the_players_changes_the_checksum`, `numbers_are_little_endian`.

---

## The main menu (a state machine outside the sim)

**What:** `Menu` has a `Screen`:
- **Main:** a list with a selected item.
- **EnterCode:** a 4-letter room code being typed.
- **Notice:** a message plus "back".

`Menu::handle` takes one `MenuKey` (Up, Down, Confirm, Back, Backspace, or a typed letter) and may return a `MenuAction` (StartLocal, CreateRoom, JoinRoom with the code). `main.rs` acts on the action. Room codes accept letters only, stored uppercase, exactly 4.

**Why:** The same "pure logic + separate keyboard reading" split as the game input: `input.rs` turns real keys into `MenuKey`s, so the menu is tested without a window. The menu isn't part of the simulation, so none of the determinism rules apply to it. The two online items show a "not connected yet" notice until I1 wires them to the relay server.

**Where:** `game/src/menu.rs`; `read_menu_keys` and `menu_keys_from` in `game/src/input.rs`; `draw_menu` in `game/src/render/mod.rs`; the `App` enum in `game/src/main.rs`.

---

## Switching screens in Rust (`App` and `switch_to`)

**What:** `main.rs` holds `app: App`, which is either `Menu(Menu)` or `Local(LocalMatch)`. Inside `match &mut app { ... }` we're *borrowing* `app`, so we can't replace it there. Instead we set `switch_to = Some(next)` and do `app = next` after the match ends.

**Why:** Rust forbids changing a value while something borrows it; that's how it prevents use-after-free bugs. Deciding inside and swapping afterwards is the standard way around it. Starting a match makes a fresh `LocalMatch` with a fresh `FixedClock`, so time spent in the menu isn't banked as game ticks.

**Where:** `game/src/main.rs`, `main`, `App`, `LocalMatch::new`.

---

## Not losing quick taps (frames vs ticks again)

**What:** Two fixes so a fast key tap always reaches the sim:
1. **Within a frame:** a key counts as held if it's down now **or** was pressed since the last frame (`is_key_down || is_key_pressed`). A tap that goes down and up between two frames is otherwise invisible to `is_key_down`.
2. **Across frames:** `LocalMatch` ORs each frame's input into `buffered` and only clears it after a tick runs. On a 144 Hz screen most frames run *no* tick, and a tap seen on one of those frames would otherwise never be applied.

**Why:** Found by testing the browser build: at the ~30 fps of software rendering, a tap of the attack key was dropped. Online play will need the same care, because the input that rewind sends is whatever we sample.

**Where:** `held_or_tapped` in `game/src/input.rs`; `LocalMatch::update` in `game/src/main.rs`.

---

## WebAssembly and the JS glue

**What:** `cargo build --target wasm32-unknown-unknown` turns the same Rust code into a `.wasm` file. The browser can run it, but WASM on its own has no screen, keyboard or clock; it can only call functions the page hands it. `mq_js_bundle.js` is macroquad's JavaScript that hands it those functions (WebGL drawing, key events, time) and starts it via `load("riposte-game.wasm")`.

**Why:** It's one codebase for desktop and browser, and the sim is integer-only, so it gives identical results on both. That's essential for a native player against a browser player. The JS bundle must come from the *same* macroquad version as the Rust side, because the two halves call each other by name.

**Where:** `web/index.html`, `web/mq_js_bundle.js`, `scripts/build-web.sh`. Steps in SETUP.md section 9.

---

## Rollback netcode (the big idea)

**What:** Each player's machine runs the whole game. Only inputs travel over the network. When the opponent's input for a frame hasn't arrived yet, we don't wait: we *guess* it and keep playing. When the real input arrives, if the guess was wrong we *roll back*: load the saved state from that frame, and quickly re-run every frame since with the corrected input, all within one tick, before drawing.

**Why:** Waiting for the opponent's input every frame (delay-based netcode) makes the game feel as laggy as the connection. Guessing makes your own inputs respond instantly. Most guesses are right, because players hold the same buttons for many frames in a row, so rollbacks are rare and short. The cost is that the game must be able to save, load and re-run itself exactly, which is why the sim is deterministic.

**Where:** the trait every game implements is `RollbackGame` in `rewind/src/api.rs`. The rollback loop itself is `P2PSession` (R3, `rewind/src/session/p2p.rs`).

---

## Traits and associated types (`RollbackGame`, `Transport`)

**What:** A *trait* is a list of methods a type promises to have, like an interface. `RollbackGame` says "a game can `save`, `load`, `advance` and `checksum`". An *associated type* is a type the implementer picks: `type Input` and `type State`. Riposte will say `type Input = u16` and `type State = GameState`; the `boxes` toy game picks its own.

**Why:** It's what makes rewind game-agnostic. The session code is written once against the trait and works with any game that implements it. The bounds on `Input` (`Copy + PartialEq + Default + Serialize + DeserializeOwned`) are exactly what rollback needs: copy it around freely, compare a guess with the real input, have a "nothing pressed" value to guess before any input exists, and send it over the network. `Transport` does the same for networking: `send` bytes, `poll` for received bytes, so the session doesn't care if it's an in-memory pair (tests), a fake laggy network (R4) or a WebSocket (R6).

**Where:** `rewind/src/api.rs`. This file is frozen: changing it needs both authors to agree (DESIGN.md section 4).

---

## Input prediction and the InputQueue

**What:** `InputQueue` holds one player's confirmed inputs in frame order. Asking it for a frame it doesn't have yet returns a *prediction*: the last confirmed input (or "nothing pressed" if there's none). It remembers which frames it predicted, and when the real input for one of them arrives, it compares the two. The earliest frame where the guess was wrong is reported by `first_incorrect_frame`: that's the frame the session rolls back to.

**Why:**
- *Predict "same as last"* because it's simple and usually right: at 60 frames per second, buttons change rarely compared to how often they stay the same.
- *Only the earliest wrong frame matters*, because rolling back to it re-runs every later frame anyway.
- *Inputs must arrive in order* (`AddOutcome::Gap` otherwise). A hole would be a frame we could never confirm. The protocol avoids holes by resending every unacknowledged input in each message, which is also why repeats (`AddOutcome::AlreadyHad`) are normal and ignored.
- *`reset_prediction`* is called when the session rolls back. The old guesses are about to be thrown away and re-made with newer information, so comparing against them later would report rollbacks that aren't needed.
- *`discard_before`* drops inputs no rollback can reach any more (older than `max_prediction` frames), so memory doesn't grow for the whole match. A `VecDeque` (a double-ended queue) makes dropping from the front cheap.

**Where:** `rewind/src/input_queue.rs` (`InputQueue::add_input`, `input`, `first_incorrect_frame`, `reset_prediction`, `discard_before`). Tests in the same file: `predicts_last_confirmed_input`, `wrong_prediction_is_flagged`, `earliest_wrong_frame_is_kept`, `reset_clears_predictions_and_the_flag`.

---

## Sync test (proving determinism offline)

**What:** `SyncTestSession` runs a game on one machine with no network. Every tick it advances one frame normally and records the checksum. Then it loads the state from `check_distance` frames ago (8, the furthest a real rollback goes), replays those frames with the same inputs, and checks every checksum matches the first run. Any difference is returned as a `SyncTestError` naming the frame.

**Why:** Online, a determinism bug shows up as a desync minutes into a match, on two machines, with no clue where it started. The sync test makes every single tick a rollback, so the same bug appears within a few frames, locally, with the exact frame. The usual culprits it catches:
- state that `save`/`load` forget (a counter or timer kept outside `State`),
- reading the clock, the window or a random generator that isn't part of the state,
- iteration order that isn't fixed (`HashMap`),
- floats that round differently.

It's why DESIGN.md says the game must pass it before any online work is merged (G5).

**Where:** `rewind/src/session/synctest.rs` (`SyncTestSession::tick`). Tests: `deterministic_game_passes`, `state_missing_from_save_is_caught`, `rollbacks_leave_the_same_state_as_a_plain_run`. Demo: `cargo run -p rewind --example boxes` passes, and `-- --broken` (a tick counter left out of `save`) fails at frame 7.

---

## Checksums with FNV-1a

**What:** A checksum squeezes a whole game state into one `u64`. FNV-1a starts from a fixed number and, for each byte of the state, XORs the byte in and then multiplies by a fixed prime. Two identical states always give the same number, and different ones almost always give different numbers.

**Why:** Comparing whole states every frame (or sending them over the network) would be slow and large. Comparing one `u64` is instant and 8 bytes to send. We write our own instead of using Rust's `DefaultHasher` because that one is allowed to change between Rust versions and is randomly seeded in `HashMap`. Our checksum has to be identical on every machine, native or WASM, forever.

**Where:** `Fnv1a` in `rewind/examples/boxes.rs` (the toy game's `checksum`). Riposte gets its own copy in G4.

---

## Toy game for testing a library (`examples/boxes.rs`)

**What:** Two boxes moving around a walled arena, with integer positions, implemented as a `RollbackGame`. Cargo builds files in `examples/` against the library as if they were an outside user, and runs one with `cargo run -p rewind --example boxes`.

**Why:** It proves rewind really is game-agnostic: it works with a game that isn't Riposte. It's also small enough to debug rewind itself without the fighting game in the way, and `cargo test` builds it, so an API change that breaks it is caught.

**Where:** `rewind/examples/boxes.rs`.

---

## Relay server

**What:** A program that sits between two players and passes bytes from one to the other. It pairs them by a 4-letter room code, tells each which player they are, and then forwards whatever one sends to the other without reading it. Rollback needs this much only because browsers can't connect to each other directly.

**Why "dumb":** Each client runs the whole game and only inputs travel, so the server never needs to understand the game. It is a few hundred lines, has no game logic to get wrong, and the same relay works for any `Transport` the clients speak.

**The lifecycle:**
1. A client connects to `/ws` and sends `CreateRoom`. The server makes a free 4-letter code and replies `RoomCreated`.
2. The second client connects and sends `JoinRoom { code }`. The server picks one random seed and sends `MatchStart` to both, with `player_index` 0 for the host and 1 for the guest. The same seed means both games' PRNGs start identically.
3. From then on each `Relay(bytes)` goes to the other player unchanged.
4. When either connection ends, the other gets `PeerLeft`, the room is deleted, and the server closes that connection. To play again, a client reconnects.

**Bad requests:** the protocol has no error message, and it is frozen. So a client that joins an unknown or full room, sends bytes that don't decode, sends a message over 16 KiB, or creates a second room is simply disconnected. The client sees a closed socket.

**Where:** `server/src/protocol.rs` (the two message enums), `server/src/rooms.rs` (`Rooms::create_room`, `join_room`, `relay`, `leave`), `server/src/lib.rs` (`handle_socket`). Tests: unit tests in `rooms.rs`, and `server/tests/relay.rs` which runs a real server and real WebSocket clients.

---

## Async tasks and channels (tokio)

**What:** An *async runtime* runs thousands of lightweight tasks on a handful of OS threads. A task that is waiting for the network gives up its thread until bytes arrive. Here each connection is one task, `handle_socket`.

**Why a channel:** Player A's task needs to push bytes into player B's socket, but only B's task owns that socket. So every connection has an *outbox*, a channel (`mpsc`: many senders, one receiver). A's task drops a message into B's outbox, and B's task, which is waiting in `tokio::select!` on "socket has data" or "outbox has a message", writes it out. The room logic only ever touches outboxes, never sockets, which is why `rooms.rs` can be unit tested with no network.

**Where:** `server/src/lib.rs` (`handle_socket` and its `select!` loop), `server/src/rooms.rs` (`Outbox`).

---

## Shared state behind a Mutex

**What:** All rooms live in one `Rooms` value that every connection task needs to change. `Arc<Mutex<Rooms>>` shares it: `Arc` is a reference-counted pointer so many tasks can own it, and `Mutex` lets one task at a time edit it.

**Why a plain `std` Mutex and not tokio's:** we only hold the lock for a few map operations and never across an `.await`. Holding a std lock across an await can stall other tasks, so the rule is: lock, edit, unlock, then await.

**Where:** `AppState` in `server/src/lib.rs`.

---

## Idle timeouts

**What:** A room remembers the time of its last activity (create, join, or a relayed message). A background task wakes every 30 seconds and deletes rooms that have been quiet for more than 5 minutes, telling their players `PeerLeft`.

**Why:** Without it, abandoned rooms (someone creates a room, shares the code, never joins) would pile up forever. A live match is never idle, because both clients send inputs 60 times a second.

**Where:** `Rooms::sweep_idle` in `server/src/rooms.rs`, `spawn_idle_sweeper` in `server/src/lib.rs`. Tested with fake `Instant`s in `sweep_removes_only_rooms_idle_past_the_limit`, and for real in `idle_rooms_are_deleted`.

---

## Game feel: render-only effects

**What:** Hit flash and screen shake make a hit *feel* like something, but they live entirely in `render/`. `Effects` watches the sim's state each draw: when a fighter's health drops, a hit just landed, so that fighter flashes and the view shakes, harder and longer for bigger damage. Timers count down in sim ticks (the change in `game.frame`), not wall-clock time, so the effect lasts the same on a 60 Hz or a 240 Hz monitor, and redrawing the same tick twice doesn't fade it.

**Why:** The sim must stay deterministic and identical on both peers, and rollback re-runs it. If effects were in the sim they would be part of the checksum and could desync. Reading the state and never writing to it keeps feel and fairness separate. A frame counter going backwards (restart) or health going up is treated as a new baseline, never as damage.

**Where:** `game/src/render/effects.rs` (`Effects::observe`, `flash`, `shake_offset`), called from `draw` in `game/src/render/mod.rs`. Tests: the `tests` module in `effects.rs`.

---

## Picking sprite frames from sim state

**What:** Each fighter's sprite is a numbered PNG in an animation folder (`fighter/p1/heavy/03.png`). `Animator::pose` turns the sim's state into "which folder, which drawing". Attacks use `action_frame` against a timeline of ticks per drawing (`HEAVY_TIMELINE`); the jump uses the vertical speed (since `vy = JUMP_VELOCITY + GRAVITY * t`, the ticks since takeoff are `(vy - JUMP_VELOCITY) / GRAVITY`); the intro and win poses use the round timer. A few things the sim doesn't store (ticks spent crouching, whether hitstun came from a light or heavy hit, the landing squash) are remembered by the animator itself, counted in sim ticks like `Effects`.

**Why:** Players read the art, the sim reads the boxes, so the two must agree tick for tick: the drawing with the fist at full reach has to be on screen exactly on the attack's active ticks. Deriving the frame from sim fields makes that exact, and keeps everything render-only: the sim never learns which drawing is shown, so art can never change a hit or a checksum. The tests check that the reach drawings line up with `is_active` for every tick.

**Where:** `game/src/render/anim.rs` (`pose_for`, `timeline`, the `*_TIMELINE` constants and their tests); drawn by `draw_fighter` in `game/src/render/mod.rs`.

---

## Embedding assets with a build script

**What:** `game/build.rs` is a Cargo build script: a small Rust program Cargo compiles and runs *before* compiling the game. It lists every PNG and TTF under `game/assets/` and writes a Rust file into Cargo's `OUT_DIR` with one `include_bytes!("…/00.png")` per file. `render/assets.rs` pulls that file in with `include!`, so the image bytes end up inside the executable, and decodes them into textures once at startup.

**Why:** No file loading at runtime means no "assets folder not found" on desktop and no extra downloads to set up in the browser build, and it works the same on both. Because the list is generated, dropping a new frame into an animation folder needs no code change, only a rebuild (`cargo:rerun-if-changed` tells Cargo when to re-run the script). The cost is a bigger binary (under 1 MB of PNGs here).

**Where:** `game/build.rs`, `game/src/render/assets.rs` (`Assets::load`, `image`, `anim`).

---

## Procedural pixel art (scripts/art)

**What:** The fighter is drawn by code from a posed skeleton: each animation frame gives hip position, lean, and where the hands and feet should be. Two-bone inverse kinematics (the law of cosines) finds the elbows and knees, the limbs are filled as thick capsules into a half-resolution "material" map, and a shading pass adds a light top edge, a dark bottom edge, contour lines where a limb overlaps the body, and a 1-pixel outline. Scaling 2x with nearest-neighbour gives the chunky pixel look. Stage, effects, HUD and the font come from the same scripts.

**Why:** Art as code can be regenerated after any change in seconds, is checked against the hitboxes (the reach frames are posed to touch the hitbox edges), and keeps both colour schemes pixel-identical apart from colour, as the checklist requires.

**Where:** `scripts/art/fighter.py` (`two_bone`, `draw_pose`, `shade`, the animation functions), `scripts/art/gen_art.py`, `scripts/art/pixfont.py`.

---

## Snapshot ring buffer

**What:** A fixed number of slots (`max_prediction + 2`, so 10). The state saved just before frame `f` goes in slot `f % 10`, tagged with `f`. Saving frame 10 overwrites frame 0's slot. Asking for a frame whose slot now holds a different frame returns `None`.

**Why:** A rollback goes back at most `max_prediction` frames, so older states are never needed. A ring that overwrites the oldest slot keeps memory constant however long the match runs, with no allocation per frame. The frame tag stops a stale state from ever being loaded by mistake.

**Where:** `rewind/src/snapshot.rs` (`SnapshotBuffer::save`, `get`), used by `P2PSession::advance_frame` and `rollback_to`.

---

## The P2P session tick (prediction, rollback, resimulation)

**What:** Each `P2PSession::tick` does six steps:
1. Put the local input on frame `current + input_delay`.
2. Read the peer's messages. Their inputs go into the remote `InputQueue`, which notices if any of them differ from what we guessed.
3. If a guess was wrong, **roll back**: load the snapshot from the first wrong frame, then **resimulate** every frame from there to now with the corrected inputs. Each resimulated frame saves a fresh snapshot.
4. Send our unacknowledged inputs.
5. Snapshot, then advance the current frame, guessing the peer's input as "same as their last one" if it hasn't arrived.
6. Forget inputs no future rollback or resend can need.

**Why:** The local player's button presses take effect immediately (after the small input delay) instead of waiting a full network round trip. Most guesses are right, because players hold buttons for many frames, so rollbacks are rare and short. When one happens, it's invisible: the corrected frames are re-run within the same tick, before anything is drawn.

**Where:** `rewind/src/session/p2p.rs`: `tick`, `receive`, `rollback_to`, `send`, `advance_frame`, `forget_old_inputs`. Tests: `late_inputs_roll_back_and_converge`, `no_input_delay_rolls_back_and_still_matches`.

---

## Input delay

**What:** With `input_delay = 2`, a button pressed on tick `t` applies to frame `t + 2`. The first 2 frames get "nothing pressed" on both machines.

**Why:** The input is sent immediately but used 2 frames later, so it has about 33 ms to reach the peer before they need it. If it arrives in time, the peer never has to guess. The test `input_delay_hides_one_tick_of_lag` shows zero rollbacks when messages arrive one tick late. Too much delay makes the game feel sluggish, so 2 frames is a common compromise.

**Where:** `P2PSession::new` (filling frames `0..input_delay`) and step 1 of `tick`. `Config::input_delay`.

---

## Redundant sends and acknowledgements

**What:** Every message carries *all* local inputs the peer hasn't acknowledged yet (`start_frame` plus a list), and our acknowledgement of theirs (`ack_frame` = "I have all your inputs before this frame"). When their acknowledgement arrives, we stop resending those frames.

**Why:** Over a real network messages get lost. Instead of detecting loss and asking for a resend (an extra round trip), the next message simply repeats everything that might be missing. Inputs are tiny (2 bytes each), so this is cheap. Duplicates are harmless because the queue ignores frames it already has, and the acknowledgement only ever moves forward, so out-of-order messages can't move it back.

**Where:** `P2PSession::send` and `receive`; `PeerMsg::Inputs` in `rewind/src/protocol.rs`. Tests: `unacknowledged_inputs_are_resent_until_acked`, `lost_messages_are_covered_by_resending`, `duplicate_messages_are_harmless`.

---

## Testing that tests can fail (mutation check)

**What:** After writing the session tests, the rollback call was temporarily removed. 5 of the 10 tests failed, which proves those tests really depend on rollback working. The other 5 pass on purpose: lockstep with input delay never needs to guess, and the rest test messaging. The code was then restored.

**Why:** A test that passes whether or not the feature works proves nothing. Breaking the feature on purpose ("mutation testing", done here by hand) is the quickest way to know the tests are meaningful.

**Where:** the tests module of `rewind/src/session/p2p.rs`.

---

## Sharing in one thread: `Rc<RefCell<..>>`

**What:** The in-memory transport pair needs two objects to write into and read from the same queue. `Rc` (reference counted) lets several owners share one value; it's freed when the last owner goes away. `RefCell` allows changing that shared value: Rust checks the borrow rules at run time instead of compile time.

**Why:** Normal Rust ownership allows either one owner or shared read-only access. A mailbox both ends write to needs shared *and* mutable access. In a single thread, `Rc<RefCell<..>>` is the standard tool. Across threads it would be `Arc<Mutex<..>>`.

**Where:** `rewind/src/transport/memory.rs` (`Mailbox`, `memory_pair`).

---

## Implementing a library's trait for the game (G5)

**What:** `impl RollbackGame for GameState` tells rewind how to handle Riposte:
- `save` clones the whole state.
- `load` copies a saved one back.
- `advance` and `checksum` call the game's own functions.
- `Input` is the `u16` bitmask.

Rust allows this because `GameState` is our own type. The game crate depends on rewind through a *path dependency*, so Cargo builds it from the folder next door.

**Why:** It's the whole point of the frozen interface. rewind never learns what a fighter is, and the game never learns how rollback works. The four methods are the only contact between them. Using the entire `GameState` as the snapshot is the simplest correct choice: it's a few hundred bytes, so copying it 60+ times a second is nothing, and a full copy can't forget a field.

**A naming detail:** `GameState` already has inherent methods called `advance` and `checksum`, and the trait has methods with the same names. Inherent methods win in method-call syntax, so the trait impl calls `GameState::advance(self, ..)` explicitly to make it obvious which one runs.

**Where:** `game/src/sim/rollback.rs`.

---

## The sync test on the real game

**What:** `riposte_passes_the_sync_test` runs bot-vs-bot under rewind's `SyncTestSession`, rolling back 8 frames *every* tick, for 6000 frames on each of 4 seeds. Every frame is simulated once, then re-simulated 8 times from a loaded snapshot, and every checksum must match the first run.

**Why it's believable:**
- *Coverage is asserted, not assumed.* The test counts what each run went through: about 100 hits, 30–45 jumps, 4 KOs, 4 round ends and 1–2 full matches per seed. It fails if any of these is too low, so it can't pass by watching two idle fighters.
- *It catches real bugs.* Two bugs were planted on purpose and both failed with the exact frame. In the first, `load` forgot last tick's inputs: desync at frame 60, the first frame of the fight. In the second, a hidden global counter changed knockback: desync at frame 163, the first hit.

**Why it matters:** DESIGN.md requires this before any online work. A sync-test failure on one machine in milliseconds is far easier to debug than a desync between two laptops minutes into a match.

**Where:** `game/src/sim/rollback.rs` (tests: `Bot`, `Coverage`, `sync_test`). Run with `cargo test -p riposte-game sync -- --nocapture`.

---

## A deterministic fake network

**What:** `FakeNetwork` joins two `Transport` ends with one link per direction. Each message gets:
- an arrival time: `now + latency + random(0..=jitter)`;
- a `loss_percent` chance of being dropped, but never more than `max_loss_streak` in a row.

Time is a tick counter that only moves when the test calls `advance()`, once per game tick. Randomness is a seeded xorshift per direction, so **the same seed gives exactly the same delays and drops every run**.

**TCP vs UDP:** with `in_order: true`, a message can't overtake an earlier one (it waits behind it), like TCP and so WebSockets. With `in_order: false`, jitter can reorder messages, like UDP. Riposte will run over WebSockets, which never lose or reorder messages, only delay them. Testing loss and reordering anyway proves the resend and acknowledgement logic works under worse conditions.

**Why:** Real networks can't be repeated: a bug that shows up once in a thousand matches can't be reproduced. A fake network that's bad *on purpose and the same way every time* turns that into a test that fails the same way every run, naming the seed.

**Where:** `rewind/src/transport/fake.rs` (`FakeConfig`, `FakeNetwork`, `FakeTransport`, `Link::send`, `Link::receive`).

---

## Convergence testing across many seeds

**What:** `rewind/tests/convergence.rs` plays two `P2PSession`s against each other over 200 randomly chosen bad networks, each with random input scripts. After a quiet period at the end, so every input arrives, both games must be identical to each other *and* to a reference run where every input was known.

Across the 200 runs, about 30,000 frames were re-simulated and over 20,000 messages dropped, and the deepest single rollback was 5 frames. Breaking resending on purpose (sending only the newest input) makes all four tests fail.

`game/src/sim/rollback.rs` runs the same idea on Riposte itself (`riposte_converges_over_a_bad_network`).

**Integration tests:** files in a crate's `tests/` folder are compiled as separate programs that can only use the crate's *public* API, exactly as the game does. That's why `P2PSession`, `Config` and the fake network are re-exported from `rewind/src/lib.rs`.

**Where:** `rewind/tests/convergence.rs`, run with `cargo test -p rewind --test convergence -- --nocapture`.

---

## Stalling when too far ahead (R5)

**What:** A session may run at most `max_prediction` frames ahead of the last frame it has the peer's real input for. At that point `tick` doesn't advance the game: it still sends (so the peer gets our inputs and acknowledgements), reports `Event::Stalled`, and tries again next tick. The check is `P2PSession::too_far_ahead`: `current_frame >= confirmed_end + max_prediction`, where `confirmed_end` is the first frame whose remote input is unknown.

**Why:** A rollback can only go back as far as the snapshot ring reaches (`max_prediction + 2` frames). Without a limit, a lag spike makes the session guess further than that and the next correction has nothing to load. Stalling turns "crash or silent desync" into "a short pause". It also can't deadlock: a stalled side keeps sending, and the peer's acknowledgements and inputs are exactly what un-stall it.

**Detail worth knowing:** the local input is only scheduled on ticks that really advance. If a stalled tick also queued an input, the next tick would try to queue a second one for the same frame. The cost is that an input given on a stalled tick is dropped, which is fine because the game keeps passing the player's current buttons.

**Where:** `P2PSession::tick`, `too_far_ahead`, `confirmed_end` in `rewind/src/session/p2p.rs`; tests `stalls_instead_of_predicting_past_max_prediction` and (with 20 ticks of latency) `lag_beyond_max_prediction_stalls_instead_of_desyncing` in `rewind/tests/convergence.rs`.

---

## Checksum exchange and desync detection (R5)

**What:** Every `checksum_interval` (30) frames, each side hashes its game with `RollbackGame::checksum` and sends `PeerMsg::Checksum { frame, hash }`. When both hashes for a frame are in hand and differ, the session emits `Event::Desync { frame }`.

**Why only confirmed frames:** while a frame still rests on a guess about the peer's input, a late real input can roll it back and change the hash. Comparing then would flag a "desync" that is just normal rollback. So a checksum is sent, and compared, only once every input up to and including that frame is real (`frame < min(confirmed_end, current_frame)`). Each frame's hash is recorded as it is simulated, and re-simulation overwrites it, so the stored hash is always from the latest run of that frame.

**What a Desync means:** both machines had identical inputs but different states, so the game is not deterministic (a float, a HashMap iteration, a clock read). It is always a bug, never a network problem. The sync test (R2) is the offline way to catch the same class of bug.

**Limits:** checksums are sent once. A lost one just skips that check, and only the 16 newest unmatched checksums per side are kept so memory stays bounded.

**Where:** `record_checksum`, `exchange_checksums` and the `PeerMsg::Checksum` arm of `receive` in `rewind/src/session/p2p.rs`; tests `a_diverging_simulation_is_reported_as_desync`, `agreeing_peers_report_no_desync_even_with_rollbacks`, `checksum_for_an_unconfirmed_frame_is_not_compared_early`.

---

## The prediction envelope

**What:** A session can only roll back as far as its snapshot ring reaches: `max_prediction + 2` = 10 frames. With 2 frames of input delay, an input that's up to about 10 ticks late can still be absorbed without stalling. The random fake networks keep messages within that envelope; `lag_beyond_max_prediction_stalls_instead_of_desyncing` goes past it with 20 ticks of latency and shows the session stalls instead (see "Stalling when too far ahead").

**Where:** `SnapshotBuffer` capacity in `P2PSession::new`; `P2PSession::deepest_rollback`; the last test in `rewind/tests/convergence.rs`.

---

## One tick driver for every kind of match

**What:** `TickDriver` combines the fixed-timestep clock (G1) with the input buffering (G7): each frame it banks the elapsed time and ORs in the buttons seen, and returns how many ticks are due plus the inputs to use for them. Local, practice and online matches all use it.

**Why:** These two rules are easy to get subtly wrong and were already being copied: "run exactly 60 ticks a second" and "never lose a tap that falls between ticks". One small type with its own tests keeps every mode behaving the same.

**Where:** `game/src/driver.rs` (`TickDriver::frame`). Tests: `tap_on_a_frame_without_a_tick_is_kept`, `buffer_is_cleared_after_a_tick`.

---

## OnlineMatch: the game plus a rollback session over any transport

**What:** `OnlineMatch<T: Transport>` holds this machine's `GameState`, a `P2PSession` and the render effects. Each tick it hands the session the local input and lets the session advance the game, rolling back if it must. It is *generic over the transport*: the same code runs over the fake network now and over the WebSocket transport (R6) later.

**Why:** It's the integration point for I1. When R6 lands, joining a real match is: connect, wait for `MatchStart { player_index, seed }`, then `OnlineMatch::new(player_index, seed, websocket)`. Both machines must use the same seed and opposite player numbers, or their games differ from frame 0.

**Where:** `game/src/online.rs`. Test: `two_online_matches_stay_identical_without_lag` (with no lag nothing is ever guessed, so both machines agree on every single tick).

---

## Practice vs lag (two peers in one program)

**What:** A menu mode where you play the bot through the full netcode stack. Your `OnlineMatch` is player 0. The bot runs its *own* `P2PSession` and its *own* copy of the game as player 1. A `FakeNetwork` joins them, and its lag can be changed mid-match (keys 1-4, `FakeNetwork::set_config`). Each tick: you tick, the bot ticks, then one tick of network time passes.

**Why:** It's the only way to *feel* rollback before two real machines can connect. With high lag, a hit from the bot can appear a few frames late and the bot can "teleport" slightly: that's a rollback correcting a wrong guess. The bot is a *player*, not part of the simulation: it only produces inputs, which travel over the network like a human's, so it has no determinism rules.

**Where:** `game/src/practice.rs` (`Practice::update`, `LAG_PRESETS`), `game/src/bot.rs`. The presets stay within the prediction envelope; bigger lag needs R5's stall.

---

## Boxing a large enum variant

**What:** `App::Practice(Box<Practice>)`. A Rust enum is as big as its largest variant. `Practice` holds two games and two snapshot rings, so storing it inline would make every `App` value, even the menu, that big. `Box` puts it on the heap, and the enum only stores a pointer.

**Where:** `App` in `game/src/main.rs`.

---

## The WebSocket transport and the relay (R6)

**What:** `WebSocketTransport` (`rewind/src/transport/websocket.rs`) is a `Transport` that sends bytes to the relay server over one WebSocket. `send(bytes)` becomes `ClientMsg::Relay(bytes)`; relayed bytes from the other player arrive as `ServerMsg::Relay(bytes)` and come out of `poll`. The session never knows: it saw the same `send`/`poll` with the in-memory and fake transports.

**Lobby on the same connection:** the room handshake (`create_room`, `join_room`, then `RoomCreated` / `MatchStart` / `PeerLeft`) travels on the same socket, so the transport has a small second API, `lobby_events()`, for the menu. `pump` reads the socket once and sorts each message into the game stream or the lobby list, so neither side loses the other's messages.

**Non-blocking I/O:** a game loop must never wait for the network. `poll` calls `try_recv`, which returns at once with a message or nothing. On desktop that means putting the socket in non-blocking mode after the handshake; a read with nothing there fails with `WouldBlock`, which means "nothing yet", not an error. In the browser the socket is JavaScript's and queues messages for us.

**Held-back sends:** a browser WebSocket is "connecting" for a moment after `connect`, and sending then throws. The transport keeps outgoing messages in `unsent` and flushes them once `is_open()`.

**A trait for the socket:** `ByteSocket` (send, try_recv, is_open) is the only thing `WebSocketTransport` needs from a socket. Desktop and browser each provide one, and the unit tests provide a fake one, so the logic is tested with no network at all.

**Same enums, two crates:** `ClientMsg` / `ServerMsg` are defined in both `server/src/protocol.rs` and `websocket.rs`. bincode encodes an enum variant by its position, so they must stay in the same order. The test `server/tests/rewind_client.rs` runs the real transport against the real server, so any mismatch fails there.

**Where:** `rewind/src/transport/websocket.rs`; end-to-end in `server/tests/rewind_client.rs`.

---

## The online lobby and match screens (I1)

**What:** Online play goes **menu → `Lobby` → `OnlineScreen`**.
- **`Lobby`** holds Ram's `WebSocketTransport` and reads its `lobby_events()` each frame: `RoomCreated` shows the code, `MatchStart { player_index, seed }` hands the transport to a new `OnlineMatch`, and `PeerLeft` ends the lobby.
- **`OnlineScreen`** runs the match through a `TickDriver`. It records R5's `Desync` event (shown as a loud warning) and stops the match when the other player leaves.

**Timeouts as the only signal:** the relay answers a wrong or full room code by closing the connection, because the frozen `ServerMsg` has no error variant. So the joining side treats 5 seconds without `MatchStart` as "no such room (or it's full)".

**Reaching the transport mid-match:** `P2PSession` owns the transport, so the game can't call `lobby_events()` on it directly. `P2PSession::transport_mut()` (and `OnlineMatch::transport_mut()`) give that access, and the game uses it only to read the relay's `PeerLeft`. Sending or polling through it would bypass the session.

**Where:** `game/src/netplay.rs` (`Lobby`, `OnlineScreen`, `server_url`), the `App::Lobby` / `App::Online` arms in `game/src/main.rs`, and `P2PSession::transport_mut` in `rewind/src/session/p2p.rs`. Tests use a fake `ByteSocket` that plays the relay's side.

---

## WebAssembly imports and `--allow-undefined`

**What:** A `.wasm` module can *import* functions that the page provides. quad-net declares `extern "C" { fn ws_connect(...); }` with no Rust body, because the body is JavaScript in `mq_js_bundle.js`. Current Rust treats such missing functions as a link error for `wasm32-unknown-unknown`. The linker flag `--allow-undefined` turns them into imports from the `env` module instead, and the page fills them in when it loads the `.wasm`.

**Why it appeared only now:** until the menu called `WebSocketTransport::connect`, no code used quad-net. The linker threw that code away, and the missing functions never mattered.

**Where:** `.cargo/config.toml`. You can check the result by listing a module's imports with `WebAssembly.Module.imports(...)` in Node.

---

## Debugging the browser bundle: two bugs between JavaScript and Rust

Online play in the browser failed twice, both times *between* the two halves of quad-net. Each was fixed with a one-line change in `web/mq_js_bundle.js`.

1. **`consume_js_object is not defined`:** the bundle was minified. The minifier wrapped the `sapp_jsutils` helpers in a private scope and renamed them (`js_object` became `t`, `consume_js_object` became `a`). The quad-net code still calls them by their original global names. Fix: publish them as `window.js_object`, `window.consume_js_object` and `window.get_js_object`.
2. **Nothing ever received:** quad-net 0.1.2's JavaScript stores each binary message as `{text: 0, data: bytes}`, but its Rust side copies bytes from the *outer object*, which has no length. So every binary message arrived empty, and all our messages are binary (bincode). Fix: push the byte array itself, with `text = 0` set on it.

**How they were found:** two headless Chromium browsers played a real match through the real relay. The first bug showed up as a JavaScript error in the console. For the second, the server log showed "room created" and "match started" while the game screens never changed, which pointed at the receive path.

**Where:** `web/mq_js_bundle.js` (both changes are marked with comments or sit next to Ram's R6 fix).

---

## Same-origin deployment (one app, one address)

**What:** The relay serves the browser build (`server/src/web.rs`) when `WEB_DIR` is set, so the game page and the `/ws` WebSocket come from the *same address*. The page then connects to wherever it was loaded from: `ws://host/ws`, or `wss://host/ws` when the page is `https://`.

**Why:** Browsers refuse a WebSocket to `ws://` from an `https://` page ("mixed content"), and hosting platforms provide HTTPS by terminating TLS in front of the app. If the page and relay share an address, the platform's HTTPS covers both with no certificates in our code, and there is no address to configure.

**Safe file serving:** `web.rs` serves exactly four fixed URLs mapped to three fixed files, never a path from the request. Tests check `/secret.txt` and `/../secret.txt` in the same folder return 404 (path traversal is when `..` in a URL escapes the folder). The `.wasm` must be sent as `application/wasm`, or the browser won't compile it while streaming.

**Where:** `server/src/web.rs`, `app_with_web` in `server/src/lib.rs`, `server/tests/web.rs`, the `Dockerfile` (`ENV WEB_DIR=/app/web`).

---

## Handing data from a web page to WebAssembly

**What:** WASM can't read the page's URL. The page adds two functions to the module's *imports*: `riposte_server_url_len()` and `riposte_server_url_copy(ptr, capacity)`. The Rust side declares them in `extern "C"`, asks for the length, allocates a buffer of that size, and has the page write the UTF-8 bytes straight into the game's memory at `ptr`. macroquad's `miniquad_add_plugin` is the hook for adding imports.

**Why this way:** it needs no new crates. The alternatives (a wasm-bindgen toolchain, or baking the address in at build time) add tooling or force a rebuild per deployment.

**The choice itself** is plain JavaScript in `web/index.html` (`relayUrl`): `?server=` override first, then `ws://<host>:8080/ws` if the page is on some other dev port, else the page's own address. It was tested against fake locations including an HTTPS page.

**Where:** `web/index.html` (`relayUrl`, the plugin), `server_url()` for wasm in `game/src/netplay.rs`. The `unsafe` block there is needed because calling a foreign function is unchecked; its comment says why it's sound.

---

## Noticing that your own connection is gone

**What:** two different "gone" events. `PeerLeft` is the relay telling us *the other player* left. `ConnectionLost` is our own socket closing, so the relay itself is unreachable and can't tell us anything. Without the second, a rollback session just waits: it can't advance more than a few frames past the last input it got, so the game sits in a stall forever, which looks like a hang.

**How it's detected:** a TCP socket finds out it's dead when a read or write fails. If the other end's process dies, its OS sends a reset and the next read fails at once (what the LAN test saw). If a cable or Wi-Fi link silently disappears, nothing arrives at all, and the OS only gives up after its retransmission timeout. A game-level heartbeat (no message from the peer for N seconds) would catch that case sooner; we don't have one yet.

**Where:** `ByteSocket::is_closed` and the end of `WebSocketTransport::pump` in `rewind/src/transport/websocket.rs`; `connection_lost` in `OnlineScreen` and the `ConnectionLost` arm of `Lobby::update` in `game/src/netplay.rs`.
