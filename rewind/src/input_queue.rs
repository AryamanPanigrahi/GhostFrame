//! One player's inputs, frame by frame, plus the bookkeeping rollback needs:
//! which frames we had to guess, and the earliest guess that turned out wrong.
//!
//! A session keeps one `InputQueue` per player. Confirmed inputs arrive in
//! frame order (the local player's from the keyboard, the remote player's
//! from the network). When the game needs a frame we don't have yet, the
//! queue *predicts* it as "same as the last confirmed input". When the real
//! input for that frame arrives, the queue compares it with the guess and
//! remembers the first frame where they differ, so the session knows how far
//! back to roll.

use std::collections::VecDeque;

use crate::api::Frame;

/// Whether an input handed out by `InputQueue::input` was real or a guess.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputStatus {
    Confirmed,
    Predicted,
}

/// What `InputQueue::add_input` did with an input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AddOutcome {
    /// It was the next frame we were waiting for, and is now confirmed.
    Added,
    /// We already had this frame. Normal: peers resend every unacknowledged
    /// input in each message, so most frames arrive more than once.
    AlreadyHad,
    /// It skipped ahead of the next frame we need. The input was not stored,
    /// because a hole in the middle would leave a frame we can never confirm.
    Gap { expected: Frame },
}

/// A run of consecutive frames that were handed out as predictions and are
/// still unconfirmed. Every frame in it was predicted as `value`.
#[derive(Clone, Copy, Debug)]
struct Prediction<I> {
    first: Frame,
    last: Frame,
    value: I,
}

pub struct InputQueue<I> {
    /// Confirmed inputs. `confirmed[0]` is the input for `start_frame`.
    confirmed: VecDeque<I>,
    /// Frame of `confirmed[0]`. Grows as old frames are discarded.
    start_frame: Frame,
    /// The newest confirmed input, kept even after it is discarded, because
    /// it is what we predict with.
    last_input: I,
    prediction: Option<Prediction<I>>,
    first_incorrect: Option<Frame>,
}

impl<I: Copy + PartialEq + Default> Default for InputQueue<I> {
    fn default() -> InputQueue<I> {
        InputQueue::new()
    }
}

impl<I: Copy + PartialEq + Default> InputQueue<I> {
    /// An empty queue whose first expected frame is 0.
    pub fn new() -> InputQueue<I> {
        InputQueue {
            confirmed: VecDeque::new(),
            start_frame: 0,
            // Before anyone has pressed anything, "no buttons" is the best guess.
            last_input: I::default(),
            prediction: None,
            first_incorrect: None,
        }
    }

    /// The next frame we need a confirmed input for.
    pub fn next_frame(&self) -> Frame {
        self.start_frame + self.confirmed.len() as Frame
    }

    /// The newest frame with a confirmed input, or `None` if there is none yet.
    pub fn last_confirmed_frame(&self) -> Option<Frame> {
        self.next_frame().checked_sub(1)
    }

    /// Store the real input for `frame`. Inputs must arrive in order; repeats
    /// are ignored. If `frame` had been predicted and the guess was wrong,
    /// `first_incorrect_frame` will report it.
    pub fn add_input(&mut self, frame: Frame, input: I) -> AddOutcome {
        let expected = self.next_frame();
        if frame < expected {
            return AddOutcome::AlreadyHad;
        }
        if frame > expected {
            return AddOutcome::Gap { expected };
        }

        self.confirmed.push_back(input);
        self.last_input = input;

        // Was this frame one we guessed? Inputs arrive in order, so if it was,
        // it is the first frame of the predicted run.
        if let Some(mut prediction) = self.prediction {
            if prediction.first == frame {
                // Keep only the earliest wrong frame: rolling back to it
                // resimulates everything after it anyway.
                if input != prediction.value && self.first_incorrect.is_none() {
                    self.first_incorrect = Some(frame);
                }
                if prediction.first == prediction.last {
                    self.prediction = None;
                } else {
                    prediction.first += 1;
                    self.prediction = Some(prediction);
                }
            }
        }
        AddOutcome::Added
    }

    /// The input to simulate `frame` with: the real one if we have it,
    /// otherwise a prediction (the last confirmed input).
    ///
    /// Panics if `frame` was already discarded. A session only asks for frames
    /// it might still roll back to, so that would be a bug in the session.
    pub fn input(&mut self, frame: Frame) -> (I, InputStatus) {
        assert!(
            frame >= self.start_frame,
            "input for frame {frame} was discarded (oldest kept is {})",
            self.start_frame
        );

        let index = (frame - self.start_frame) as usize;
        if let Some(&input) = self.confirmed.get(index) {
            return (input, InputStatus::Confirmed);
        }

        let value = self.last_input;
        match &mut self.prediction {
            // Extend the run. A session asks for frames in increasing order,
            // so `frame` is at or past the end of the run.
            Some(prediction) => prediction.last = prediction.last.max(frame),
            None => {
                self.prediction = Some(Prediction {
                    first: frame,
                    last: frame,
                    value,
                })
            }
        }
        (value, InputStatus::Predicted)
    }

    /// The earliest frame whose prediction turned out wrong since the last
    /// `reset_prediction`. The session rolls back to this frame.
    pub fn first_incorrect_frame(&self) -> Option<Frame> {
        self.first_incorrect
    }

    /// Forget all outstanding predictions. The session calls this when it
    /// rolls back: it is about to resimulate those frames and will ask for
    /// (and re-predict) them again with the newest confirmed input.
    pub fn reset_prediction(&mut self) {
        self.prediction = None;
        self.first_incorrect = None;
    }

    /// Drop confirmed inputs for frames before `frame`, once no rollback can
    /// reach them. Without this the queue would grow for the whole match.
    pub fn discard_before(&mut self, frame: Frame) {
        // Never discard past what we've confirmed, or `next_frame` would jump.
        let frame = frame.min(self.next_frame());
        while self.start_frame < frame {
            self.confirmed.pop_front();
            self.start_frame += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type Queue = InputQueue<u16>;

    #[test]
    fn starts_empty() {
        let queue = Queue::new();
        assert_eq!(queue.next_frame(), 0);
        assert_eq!(queue.last_confirmed_frame(), None);
        assert_eq!(queue.first_incorrect_frame(), None);
    }

    #[test]
    fn confirmed_inputs_come_back_in_order() {
        let mut queue = Queue::new();
        assert_eq!(queue.add_input(0, 5), AddOutcome::Added);
        assert_eq!(queue.add_input(1, 7), AddOutcome::Added);
        assert_eq!(queue.last_confirmed_frame(), Some(1));
        assert_eq!(queue.input(0), (5, InputStatus::Confirmed));
        assert_eq!(queue.input(1), (7, InputStatus::Confirmed));
    }

    #[test]
    fn repeated_input_is_ignored() {
        let mut queue = Queue::new();
        queue.add_input(0, 5);
        // A resend, even with a different value, can't change a confirmed frame.
        assert_eq!(queue.add_input(0, 9), AddOutcome::AlreadyHad);
        assert_eq!(queue.input(0), (5, InputStatus::Confirmed));
        assert_eq!(queue.next_frame(), 1);
    }

    #[test]
    fn gap_is_rejected() {
        let mut queue = Queue::new();
        queue.add_input(0, 5);
        assert_eq!(queue.add_input(2, 9), AddOutcome::Gap { expected: 1 });
        assert_eq!(queue.next_frame(), 1);
    }

    #[test]
    fn predicts_default_before_any_input() {
        let mut queue = Queue::new();
        assert_eq!(queue.input(0), (0, InputStatus::Predicted));
    }

    #[test]
    fn predicts_last_confirmed_input() {
        let mut queue = Queue::new();
        queue.add_input(0, 5);
        queue.add_input(1, 7);
        assert_eq!(queue.input(2), (7, InputStatus::Predicted));
        assert_eq!(queue.input(3), (7, InputStatus::Predicted));
    }

    #[test]
    fn correct_prediction_is_not_flagged() {
        let mut queue = Queue::new();
        queue.add_input(0, 5);
        queue.input(1);
        queue.input(2);
        queue.add_input(1, 5);
        queue.add_input(2, 5);
        assert_eq!(queue.first_incorrect_frame(), None);
        // Both guesses resolved, so a fresh question starts a fresh run.
        assert_eq!(queue.input(3), (5, InputStatus::Predicted));
    }

    #[test]
    fn wrong_prediction_is_flagged() {
        let mut queue = Queue::new();
        queue.add_input(0, 5);
        queue.input(1);
        queue.input(2);
        queue.input(3);
        queue.add_input(1, 5);
        queue.add_input(2, 9);
        assert_eq!(queue.first_incorrect_frame(), Some(2));
    }

    #[test]
    fn earliest_wrong_frame_is_kept() {
        let mut queue = Queue::new();
        queue.input(0);
        queue.input(1);
        queue.add_input(0, 4);
        queue.add_input(1, 8);
        assert_eq!(queue.first_incorrect_frame(), Some(0));
    }

    #[test]
    fn unpredicted_frames_are_never_flagged() {
        let mut queue = Queue::new();
        queue.add_input(0, 5);
        queue.add_input(1, 9);
        assert_eq!(queue.first_incorrect_frame(), None);
    }

    #[test]
    fn reset_clears_predictions_and_the_flag() {
        let mut queue = Queue::new();
        queue.input(0);
        queue.input(1);
        queue.add_input(0, 4);
        queue.reset_prediction();
        assert_eq!(queue.first_incorrect_frame(), None);
        // Frame 1 was predicted before the reset; confirming it now must not
        // compare against that stale guess.
        queue.add_input(1, 8);
        assert_eq!(queue.first_incorrect_frame(), None);
    }

    #[test]
    fn repredicts_with_newest_input_after_reset() {
        let mut queue = Queue::new();
        queue.input(0);
        queue.input(1);
        queue.add_input(0, 4);
        queue.reset_prediction();
        assert_eq!(queue.input(1), (4, InputStatus::Predicted));
        queue.add_input(1, 4);
        assert_eq!(queue.first_incorrect_frame(), None);
    }

    #[test]
    fn discard_keeps_newer_frames_and_prediction() {
        let mut queue = Queue::new();
        for frame in 0..5 {
            queue.add_input(frame, frame as u16);
        }
        queue.discard_before(3);
        assert_eq!(queue.input(3), (3, InputStatus::Confirmed));
        assert_eq!(queue.input(4), (4, InputStatus::Confirmed));
        assert_eq!(queue.next_frame(), 5);
        assert_eq!(queue.input(5), (4, InputStatus::Predicted));
    }

    #[test]
    fn discard_never_passes_confirmed_frames() {
        let mut queue = Queue::new();
        queue.add_input(0, 1);
        queue.discard_before(10);
        assert_eq!(queue.next_frame(), 1);
        assert_eq!(queue.add_input(1, 2), AddOutcome::Added);
    }

    #[test]
    #[should_panic(expected = "discarded")]
    fn asking_for_a_discarded_frame_panics() {
        let mut queue = Queue::new();
        queue.add_input(0, 1);
        queue.add_input(1, 2);
        queue.discard_before(2);
        queue.input(0);
    }
}
