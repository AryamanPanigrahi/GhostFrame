//! A fake network for tests: two `Transport` ends joined by links that add
//! latency, jitter and packet loss, all driven by a seeded generator and a
//! tick clock instead of real time.
//!
//! Real networks are slow and messy but also unrepeatable. This one is
//! messy *on purpose and the same way every run*: a failing seed fails
//! forever, so it can be debugged.
//!
//! Time only moves when the test calls `FakeNetwork::advance` (once per game
//! tick), so a delay of 3 means "3 ticks", about 50 ms at 60 ticks/s.

use std::cell::RefCell;
use std::rc::Rc;

use crate::api::Transport;

/// How bad the network is. Delays are in ticks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FakeConfig {
    /// Every message takes at least this long to arrive.
    pub latency: u32,
    /// Plus a random extra delay of 0..=jitter.
    pub jitter: u32,
    /// Chance (0-100) that a message is dropped.
    pub loss_percent: u32,
    /// Never drop more than this many messages in a row. Without a stall
    /// (R5), a long run of losses can push the session past
    /// `max_prediction`; this keeps tests inside what the session can
    /// handle on its own.
    pub max_loss_streak: u32,
    /// Deliver messages in the order they were sent, like TCP (and so
    /// WebSockets). When false, jitter can reorder them, like UDP.
    pub in_order: bool,
    pub seed: u64,
}

impl Default for FakeConfig {
    /// A perfect network: no delay, no loss.
    fn default() -> FakeConfig {
        FakeConfig {
            latency: 0,
            jitter: 0,
            loss_percent: 0,
            max_loss_streak: 0,
            in_order: true,
            seed: 1,
        }
    }
}

/// Counts for one direction, to check a test really was lossy or delayed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LinkStats {
    pub sent: u32,
    pub dropped: u32,
    pub delivered: u32,
}

/// A message on its way: when it arrives, and a sequence number so that
/// messages due on the same tick come out in the order they were sent.
struct InFlight {
    arrives_at: u32,
    seq: u64,
    bytes: Vec<u8>,
}

/// One direction of the network.
struct Link {
    in_flight: Vec<InFlight>,
    rng: u64,
    loss_streak: u32,
    /// The latest arrival time scheduled so far, for in-order delivery.
    last_arrival: u32,
    next_seq: u64,
    stats: LinkStats,
}

impl Link {
    fn new(seed: u64) -> Link {
        Link {
            in_flight: Vec::new(),
            // xorshift gets stuck at 0, so make sure the seed isn't.
            rng: seed | 1,
            loss_streak: 0,
            last_arrival: 0,
            next_seq: 0,
            stats: LinkStats::default(),
        }
    }

    /// xorshift64, the same generator the game uses (DESIGN.md section 7).
    fn random(&mut self) -> u64 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng = x;
        x
    }

    fn send(&mut self, bytes: &[u8], now: u32, config: &FakeConfig) {
        self.stats.sent += 1;
        let lose = (self.random() % 100) < config.loss_percent as u64;
        if lose && self.loss_streak < config.max_loss_streak {
            self.loss_streak += 1;
            self.stats.dropped += 1;
            return;
        }
        self.loss_streak = 0;

        let jitter = self.random() % (config.jitter as u64 + 1);
        let mut arrives_at = now + config.latency + jitter as u32;
        if config.in_order {
            // Like TCP: a message can't overtake one sent before it, so it
            // waits behind it instead.
            arrives_at = arrives_at.max(self.last_arrival);
        }
        self.last_arrival = self.last_arrival.max(arrives_at);
        self.in_flight.push(InFlight {
            arrives_at,
            seq: self.next_seq,
            bytes: bytes.to_vec(),
        });
        self.next_seq += 1;
    }

    /// Everything due by `now`, earliest first (ties in sending order).
    fn receive(&mut self, now: u32) -> Vec<Vec<u8>> {
        let mut due = Vec::new();
        let mut waiting = Vec::new();
        for msg in self.in_flight.drain(..) {
            if msg.arrives_at <= now {
                due.push(msg);
            } else {
                waiting.push(msg);
            }
        }
        self.in_flight = waiting;
        due.sort_by_key(|msg| (msg.arrives_at, msg.seq));
        self.stats.delivered += due.len() as u32;
        due.into_iter().map(|msg| msg.bytes).collect()
    }
}

struct Shared {
    config: FakeConfig,
    now: u32,
    /// links[0] carries end A → end B, links[1] carries B → A.
    links: [Link; 2],
}

/// The clock and the statistics. The two `FakeTransport` ends are handed to
/// the sessions; the test keeps this to move time forward.
pub struct FakeNetwork {
    shared: Rc<RefCell<Shared>>,
}

pub struct FakeTransport {
    shared: Rc<RefCell<Shared>>,
    /// The link this end sends on (0 for A, 1 for B); it receives on the other.
    outgoing: usize,
}

impl FakeNetwork {
    /// A network and its two ends, A and B.
    pub fn new(config: FakeConfig) -> (FakeNetwork, FakeTransport, FakeTransport) {
        let shared = Rc::new(RefCell::new(Shared {
            config,
            now: 0,
            // Different seeds per direction, so A→B and B→A aren't mirror
            // images of each other.
            links: [
                Link::new(config.seed ^ 0xA5A5_A5A5),
                Link::new(config.seed ^ 0x5A5A_5A5A_0000),
            ],
        }));
        let a = FakeTransport {
            shared: shared.clone(),
            outgoing: 0,
        };
        let b = FakeTransport {
            shared: shared.clone(),
            outgoing: 1,
        };
        (FakeNetwork { shared }, a, b)
    }

    /// Change how bad the network is from now on (e.g. a lag setting in a
    /// practice mode). Messages already on their way keep their arrival
    /// times; in-order delivery still holds, because new messages never
    /// arrive before ones already scheduled.
    pub fn set_config(&self, config: FakeConfig) {
        self.shared.borrow_mut().config = config;
    }

    /// One tick of time passes.
    pub fn advance(&self) {
        self.shared.borrow_mut().now += 1;
    }

    /// Statistics for A → B and B → A.
    pub fn stats(&self) -> [LinkStats; 2] {
        let shared = self.shared.borrow();
        [shared.links[0].stats, shared.links[1].stats]
    }
}

impl Transport for FakeTransport {
    fn send(&mut self, bytes: &[u8]) {
        let mut shared = self.shared.borrow_mut();
        let (now, config) = (shared.now, shared.config);
        shared.links[self.outgoing].send(bytes, now, &config);
    }

    fn poll(&mut self) -> Vec<Vec<u8>> {
        let mut shared = self.shared.borrow_mut();
        let now = shared.now;
        shared.links[1 - self.outgoing].receive(now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Send one numbered message per tick from A for `ticks` ticks, and
    /// return what B received on each tick.
    fn run(config: FakeConfig, ticks: u8) -> Vec<Vec<u8>> {
        let (net, mut a, mut b) = FakeNetwork::new(config);
        let mut received = Vec::new();
        for t in 0..ticks {
            a.send(&[t]);
            received.push(b.poll().into_iter().map(|m| m[0]).collect());
            net.advance();
        }
        received
    }

    #[test]
    fn perfect_network_delivers_immediately() {
        let got = run(FakeConfig::default(), 3);
        assert_eq!(got, vec![vec![0], vec![1], vec![2]]);
    }

    #[test]
    fn latency_delays_by_whole_ticks() {
        let config = FakeConfig {
            latency: 2,
            ..FakeConfig::default()
        };
        let got = run(config, 4);
        assert_eq!(got, vec![vec![], vec![], vec![0], vec![1]]);
    }

    #[test]
    fn loss_drops_some_but_never_more_than_the_streak() {
        let config = FakeConfig {
            loss_percent: 50,
            max_loss_streak: 2,
            seed: 7,
            ..FakeConfig::default()
        };
        let (net, mut a, mut b) = FakeNetwork::new(config);
        let mut last_seen: i32 = -1;
        for t in 0..200u8 {
            a.send(&[t]);
            for m in b.poll() {
                // At most 2 lost in a row means at most 2 missing numbers.
                assert!(m[0] as i32 - last_seen <= 3, "lost more than 2 in a row");
                last_seen = m[0] as i32;
            }
            net.advance();
        }
        let stats = net.stats()[0];
        assert_eq!(stats.sent, 200);
        assert!(stats.dropped > 40, "50% loss should drop plenty: {stats:?}");
        assert_eq!(stats.dropped + stats.delivered, 200);
    }

    #[test]
    fn in_order_never_reorders_even_with_jitter() {
        let config = FakeConfig {
            latency: 1,
            jitter: 5,
            in_order: true,
            seed: 3,
            ..FakeConfig::default()
        };
        let all: Vec<u8> = run(config, 100).into_iter().flatten().collect();
        let mut sorted = all.clone();
        sorted.sort();
        assert_eq!(all, sorted);
    }

    #[test]
    fn out_of_order_mode_can_reorder() {
        let config = FakeConfig {
            latency: 1,
            jitter: 5,
            in_order: false,
            seed: 3,
            ..FakeConfig::default()
        };
        let all: Vec<u8> = run(config, 100).into_iter().flatten().collect();
        let mut sorted = all.clone();
        sorted.sort();
        assert_ne!(all, sorted, "jitter should have swapped some messages");
    }

    #[test]
    fn same_seed_same_network() {
        let config = FakeConfig {
            latency: 1,
            jitter: 3,
            loss_percent: 20,
            max_loss_streak: 3,
            in_order: false,
            seed: 99,
        };
        assert_eq!(run(config, 120), run(config, 120));
        let other = FakeConfig {
            seed: 100,
            ..config
        };
        assert_ne!(run(config, 120), run(other, 120));
    }

    #[test]
    fn directions_are_independent() {
        let (net, mut a, mut b) = FakeNetwork::new(FakeConfig::default());
        a.send(b"to b");
        b.send(b"to a");
        assert_eq!(b.poll(), vec![b"to b".to_vec()]);
        assert_eq!(a.poll(), vec![b"to a".to_vec()]);
        assert_eq!(net.stats()[0].sent, 1);
        assert_eq!(net.stats()[1].sent, 1);
    }
}
