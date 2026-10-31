//! The online session: one local player, one remote player, inputs over a
//! `Transport`, and rollback when a guess about the remote player was wrong
//! (DESIGN.md section 5).
//!
//! Each `tick`:
//! 1. Schedule the local input `input_delay` frames ahead.
//! 2. Receive the peer's inputs and acknowledgements.
//! 3. If any earlier guess about the peer was wrong, load the snapshot from
//!    the first wrong frame and re-run every frame since with the real
//!    inputs.
//! 4. Send every local input the peer hasn't acknowledged yet.
//! 5. Save a snapshot and advance the current frame, guessing the peer's
//!    input if it hasn't arrived.
//! 6. Forget inputs that can never be needed again.
//!
//! Receiving before sending means the acknowledgement we send is as fresh
//! as possible. Checksums, desync events and stalling when too far ahead are
//! R5; until then nothing stops this side running more than `max_prediction`
//! frames ahead of the peer (see `rollback_to`).

use crate::api::{Event, Frame, RollbackGame, Transport};
use crate::input_queue::{AddOutcome, InputQueue};
use crate::protocol::PeerMsg;
use crate::snapshot::SnapshotBuffer;

/// Tuning knobs, with the defaults from DESIGN.md section 5.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    /// Local inputs take effect this many frames after they're pressed.
    /// A little delay gives the peer's input time to arrive, so fewer
    /// guesses are needed and fewer rollbacks happen.
    pub input_delay: Frame,
    /// The furthest the session may run ahead of the peer's last known
    /// input, and so the furthest it may ever have to roll back.
    pub max_prediction: Frame,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            input_delay: 2,
            max_prediction: 8,
        }
    }
}

pub struct P2PSession<G: RollbackGame, T: Transport> {
    config: Config,
    transport: T,
    /// 0 or 1: which player this machine controls.
    local_player: usize,
    /// One queue per player, indexed by player number (not local/remote),
    /// so `queues[p]` is always player `p`'s inputs.
    queues: [InputQueue<G::Input>; 2],
    /// States from just before each recent frame, for rolling back.
    snapshots: SnapshotBuffer<G::State>,
    /// The next frame to simulate.
    current_frame: Frame,
    /// The local input given since the last tick. Kept until replaced, so a
    /// caller that skips `add_local_input` for a tick repeats the last one.
    local_input: G::Input,
    /// The first local frame the peer has not confirmed receiving. Every
    /// local input from here on is sent again in each message.
    peer_ack: Frame,
    /// Total frames re-simulated by rollbacks, for tests and debug overlays.
    rollback_frames: u64,
    /// The most frames a single rollback has re-simulated.
    deepest_rollback: Frame,
}

impl<G: RollbackGame, T: Transport> P2PSession<G, T> {
    pub fn new(local_player: usize, transport: T, config: Config) -> P2PSession<G, T> {
        assert!(local_player < 2, "local_player must be 0 or 1");
        let mut queues = [InputQueue::new(), InputQueue::new()];
        // With an input delay of D, the first local input lands on frame D,
        // so frames 0..D need an input from somewhere: "nothing pressed" on
        // both machines. The peer fills its own queue the same way and sends
        // these to us like any other input.
        for frame in 0..config.input_delay {
            queues[local_player].add_input(frame, G::Input::default());
        }
        P2PSession {
            config,
            transport,
            local_player,
            queues,
            // +2: the frame being rolled back to, and the current one.
            snapshots: SnapshotBuffer::new(config.max_prediction as usize + 2),
            current_frame: 0,
            local_input: G::Input::default(),
            peer_ack: 0,
            rollback_frames: 0,
            deepest_rollback: 0,
        }
    }

    /// The local player's input for the next tick.
    pub fn add_local_input(&mut self, input: G::Input) {
        self.local_input = input;
    }

    /// Run one tick (see the module comment for the steps). The game ends up
    /// one frame further on, with any wrong guesses corrected.
    pub fn tick(&mut self, game: &mut G) -> Vec<Event> {
        let local_frame = self.current_frame + self.config.input_delay;
        let outcome = self.queues[self.local_player].add_input(local_frame, self.local_input);
        assert_eq!(outcome, AddOutcome::Added, "local input out of step");

        self.receive();
        if let Some(first_wrong) = self.queues[self.remote_player()].first_incorrect_frame() {
            self.rollback_to(first_wrong, game);
        }
        self.send();
        self.advance_frame(game);
        self.forget_old_inputs();
        Vec::new()
    }

    /// The next frame `tick` will simulate.
    pub fn current_frame(&self) -> Frame {
        self.current_frame
    }

    pub fn local_player(&self) -> usize {
        self.local_player
    }

    /// Total frames re-simulated because a guess was wrong.
    pub fn rollback_frames(&self) -> u64 {
        self.rollback_frames
    }

    /// The most frames any single rollback has re-simulated. Must stay
    /// within `max_prediction`; the network tests (R4) check it.
    pub fn deepest_rollback(&self) -> Frame {
        self.deepest_rollback
    }

    fn remote_player(&self) -> usize {
        1 - self.local_player
    }

    /// Read every message the peer sent since the last tick.
    fn receive(&mut self) {
        let remote = self.remote_player();
        for bytes in self.transport.poll() {
            // A message we can't decode is ignored: the peer resends every
            // unacknowledged input anyway, so nothing is lost for good.
            let Ok(msg) = bincode::deserialize::<PeerMsg<G::Input>>(&bytes) else {
                continue;
            };
            match msg {
                PeerMsg::Inputs {
                    start_frame,
                    inputs,
                    ack_frame,
                } => {
                    for (i, input) in inputs.into_iter().enumerate() {
                        let frame = start_frame + i as Frame;
                        // AlreadyHad is normal (inputs are resent until
                        // acknowledged). A gap means frames are missing in
                        // between, so the rest can't be used yet; they'll
                        // come again in a later message.
                        if let AddOutcome::Gap { .. } = self.queues[remote].add_input(frame, input)
                        {
                            break;
                        }
                    }
                    // Messages can arrive out of order, so never move the
                    // acknowledgement backwards.
                    self.peer_ack = self.peer_ack.max(ack_frame);
                }
                // Checksum exchange and desync detection are R5.
                PeerMsg::Checksum { .. } => {}
            }
        }
    }

    /// Go back to just before `first_wrong` and re-run every frame up to
    /// now. The remote queue now holds the real inputs where we'd guessed,
    /// so the re-run produces what both machines should have seen.
    fn rollback_to(&mut self, first_wrong: Frame, game: &mut G) {
        let Some(state) = self.snapshots.get(first_wrong) else {
            panic!(
                "can't roll back to frame {first_wrong} from frame {}: further than \
                 max_prediction ({}). Stalling when too far ahead (R5) prevents this.",
                self.current_frame, self.config.max_prediction
            );
        };
        game.load(state);
        // The old guesses are settled; the re-run makes fresh ones for any
        // frames whose input still hasn't arrived.
        self.queues[self.remote_player()].reset_prediction();

        let resume_at = self.current_frame;
        self.deepest_rollback = self.deepest_rollback.max(resume_at - first_wrong);
        self.current_frame = first_wrong;
        while self.current_frame < resume_at {
            self.advance_frame(game);
            self.rollback_frames += 1;
        }
    }

    /// Send every local input from the peer's acknowledgement onward, plus
    /// our own acknowledgement of their inputs.
    fn send(&mut self) {
        let local = self.local_player;
        let end = self.queues[local].next_frame();
        let start = self.peer_ack.min(end);
        let mut inputs = Vec::with_capacity((end - start) as usize);
        for frame in start..end {
            inputs.push(self.queues[local].input(frame).0);
        }
        let msg = PeerMsg::Inputs {
            start_frame: start,
            inputs,
            ack_frame: self.queues[self.remote_player()].next_frame(),
        };
        // Serializing plain integers and Vecs into a Vec<u8> can't fail.
        let bytes = bincode::serialize(&msg).expect("PeerMsg always serializes");
        self.transport.send(&bytes);
    }

    /// Snapshot, then simulate `current_frame` with the best inputs we have:
    /// confirmed where known, guessed where not.
    fn advance_frame(&mut self, game: &mut G) {
        let frame = self.current_frame;
        self.snapshots.save(frame, game.save());
        let inputs = [self.queues[0].input(frame).0, self.queues[1].input(frame).0];
        game.advance(inputs);
        self.current_frame += 1;
    }

    /// Drop inputs no future rollback or resend can need, so memory stays
    /// bounded in a long match.
    ///
    /// A rollback always starts at a frame where the remote input was a
    /// guess, which is at or after the remote queue's next frame. Frames
    /// before that, and before the current frame, are never re-simulated.
    /// Local inputs are also kept until the peer acknowledges them.
    fn forget_old_inputs(&mut self) {
        let remote = self.remote_player();
        let keep_from = self.queues[remote].next_frame().min(self.current_frame);
        self.queues[remote].discard_before(keep_from);
        self.queues[self.local_player].discard_before(keep_from.min(self.peer_ack));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::memory::{memory_pair, MemoryTransport};

    /// A game whose state depends on every input *and the order of players*
    /// (different weights), so swapped players or a wrong input on any frame
    /// changes the final value.
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

    type Session<T> = P2PSession<Tally, T>;

    /// Inputs that change every few frames, like a player pressing buttons,
    /// so guesses are sometimes wrong.
    fn script(seed: u16, ticks: usize) -> Vec<u16> {
        (0..ticks)
            .map(|t| ((t as u16 / 3).wrapping_mul(seed) ^ seed) % 17)
            .collect()
    }

    /// What the game should be after `frames` frames when both players'
    /// inputs are known: each input applies `delay` frames after its tick,
    /// and the first `delay` frames get default inputs.
    fn reference(p0: &[u16], p1: &[u16], delay: u32, frames: u32) -> Tally {
        let input_at = |script: &[u16], frame: u32| -> u16 {
            if frame < delay {
                0
            } else {
                script.get((frame - delay) as usize).copied().unwrap_or(0)
            }
        };
        let mut game = Tally::default();
        for frame in 0..frames {
            game.advance([input_at(p0, frame), input_at(p1, frame)]);
        }
        game
    }

    fn pair(config: Config) -> (Session<MemoryTransport>, Session<MemoryTransport>) {
        let (ta, tb) = memory_pair();
        (Session::new(0, ta, config), Session::new(1, tb, config))
    }

    /// Run both peers through the scripts, `burst` ticks of A then `burst`
    /// ticks of B, so each sees the other's inputs up to `burst` ticks late.
    /// Then idle with no input long enough for every input to be confirmed.
    fn run<T: Transport>(
        a: &mut Session<T>,
        b: &mut Session<T>,
        p0: &[u16],
        p1: &[u16],
        burst: usize,
    ) -> (Tally, Tally) {
        let mut ga = Tally::default();
        let mut gb = Tally::default();
        let idle = 20;
        let total = p0.len() + idle;
        let input = |script: &[u16], t: usize| script.get(t).copied().unwrap_or(0);
        let mut t = 0;
        while t < total {
            for k in t..(t + burst).min(total) {
                a.add_local_input(input(p0, k));
                a.tick(&mut ga);
            }
            for k in t..(t + burst).min(total) {
                b.add_local_input(input(p1, k));
                b.tick(&mut gb);
            }
            t += burst;
        }
        (ga, gb)
    }

    #[test]
    fn lockstep_peers_match_the_reference() {
        let (mut a, mut b) = pair(Config::default());
        let (p0, p1) = (script(5, 120), script(11, 120));
        let (ga, gb) = run(&mut a, &mut b, &p0, &p1, 1);
        assert_eq!(ga, gb);
        assert_eq!(ga, reference(&p0, &p1, 2, ga.frame));
    }

    #[test]
    fn input_delay_hides_one_tick_of_lag() {
        // In lockstep each side's input arrives one tick after it's sent.
        // With 2 frames of input delay it's always there before it's
        // needed, so nothing is ever guessed and nothing rolls back.
        let (mut a, mut b) = pair(Config::default());
        run(&mut a, &mut b, &script(5, 120), &script(11, 120), 1);
        assert_eq!(a.rollback_frames(), 0);
        assert_eq!(b.rollback_frames(), 0);
    }

    #[test]
    fn no_input_delay_rolls_back_and_still_matches() {
        let config = Config {
            input_delay: 0,
            ..Config::default()
        };
        let (mut a, mut b) = pair(config);
        let (p0, p1) = (script(5, 120), script(11, 120));
        let (ga, gb) = run(&mut a, &mut b, &p0, &p1, 1);
        assert!(
            a.rollback_frames() > 0,
            "player 0 should have guessed wrong"
        );
        assert_eq!(ga, gb);
        assert_eq!(ga, reference(&p0, &p1, 0, ga.frame));
    }

    #[test]
    fn late_inputs_roll_back_and_converge() {
        // Bursts of 4 ticks. A always runs its burst first, so it reaches
        // frames whose B input hasn't arrived and has to guess. B then runs
        // with all of A's inputs already in, so only A ever rolls back.
        let (mut a, mut b) = pair(Config::default());
        let (p0, p1) = (script(5, 200), script(11, 200));
        let (ga, gb) = run(&mut a, &mut b, &p0, &p1, 4);
        assert!(a.rollback_frames() > 0);
        assert_eq!(b.rollback_frames(), 0);
        assert_eq!(ga, gb);
        assert_eq!(ga, reference(&p0, &p1, 2, ga.frame));
    }

    #[test]
    fn constant_input_is_always_guessed_right() {
        // "Same as last time" is the guess, so a player holding one button
        // never causes a rollback, even with late delivery.
        let (mut a, mut b) = pair(Config::default());
        let held = vec![9u16; 150];
        let (ga, gb) = run(&mut a, &mut b, &held, &held, 4);
        assert_eq!(ga, gb);
        // Only the switch from "nothing" to 9, and from 9 back to nothing
        // when the script ends, can be guessed wrong.
        assert!(a.rollback_frames() <= 2 * 8);
    }

    /// Wraps a transport and drops every other message it sends.
    struct DropEveryOther {
        inner: MemoryTransport,
        sent: u32,
    }

    impl Transport for DropEveryOther {
        fn send(&mut self, bytes: &[u8]) {
            self.sent += 1;
            if self.sent.is_multiple_of(2) {
                self.inner.send(bytes);
            }
        }
        fn poll(&mut self) -> Vec<Vec<u8>> {
            self.inner.poll()
        }
    }

    #[test]
    fn lost_messages_are_covered_by_resending() {
        let (ta, tb) = memory_pair();
        let wrap = |inner| DropEveryOther { inner, sent: 0 };
        let mut a = Session::new(0, wrap(ta), Config::default());
        let mut b = Session::new(1, wrap(tb), Config::default());
        let (p0, p1) = (script(3, 150), script(7, 150));
        let (ga, gb) = run(&mut a, &mut b, &p0, &p1, 2);
        assert_eq!(ga, gb);
        assert_eq!(ga, reference(&p0, &p1, 2, ga.frame));
    }

    /// Wraps a transport and delivers every message twice.
    struct SendTwice(MemoryTransport);

    impl Transport for SendTwice {
        fn send(&mut self, bytes: &[u8]) {
            self.0.send(bytes);
            self.0.send(bytes);
        }
        fn poll(&mut self) -> Vec<Vec<u8>> {
            self.0.poll()
        }
    }

    #[test]
    fn duplicate_messages_are_harmless() {
        let (ta, tb) = memory_pair();
        let mut a = Session::new(0, SendTwice(ta), Config::default());
        let mut b = Session::new(1, SendTwice(tb), Config::default());
        let (p0, p1) = (script(3, 100), script(7, 100));
        let (ga, gb) = run(&mut a, &mut b, &p0, &p1, 3);
        assert_eq!(ga, gb);
        assert_eq!(ga, reference(&p0, &p1, 2, ga.frame));
    }

    fn decode(bytes: &[u8]) -> PeerMsg<u16> {
        bincode::deserialize(bytes).unwrap()
    }

    #[test]
    fn unacknowledged_inputs_are_resent_until_acked() {
        // Drive one session by hand from the other end of the wire.
        let (ta, mut wire) = memory_pair();
        let mut a: Session<MemoryTransport> = Session::new(0, ta, Config::default());
        let mut game = Tally::default();
        for input in [1, 2, 3] {
            a.add_local_input(input);
            a.tick(&mut game);
        }
        let sent = wire.poll();
        // Nothing acknowledged yet: the third message carries frames 0..5
        // (2 delay frames of "nothing", then 1, 2, 3).
        assert_eq!(
            decode(&sent[2]),
            PeerMsg::Inputs {
                start_frame: 0,
                inputs: vec![0, 0, 1, 2, 3],
                ack_frame: 0,
            }
        );

        // The peer says it has frames 0..4; only frame 4 onward comes again.
        let ack = PeerMsg::<u16>::Inputs {
            start_frame: 0,
            inputs: vec![],
            ack_frame: 4,
        };
        wire.send(&bincode::serialize(&ack).unwrap());
        a.add_local_input(4);
        a.tick(&mut game);
        let sent = wire.poll();
        assert_eq!(
            decode(&sent[0]),
            PeerMsg::Inputs {
                start_frame: 4,
                inputs: vec![3, 4],
                ack_frame: 0,
            }
        );
    }

    #[test]
    fn garbage_bytes_are_ignored() {
        let (ta, mut wire) = memory_pair();
        let mut a: Session<MemoryTransport> = Session::new(0, ta, Config::default());
        let mut game = Tally::default();
        wire.send(&[0xff, 0x00, 0x13]);
        a.tick(&mut game);
        assert_eq!(a.current_frame(), 1);
    }

    #[test]
    #[should_panic(expected = "further than max_prediction")]
    fn rolling_back_past_the_snapshots_is_caught() {
        // Run 20 frames on guesses with no stall (that's R5), then deliver
        // a different input for frame 0: the snapshot is long gone.
        let (ta, mut wire) = memory_pair();
        let mut a: Session<MemoryTransport> = Session::new(0, ta, Config::default());
        let mut game = Tally::default();
        for _ in 0..20 {
            a.tick(&mut game);
        }
        let late = PeerMsg::<u16>::Inputs {
            start_frame: 0,
            inputs: vec![5],
            ack_frame: 0,
        };
        wire.send(&bincode::serialize(&late).unwrap());
        a.tick(&mut game);
    }
}
