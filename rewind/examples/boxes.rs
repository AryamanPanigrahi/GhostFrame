//! A toy game for testing rewind without Riposte: two boxes that move around
//! a walled arena. It implements `RollbackGame` and runs under
//! `SyncTestSession`, rolling back 8 frames on every tick.
//!
//!     cargo run -p rewind --example boxes
//!     cargo run -p rewind --example boxes -- --broken
//!
//! `--broken` hides a tick counter outside the saved state, the kind of
//! mistake that would desync an online match, to show the sync test catching
//! it.

use std::process::ExitCode;

use rewind::session::synctest::SyncTestSession;
use rewind::RollbackGame;

const LEFT: u8 = 1 << 0;
const RIGHT: u8 = 1 << 1;
const UP: u8 = 1 << 2;
const DOWN: u8 = 1 << 3;

/// Arena size and speed, in whole units. Integers only, like Riposte's sim.
const ARENA: i32 = 1000;
const SPEED: i32 = 7;

const FRAMES: u32 = 600; // 10 seconds at 60 ticks/s
const CHECK_DISTANCE: usize = 8; // the online session's max_prediction

#[derive(Clone, Copy, Debug, PartialEq)]
struct BoxState {
    x: i32,
    y: i32,
}

#[derive(Clone, Debug, PartialEq)]
struct Boxes {
    frame: u32,
    boxes: [BoxState; 2],
    /// Only used with `--broken`. Deliberately left out of `save`/`load`.
    broken: bool,
    hidden_ticks: u32,
}

impl Boxes {
    fn new(broken: bool) -> Boxes {
        Boxes {
            frame: 0,
            boxes: [BoxState { x: 250, y: 500 }, BoxState { x: 750, y: 500 }],
            broken,
            hidden_ticks: 0,
        }
    }
}

impl RollbackGame for Boxes {
    type Input = u8;
    type State = (u32, [BoxState; 2]);

    fn save(&self) -> Self::State {
        (self.frame, self.boxes)
    }

    fn load(&mut self, state: &Self::State) {
        (self.frame, self.boxes) = *state;
    }

    fn advance(&mut self, inputs: [u8; 2]) {
        for (b, input) in self.boxes.iter_mut().zip(inputs) {
            if input & LEFT != 0 {
                b.x -= SPEED;
            }
            if input & RIGHT != 0 {
                b.x += SPEED;
            }
            if input & UP != 0 {
                b.y -= SPEED;
            }
            if input & DOWN != 0 {
                b.y += SPEED;
            }
            b.x = b.x.clamp(0, ARENA);
            b.y = b.y.clamp(0, ARENA);
        }
        if self.broken {
            // Survives a rollback because `load` doesn't reset it, so the
            // replay sees a different value than the first run did.
            self.hidden_ticks += 1;
            if self.hidden_ticks.is_multiple_of(100) {
                self.boxes[0].x = (self.boxes[0].x + 1).min(ARENA);
            }
        }
        self.frame += 1;
    }

    fn checksum(&self) -> u64 {
        let mut hash = Fnv1a::new();
        hash.write_u32(self.frame);
        for b in &self.boxes {
            hash.write_u32(b.x as u32);
            hash.write_u32(b.y as u32);
        }
        hash.finish()
    }
}

/// FNV-1a: for each byte, XOR it in, then multiply by a fixed prime. Tiny,
/// and identical on every platform, unlike std's `DefaultHasher`, which is
/// allowed to change between Rust versions.
struct Fnv1a(u64);

impl Fnv1a {
    fn new() -> Fnv1a {
        Fnv1a(0xcbf2_9ce4_8422_2325)
    }

    fn write_u32(&mut self, value: u32) {
        for byte in value.to_le_bytes() {
            self.0 ^= byte as u64;
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }

    fn finish(&self) -> u64 {
        self.0
    }
}

/// xorshift: a seeded pseudo-random generator, so the "players" press the
/// same buttons on every run.
fn next_random(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

fn main() -> ExitCode {
    let broken = std::env::args().any(|arg| arg == "--broken");
    let mut game = Boxes::new(broken);
    let mut session = SyncTestSession::new(CHECK_DISTANCE);
    let mut rng = 0x2545_f491_4f6c_dd1d;

    for _ in 0..FRAMES {
        // Change buttons every so often, like a human would, rather than
        // every frame.
        if session.current_frame().is_multiple_of(10) {
            session.add_local_input(0, (next_random(&mut rng) >> 60) as u8);
            session.add_local_input(1, (next_random(&mut rng) >> 60) as u8);
        }
        if let Err(error) = session.tick(&mut game) {
            eprintln!("sync test FAILED: {error}");
            return ExitCode::FAILURE;
        }
    }

    println!("sync test passed: {FRAMES} frames, rolled back {CHECK_DISTANCE} frames every tick");
    println!(
        "final positions: {:?} {:?}, checksum {:#018x}",
        game.boxes[0],
        game.boxes[1],
        game.checksum()
    );
    ExitCode::SUCCESS
}
