// silence dead code warnings for the time being until things stabilize
#![allow(dead_code)]

#[macro_use]
pub mod macros;

#[cfg(test)]
mod tests;

mod atomic_waker;
mod loom;

pub mod arc_cell;
pub mod bilock;
pub mod spsc_lock;
