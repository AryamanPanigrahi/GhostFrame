//! Ways to move bytes between peers. Each implements `Transport`, so a
//! session runs unchanged over any of them.

pub mod fake;
pub mod memory;
