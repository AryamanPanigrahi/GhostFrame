# Exercises

Small changes that prove you understand the code. Do them on a scratch branch; each says **what to change**, **what you should see**, and **what it teaches**. `cargo test --workspace` should be green again at the end of each one (or the exercise tells you why it won't be).

Read `docs/frame-walkthrough.md` first.

---

## A. Warm-ups (15 minutes each)

### A1. Change how fast fighters walk
**Change:** `WALK_FORWARD_SPEED` in `game/src/sim/fighter.rs` (e.g. `4 * SUBPIXELS` → `6 * SUBPIXELS`).
**You should see:** `cargo test -p riposte-game` passes: no test pins walk speed except `walking_forward_is_faster_than_back`, which compares against the constants. Play it: `cargo run -p riposte-game`.
**Teaches:** speeds are integer subpixels per tick (256 per pixel); there is no frame-rate dependence anywhere.

### A2. Change the jump
**Change:** `JUMP_VELOCITY` (e.g. `-15` → `-12`).
**You should see:** `jump_leaves_floor_and_lands_after_jump_ticks` **fails**: it pins the exact arc (41 ticks, 157.5 px peak). Work out the new numbers from the formula in the test's comment (`height after n ticks = 3840n - 96n(n-1)` for the original values) and update the test.
**Teaches:** with integers the arc is exact, so a test can assert exact numbers instead of "roughly".

### A3. Change a hit's damage and check the frame advantage
**Change:** `HEAVY_ATTACK.hitstun` in `game/src/sim/attack.rs` from 20 to 24.
**You should see:** `frame_advantage_on_hit` fails (heavy was −4 on hit). Predict the new number before you run it, then fix the test.
**Teaches:** frame advantage is `defender's hitstun − the attacker's remaining recovery`; the numbers in `attack.rs` *are* the game design.

---

## B. Rollback experiments

### B1. Feel input delay
**Change:** in `game/src/online.rs`, `OnlineMatch::new` builds the session with `Config::default()`. Try `Config { input_delay: 0, ..Config::default() }`, then `input_delay: 5`.
**You should see:** run **Practice vs lag** (main menu, second item) and press `3` or `4` for lag. With delay 0 the overlay's "rolled back N frames" climbs fast and the bot "snaps"; with 5 it barely rolls back but your own inputs feel slower.
**Teaches:** the trade: input delay hides latency at the cost of responsiveness; rollback hides it at the cost of re-simulation.

### B2. Break the resending
**Change:** in `rewind/src/session/p2p.rs`, `send`, make it send only the newest input instead of everything unacknowledged (`let start = end - 1;`).
**You should see:** `cargo test -p rewind --test convergence` fails on every test. Undo it.
**Teaches:** why every message repeats all unacknowledged inputs, and why a test that fails when you break the feature is worth more than one that always passes.

### B3. Push past the prediction limit
**Change:** in `rewind/tests/convergence.rs`, find the "lag beyond `max_prediction`" test and raise the latency.
**You should see:** it still passes: the session **stalls** (waits) instead of guessing further, so it can never need a snapshot that's gone. Look at `too_far_ahead` in `p2p.rs` and at `Event::Stalled`.
**Teaches:** the stall is what turns "too much lag" from a crash or a desync into a short pause.

---

## C. Determinism (the important one)

### C1. Hide state from the sync test
**Change:** in `Fighter::take_hit` (`game/src/sim/fighter.rs`), add a counter that lives **outside** `GameState`:

```rust
static HITS: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
let n = HITS.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % 2;
self.vx = from_facing as i32 * (attack.knockback + n);
```

**You should see:** `cargo test -p riposte-game sync` **fails** with "desync at frame N", naming the frame and both checksums.
**Why:** the sync test replays old frames from a saved state. The saved state doesn't contain the counter, so the replay differs from the first run. This is the exact bug that would make two real machines drift apart.

### C2. A float is *not* caught: find out why
**Change:** in `Fighter::step`, replace `self.x += self.vx;` with `self.x = ((self.x as f32) + (self.vx as f32)) as i32;`.
**You should see:** the sync test **passes**. (Reason it out first: the result is still a pure function of the saved state, and the same binary on one machine rounds the same way every time.)
**The catch:** the *rule* is "no floats in `sim/`" because two **different** machines, compilers or WASM can round differently, and nothing on one machine can check that. That is why the browser-vs-desktop check in I2 matters, and why the checksum exchange (R5) exists as a runtime watchdog.
**Teaches:** what the sync test proves (replay is faithful), and what it cannot (cross-platform float agreement).

### C3. Forget to hash a field
**Change:** add `pub combo: u16` to `Fighter` (and `combo: 0` in `Fighter::new`).
**You should see:** a **compile error** in `Fighter::hash_into`: the destructuring `let Fighter { x, y, ... } = *self;` doesn't mention `combo`. Fix it by hashing the new field.
**Teaches:** a field missing from the checksum would be invisible to desync detection; the compiler makes forgetting impossible.

---

## D. New features (an hour or more each)

### D1. Add a third attack
**Do:**
1. Add `MEDIUM_ATTACK` in `game/src/sim/attack.rs` (your own frame data).
2. Add `Action::Medium` in `fighter.rs`, its number in `Action::tag`, and a case in `current_attack`.
3. Pick a button: `u16` has bits 6–15 free. Add the constant in `sim/mod.rs` and a key in `input.rs`.
4. Start it in `ground_control` like `LIGHT`/`HEAVY`.
**Done when:** a new test shows its first active frame, and `cargo test -p riposte-game sync` passes (add the new button to the bot in `bot.rs` so the test actually uses it).

### D2. Add a round timer
**Do:** add a countdown to `Round` (in `sim/round.rs`); when it reaches 0 during `Fight`, the fighter with more health wins the round (a tie is a draw). Hash the new field.
**Done when:** a test covers a time-out win, a tie, and a KO before time-out; the sync test still passes.

### D3. Add blocking
**Do:** holding back (away from the opponent) while actionable and grounded blocks: a block takes no damage and a short blockstun instead of hitstun. Decide what happens to the light attack's "hits high" property.
**Done when:** tests cover blocking a light and a heavy, what happens to blocking in the air (your decision, written down in a comment), and the sync test still passes.
**Teaches:** every mechanic is a state-machine edit plus frame data; the netcode needs no changes.

---

## E. Explain it out loud (interview practice)

Answer each in a minute or two, without looking:

1. Why do both machines run the **whole** game instead of sending game state?
2. What does the "just pressed" check need, and why is `GameState.inputs` part of the saved state?
3. Walk through a rollback: when does it happen, what is loaded, what is re-run, what does the player see?
4. Why is the guess "same as their last input" a good one?
5. What are input delay, `max_prediction`, and the snapshot ring size, and how do they relate?
6. Why does the relay not need to understand the messages?
7. What does the sync test prove, and what can it not prove (see C2)?
8. Why are positions integers? What would go wrong with `f32`?
9. Why are both hits checked before either is applied?
10. What bug did the minified `mq_js_bundle.js` cause in the browser, and how was it found? (`LEARNING.md`, last entries of I1.)
