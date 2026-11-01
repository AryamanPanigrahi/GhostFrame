//! Convergence tests (R4): two `P2PSession`s over a fake network with
//! latency, jitter, reordering and packet loss, across many seeds. However
//! messy the network, once every input has arrived both games must be
//! identical, and identical to a run where every input was known up front.
//!
//! The random networks stay within what the session can absorb without
//! stalling: a message is never late by more than
//! `latency + jitter + max_loss_streak` ticks. The last test shows what
//! happens beyond that: the session stalls (R5) rather than desyncing.

use rewind::{Config, Event, FakeConfig, FakeNetwork, FakeTransport, P2PSession, RollbackGame};

/// A tiny game whose state depends on every input and on which player sent
/// it, so any wrong, missing or swapped input changes the result.
#[derive(Clone, Debug, Default, PartialEq)]
struct Tally {
    frame: u32,
    value: u64,
}

impl RollbackGame for Tally {
    type Input = u16;
    type State = Tally;

    fn save(&self) -> Tally {
        self.clone()
    }
    fn load(&mut self, state: &Tally) {
        *self = state.clone();
    }
    fn advance(&mut self, inputs: [u16; 2]) {
        let mixed = inputs[0] as u64 * 7 + inputs[1] as u64 * 13 + 1;
        self.value = self.value.wrapping_mul(31).wrapping_add(mixed);
        self.frame += 1;
    }
    fn checksum(&self) -> u64 {
        self.value ^ self.frame as u64
    }
}

/// xorshift64, for picking scripts and network settings from a seed.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// Button-mash-like inputs: hold something for 1-8 ticks, then change.
fn script(rng: &mut Rng, ticks: usize) -> Vec<u16> {
    let mut out = Vec::with_capacity(ticks);
    while out.len() < ticks {
        let input = rng.below(64) as u16;
        for _ in 0..1 + rng.below(8) {
            out.push(input);
        }
    }
    out.truncate(ticks);
    out
}

/// The game after `frames` frames when every input is known: each input
/// lands `delay` frames after its tick, and the first `delay` frames get 0.
fn reference(p0: &[u16], p1: &[u16], delay: u32, frames: u32) -> Tally {
    let at = |s: &[u16], f: u32| {
        if f < delay {
            0
        } else {
            s.get((f - delay) as usize).copied().unwrap_or(0)
        }
    };
    let mut game = Tally::default();
    for f in 0..frames {
        game.advance([at(p0, f), at(p1, f)]);
    }
    game
}

struct Outcome {
    a: Tally,
    b: Tally,
    deepest_rollback: u32,
    rollback_frames: u64,
    dropped: u32,
}

/// Both peers tick once per network tick, playing their scripts, then idle
/// on input 0 for a while so every input arrives and every guess is fixed.
fn play(net_config: FakeConfig, session: Config, p0: &[u16], p1: &[u16]) -> Outcome {
    let (net, ta, tb) = FakeNetwork::new(net_config);
    let mut a: P2PSession<Tally, FakeTransport> = P2PSession::new(0, ta, session);
    let mut b: P2PSession<Tally, FakeTransport> = P2PSession::new(1, tb, session);
    let (mut ga, mut gb) = (Tally::default(), Tally::default());

    let idle = 60;
    for t in 0..p0.len() + idle {
        a.add_local_input(p0.get(t).copied().unwrap_or(0));
        b.add_local_input(p1.get(t).copied().unwrap_or(0));
        a.tick(&mut ga);
        b.tick(&mut gb);
        net.advance();
    }
    let stats = net.stats();
    Outcome {
        a: ga,
        b: gb,
        deepest_rollback: a.deepest_rollback().max(b.deepest_rollback()),
        rollback_frames: a.rollback_frames() + b.rollback_frames(),
        dropped: stats[0].dropped + stats[1].dropped,
    }
}

/// Random network settings inside the envelope: a message is at most
/// 3 + 2 + 2 = 7 ticks late.
fn random_network(rng: &mut Rng, seed: u64) -> FakeConfig {
    FakeConfig {
        latency: rng.below(4) as u32,
        jitter: rng.below(3) as u32,
        loss_percent: rng.below(31) as u32,
        max_loss_streak: rng.below(3) as u32,
        in_order: rng.below(2) == 0,
        seed,
    }
}

#[test]
fn converges_across_200_random_networks() {
    let session = Config::default();
    let mut total_rollback_frames = 0;
    let mut total_dropped = 0;
    let mut deepest = 0;
    for seed in 1..=200u64 {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let net = random_network(&mut rng, seed);
        let p0 = script(&mut rng, 600);
        let p1 = script(&mut rng, 600);
        let out = play(net, session, &p0, &p1);

        assert_eq!(out.a, out.b, "seed {seed}, {net:?}: peers differ");
        let expected = reference(&p0, &p1, session.input_delay, out.a.frame);
        assert_eq!(out.a, expected, "seed {seed}, {net:?}: wrong result");
        assert!(out.deepest_rollback <= session.max_prediction);

        total_rollback_frames += out.rollback_frames;
        total_dropped += out.dropped;
        deepest = deepest.max(out.deepest_rollback);
    }
    // Prove the networks were actually bad: lots of guessing, rolling back
    // and lost messages happened across the seeds.
    println!("rollback frames {total_rollback_frames}, dropped {total_dropped}, deepest {deepest}");
    assert!(total_rollback_frames > 10_000);
    assert!(total_dropped > 5_000);
    assert!(deepest >= 4);
}

#[test]
fn the_worst_network_in_the_envelope_still_converges() {
    // Every setting at its maximum, with no input delay, so every frame of
    // lag becomes a guess.
    let net = FakeConfig {
        latency: 3,
        jitter: 2,
        loss_percent: 30,
        max_loss_streak: 2,
        in_order: false,
        seed: 1234,
    };
    let session = Config {
        input_delay: 0,
        ..Config::default()
    };
    let mut rng = Rng(77);
    let (p0, p1) = (script(&mut rng, 1200), script(&mut rng, 1200));
    let out = play(net, session, &p0, &p1);
    assert_eq!(out.a, out.b);
    assert_eq!(out.a, reference(&p0, &p1, 0, out.a.frame));
    assert!(out.deepest_rollback <= session.max_prediction);
}

#[test]
fn input_delay_absorbs_small_latency() {
    // 1 tick of latency, perfectly steady: with the default 2 frames of
    // input delay every input arrives before it's needed.
    let net = FakeConfig {
        latency: 1,
        ..FakeConfig::default()
    };
    let mut rng = Rng(5);
    let (p0, p1) = (script(&mut rng, 600), script(&mut rng, 600));
    let out = play(net, Config::default(), &p0, &p1);
    assert_eq!(out.a, out.b);
    assert_eq!(out.rollback_frames, 0);
}

#[test]
fn lag_beyond_max_prediction_stalls_instead_of_desyncing() {
    // 20 ticks of latency minus 2 frames of input delay would put each side
    // ~18 frames ahead on guesses, past what the snapshot ring holds. With
    // R5's stall each side stops at max_prediction frames ahead instead.
    // Inputs given on stalled ticks are dropped, so there is no reference
    // run to compare against; the checksums show the two sides never
    // disagreed.
    let net = FakeConfig {
        latency: 20,
        ..FakeConfig::default()
    };
    let session = Config {
        checksum_interval: 10,
        ..Config::default()
    };
    let (network, ta, tb) = FakeNetwork::new(net);
    let mut a: P2PSession<Tally, FakeTransport> = P2PSession::new(0, ta, session);
    let mut b: P2PSession<Tally, FakeTransport> = P2PSession::new(1, tb, session);
    let (mut ga, mut gb) = (Tally::default(), Tally::default());
    let mut rng = Rng(9);
    let (p0, p1) = (script(&mut rng, 600), script(&mut rng, 600));

    let mut events = Vec::new();
    for t in 0..600 {
        a.add_local_input(p0[t]);
        b.add_local_input(p1[t]);
        events.extend(a.tick(&mut ga));
        events.extend(b.tick(&mut gb));
        network.advance();
    }
    assert!(events.contains(&Event::Stalled));
    assert!(a.stalled_ticks() > 0 && b.stalled_ticks() > 0);
    assert!(!events.iter().any(|e| matches!(e, Event::Desync { .. })));
    assert!(a.deepest_rollback() <= session.max_prediction);
    assert!(b.deepest_rollback() <= session.max_prediction);
    // It still makes progress: stalling is waiting, not deadlock.
    assert!(a.current_frame() > 200 && b.current_frame() > 200);
}
