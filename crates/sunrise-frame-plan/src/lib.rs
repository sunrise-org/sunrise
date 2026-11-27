//! A format-neutral frame planner (spec #6552, step 0): given a state graph of frames, each an ordered list of
//! resolved events over physical resources, it says which draws and clears can go without changing any value an
//! event, an observer or a later frame reads.
//!
//! - [`model`]: the input. Resources, states and their events, edges between frames, observers.
//! - [`liveness`]: the backward fixed point over the state graph.
//! - [`plan`]: the rules of this step. R1 removes a draw nothing reads before a kill; R4b folds an attachment clear
//!   into the load of the next draw that attaches the same resource; R2 (first version) finds resources whose only
//!   remaining uses are clears. Every rule leaves alone pinned and mipmapped resources, events touching a mipmapped
//!   one, and every resource an unresolved access may reach.
//! - [`trace`]: a frame as the runtime ran it, its projection onto the model, and the checks on it.
//!
//! The crate knows no shader-pack format and no GPU library: the adapter that builds a [`model::Graph`] from a
//! loaded pack lives with the pack runtime.

pub mod live;
pub mod model;
pub mod plan;
pub mod trace;

pub use live::{Liveness, liveness};
pub use plan::{Fold, Plan, Rule, Savings, plan};

#[cfg(test)]
mod tests;
