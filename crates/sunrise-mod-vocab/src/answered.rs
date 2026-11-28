//! Replace/collect hook answer wrapper carrying the answering mod's identity.

// no wire: dispatch result retains a static owning-mod identity; not an owned transport record.
/// A replace hook's answer, or a collect hook's entry, with the mod that gave it: the dispatch
/// fills in `mod_id` (the answering mod's `[mod] id`), so no mod can answer for another.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Answered<T> {
    /// The replace hook's answer, or the collect hook's entry.
    pub answer: T,
    /// The `[mod] id` of the mod that answered.
    pub mod_id: &'static str,
}
