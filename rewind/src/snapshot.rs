//! Ring buffer of saved game states, one per recent frame.
//!
//! To roll back to frame F the session needs the state as it was just
//! before F was simulated. A rollback never goes further back than
//! `max_prediction` frames, so only that many recent states are ever
//! needed: a fixed-size ring that overwrites the oldest slot is enough, and
//! memory stays constant however long the match runs.

use crate::api::Frame;

pub struct SnapshotBuffer<S> {
    /// Slot `frame % len` holds the state saved just before `frame`, tagged
    /// with that frame so a stale slot is never mistaken for the one asked
    /// for.
    slots: Vec<Option<(Frame, S)>>,
}

impl<S> SnapshotBuffer<S> {
    pub fn new(capacity: usize) -> SnapshotBuffer<S> {
        assert!(capacity > 0, "snapshot buffer needs at least one slot");
        let mut slots = Vec::with_capacity(capacity);
        for _ in 0..capacity {
            slots.push(None);
        }
        SnapshotBuffer { slots }
    }

    /// Store the state from just before `frame`, overwriting whatever was
    /// `capacity` frames ago.
    pub fn save(&mut self, frame: Frame, state: S) {
        let index = self.index(frame);
        self.slots[index] = Some((frame, state));
    }

    /// The state saved just before `frame`, if it's still in the buffer.
    pub fn get(&self, frame: Frame) -> Option<&S> {
        match &self.slots[self.index(frame)] {
            Some((saved, state)) if *saved == frame => Some(state),
            _ => None,
        }
    }

    fn index(&self, frame: Frame) -> usize {
        frame as usize % self.slots.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_state_comes_back() {
        let mut buf = SnapshotBuffer::new(4);
        buf.save(0, "zero");
        buf.save(1, "one");
        assert_eq!(buf.get(0), Some(&"zero"));
        assert_eq!(buf.get(1), Some(&"one"));
    }

    #[test]
    fn missing_frame_is_none() {
        let buf: SnapshotBuffer<u8> = SnapshotBuffer::new(4);
        assert_eq!(buf.get(3), None);
    }

    #[test]
    fn old_frames_are_overwritten() {
        let mut buf = SnapshotBuffer::new(4);
        for frame in 0..6 {
            buf.save(frame, frame * 10);
        }
        // Frames 0 and 1 shared slots with 4 and 5 and are gone.
        assert_eq!(buf.get(0), None);
        assert_eq!(buf.get(1), None);
        assert_eq!(buf.get(2), Some(&20));
        assert_eq!(buf.get(5), Some(&50));
    }

    #[test]
    fn resaving_a_frame_replaces_it() {
        let mut buf = SnapshotBuffer::new(4);
        buf.save(2, 1);
        buf.save(2, 2);
        assert_eq!(buf.get(2), Some(&2));
    }
}
