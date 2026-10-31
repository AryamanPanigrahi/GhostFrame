//! Two transports joined back to back in memory: what one sends, the other
//! receives on its next poll. No network, no loss, no delay, so session
//! tests are fast and fully repeatable.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use crate::api::Transport;

/// Messages waiting to be read, shared between the sender and receiver.
/// `Rc<RefCell<..>>` lets both ends own the same queue in a single thread:
/// `Rc` shares ownership, `RefCell` allows mutation through that sharing.
type Mailbox = Rc<RefCell<VecDeque<Vec<u8>>>>;

pub struct MemoryTransport {
    outbox: Mailbox,
    inbox: Mailbox,
}

/// Two connected ends: `a.send` arrives at `b.poll` and vice versa.
pub fn memory_pair() -> (MemoryTransport, MemoryTransport) {
    let a_to_b: Mailbox = Rc::default();
    let b_to_a: Mailbox = Rc::default();
    let a = MemoryTransport {
        outbox: a_to_b.clone(),
        inbox: b_to_a.clone(),
    };
    let b = MemoryTransport {
        outbox: b_to_a,
        inbox: a_to_b,
    };
    (a, b)
}

impl Transport for MemoryTransport {
    fn send(&mut self, bytes: &[u8]) {
        self.outbox.borrow_mut().push_back(bytes.to_vec());
    }

    fn poll(&mut self) -> Vec<Vec<u8>> {
        self.inbox.borrow_mut().drain(..).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_cross_over_in_order() {
        let (mut a, mut b) = memory_pair();
        a.send(b"one");
        a.send(b"two");
        b.send(b"back");
        assert_eq!(b.poll(), vec![b"one".to_vec(), b"two".to_vec()]);
        assert_eq!(a.poll(), vec![b"back".to_vec()]);
    }

    #[test]
    fn poll_drains_the_inbox() {
        let (mut a, mut b) = memory_pair();
        a.send(b"x");
        assert_eq!(b.poll().len(), 1);
        assert!(b.poll().is_empty());
    }

    #[test]
    fn a_end_does_not_receive_its_own_messages() {
        let (mut a, _b) = memory_pair();
        a.send(b"x");
        assert!(a.poll().is_empty());
    }
}
