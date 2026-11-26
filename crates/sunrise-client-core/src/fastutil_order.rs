//! Iteration order of the Java hash collections vanilla keeps things in: fastutil's open hash sets and maps
//! (open addressing with linear probing, backward-shift deletion and rehashing at the load factor; the null key, 0
//! for longs, kept apart from the table) and `java.util.HashSet<String>` (chained buckets split on resize).
//! The scoreboard's names: `/team list` walks team names in fastutil iterator order; `scoreboard.dat` saves them
//! in fastutil's forward stream order and saves team members in Java HashSet order; `/team list <team>` sorts the
//! members. A server's chunks waiting to be sent a player (`PlayerChunkSender.pendingChunks`, a
//! `LongOpenHashSet` of packed chunk positions) are streamed in that order too.

use std::borrow::Borrow;
use sunrise_core::{java_hash_set::JavaHashSet, random::java_string_hash};

/// A key of a fastutil open hash collection: its hash as the table uses it, and whether it is the null key.
pub trait OpenKey {
    /// `HashCommon.mix` of the key's hash code.
    fn mixed(&self) -> usize;
    /// The null key (`containsNull`, slot `n`): 0 for a primitive long; no string is.
    fn is_null(&self) -> bool {
        false
    }
}
impl OpenKey for str {
    fn mixed(&self) -> usize {
        mix(java_string_hash(self.encode_utf16()))
    }
}
impl OpenKey for String {
    fn mixed(&self) -> usize {
        self.as_str().mixed()
    }
}
impl OpenKey for i64 {
    /// `HashCommon.mix(long)`.
    fn mixed(&self) -> usize {
        let h = self.wrapping_mul(0x9E37_79B9_7F4A_7C15_u64 as i64);
        let h = h ^ ((h as u64) >> 32) as i64;
        (h ^ ((h as u64) >> 16) as i64) as i32 as u32 as usize
    }
    fn is_null(&self) -> bool {
        *self == 0
    }
}

/// The keys of a fastutil open hash set (or map) in their table slots.
#[derive(Clone, Debug, PartialEq)]
pub struct Open<K> {
    keys: Vec<Option<K>>,
    /// The null key, when the set holds it (`key[n]`).
    null: Option<K>,
    size: usize,
    min_n: usize,
    load: f32,
}
/// `Object2ObjectOpenHashMap<String, _>`'s keys.
pub type OpenOrder = Open<String>;
/// `LongOpenHashSet`.
pub type LongOpenOrder = Open<i64>;

impl<K> Default for Open<K> {
    /// `new ...OpenHash...()`: 16 expected entries, load 0.75.
    fn default() -> Self {
        Self::new(16, 0.75)
    }
}
/// `HashCommon.mix`.
fn mix(x: i32) -> usize {
    let h = (x as u32).wrapping_mul(0x9E37_79B9);
    (h ^ (h >> 16)) as usize
}
/// `HashCommon.arraySize`.
fn array_size(expected: usize, f: f32) -> usize {
    let wanted = (expected as f64 / f64::from(f)).ceil() as usize;
    wanted.max(2).next_power_of_two()
}
/// `HashCommon.maxFill`.
fn max_fill(n: usize, f: f32) -> usize {
    ((n as f64 * f64::from(f)).ceil() as usize).min(n - 1)
}
impl<K> Open<K> {
    pub fn new(expected: usize, load: f32) -> Self {
        let n = array_size(expected, load);
        Self {
            keys: (0..n).map(|_| None).collect(),
            null: None,
            size: 0,
            min_n: n,
            load,
        }
    }
    pub fn len(&self) -> usize {
        self.size
    }
    pub fn is_empty(&self) -> bool {
        self.size == 0
    }
    /// The bytes its table holds.
    pub fn heap_bytes(&self) -> usize {
        self.keys.capacity() * size_of::<Option<K>>()
    }
    /// Every key removed one by one, as a set emptied by `remove` is: its table shrinks as fastutil's does, so
    /// keys added later land where they would in vanilla's (`clear` would keep the table at its size).
    pub fn remove_all(&mut self)
    where
        K: OpenKey + Eq + Clone,
    {
        let keys: Vec<K> = self.slots_up().cloned().collect();
        for key in &keys {
            self.remove(key);
        }
    }
    /// `iterator()` and `forEach`: the null key, then from the last slot down.
    fn slots_down(&self) -> impl Iterator<Item = &K> {
        self.null.iter().chain(self.keys.iter().rev().flatten())
    }
    /// `spliterator()` (what `stream()` uses): the null key, then from the first slot up.
    fn slots_up(&self) -> impl Iterator<Item = &K> {
        self.null.iter().chain(self.keys.iter().flatten())
    }
}
impl<K: OpenKey + Eq> Open<K> {
    pub fn contains<Q>(&self, key: &Q) -> bool
    where
        K: Borrow<Q>,
        Q: OpenKey + Eq + ?Sized,
    {
        if key.is_null() {
            return self.null.is_some();
        }
        let mask = self.keys.len() - 1;
        let mut pos = key.mixed() & mask;
        while let Some(k) = &self.keys[pos] {
            if Borrow::<Q>::borrow(k) == key {
                return true;
            }
            pos = (pos + 1) & mask;
        }
        false
    }
    /// `add`/`put`: false when the key was already there.
    pub fn insert<Q>(&mut self, key: &Q) -> bool
    where
        K: Borrow<Q>,
        Q: OpenKey + Eq + ToOwned<Owned = K> + ?Sized,
    {
        if key.is_null() {
            if self.null.is_some() {
                return false;
            }
            self.null = Some(key.to_owned());
        } else {
            let mask = self.keys.len() - 1;
            let mut pos = key.mixed() & mask;
            while let Some(k) = &self.keys[pos] {
                if Borrow::<Q>::borrow(k) == key {
                    return false;
                }
                pos = (pos + 1) & mask;
            }
            self.keys[pos] = Some(key.to_owned());
        }
        // `if (size++ >= maxFill) rehash(arraySize(size + 1, f))`.
        let old = self.size;
        self.size += 1;
        if old >= max_fill(self.keys.len(), self.load) {
            self.rehash(array_size(self.size + 1, self.load));
        }
        true
    }
    /// `remove`: false when the key was not there.
    pub fn remove<Q>(&mut self, key: &Q) -> bool
    where
        K: Borrow<Q>,
        Q: OpenKey + Eq + ?Sized,
    {
        if key.is_null() {
            // `removeNullEntry`.
            if self.null.take().is_none() {
                return false;
            }
        } else {
            let mask = self.keys.len() - 1;
            let mut pos = key.mixed() & mask;
            loop {
                match &self.keys[pos] {
                    None => return false,
                    Some(k) if Borrow::<Q>::borrow(k) == key => break,
                    Some(_) => pos = (pos + 1) & mask,
                }
            }
            // `removeEntry`.
            self.shift_keys(pos);
        }
        self.size -= 1;
        let n = self.keys.len();
        if n > self.min_n && self.size < max_fill(n, self.load) / 4 && n > 16 {
            self.rehash(n / 2);
        }
        true
    }
    fn shift_keys(&mut self, mut pos: usize) {
        let mask = self.keys.len() - 1;
        loop {
            let last = pos;
            pos = (last + 1) & mask;
            loop {
                let Some(slot) = self.keys[pos].as_ref().map(|k| k.mixed() & mask) else {
                    self.keys[last] = None;
                    return;
                };
                let stop = if last <= pos {
                    last >= slot || slot > pos
                } else {
                    last >= slot && slot > pos
                };
                if stop {
                    self.keys[last] = self.keys[pos].take();
                    break;
                }
                pos = (pos + 1) & mask;
            }
        }
    }
    fn rehash(&mut self, new_n: usize) {
        let old = std::mem::replace(&mut self.keys, (0..new_n).map(|_| None).collect());
        let mask = new_n - 1;
        // `for (int j = realSize(); j-- != 0;) { while (key[--i] == null); ... }`
        for key in old.into_iter().rev().flatten() {
            let mut pos = key.mixed() & mask;
            while self.keys[pos].is_some() {
                pos = (pos + 1) & mask;
            }
            self.keys[pos] = Some(key);
        }
    }
}
impl Open<String> {
    /// `iterator()` and `forEach`: from the last slot down.
    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.slots_down().map(String::as_str)
    }
    /// `spliterator()` (what `stream()` uses): from the first slot up.
    pub fn stream(&self) -> impl Iterator<Item = &str> {
        self.slots_up().map(String::as_str)
    }
}
impl Open<i64> {
    /// `iterator()` and `forEach`: the null key (0), then from the last slot down.
    pub fn iter(&self) -> impl Iterator<Item = i64> + '_ {
        self.slots_down().copied()
    }
    /// `spliterator()` (what `stream()` and `longStream()` use): the null key (0), then from the first slot up.
    pub fn stream(&self) -> impl Iterator<Item = i64> + '_ {
        self.slots_up().copied()
    }
}

/// Score holder names in `java.util.HashSet<String>` traversal order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HashSetOrder(JavaHashSet<String>);

impl HashSetOrder {
    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub fn contains(&self, key: &str) -> bool {
        self.0.contains(key)
    }
    pub fn insert(&mut self, key: &str) -> bool {
        self.0.insert(key.to_owned())
    }
    pub fn remove(&mut self, key: &str) -> bool {
        self.0.remove(key)
    }
    pub fn iter(&self) -> impl Iterator<Item = &String> {
        self.0.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_sets_zero_key_comes_first_in_both_orders() {
        // fastutil keeps 0, the null key, apart from the table: first in `iterator()` and in `spliterator()`.
        let mut set = LongOpenOrder::default();
        for key in [5_i64, 0, -3, 1 << 32] {
            assert!(set.insert(&key));
        }
        assert!(!set.insert(&0));
        assert_eq!(set.iter().next(), Some(0));
        assert_eq!(set.stream().next(), Some(0));
        let down: Vec<i64> = set.iter().skip(1).collect();
        let mut up: Vec<i64> = set.stream().skip(1).collect();
        up.reverse();
        assert_eq!(down, up, "the table, from either end");
        assert!(set.remove(&0) && !set.contains(&0) && set.len() == 3);
        set.remove_all();
        assert!(set.is_empty() && set.stream().next().is_none());
    }

    #[test]
    fn team_names_follow_java_hashset_tree_order() {
        let keys: Vec<String> = (0..16)
            .map(|i| {
                (0..4)
                    .map(|b| if i & (1 << b) == 0 { "Aa" } else { "BB" })
                    .collect()
            })
            .collect();
        let mut members = HashSetOrder::default();
        for k in &keys {
            assert!(members.insert(k));
        }
        // Temurin 25.0.5, HashSet populated by adding keys 0 through 15.
        let order = [1, 0, 2, 3, 13, 4, 12, 5, 6, 14, 7, 15, 11, 8, 9, 10];
        assert!(members.iter().eq(order.iter().map(|&i| &keys[i])));
        assert!(members.contains(&keys[11]));
        assert!(members.remove(&keys[11]));
        assert!(!members.contains(&keys[11]));
        assert!(!members.remove(&keys[11]));
    }

    #[test]
    fn hash_and_order() {
        // String.hashCode over UTF-16 code units: the emoji is two surrogates.
        assert_eq!(java_string_hash("\u{1F600}".encode_utf16()), 1_772_899);
        let mut o = OpenOrder::new(16, 0.5);
        for k in ["harness", "harness_h"] {
            o.insert(k);
        }
        assert_eq!(o.iter().count(), 2);
        let mut asc: Vec<&str> = o.stream().collect();
        asc.reverse();
        assert_eq!(asc, o.iter().collect::<Vec<_>>());
        o.remove("harness");
        assert_eq!(o.iter().collect::<Vec<_>>(), vec!["harness_h"]);
        for i in 0..40 {
            o.insert(&format!("k{i}"));
        }
        assert_eq!(o.iter().count(), 41);
        for i in 0..40 {
            o.remove(&format!("k{i}"));
        }
        assert_eq!(o.iter().collect::<Vec<_>>(), vec!["harness_h"]);
    }

    #[test]
    fn hash_set_buckets() {
        let mut s = HashSetOrder::default();
        for i in 0..30 {
            assert!(s.insert(&format!("p{i}")));
        }
        assert!(!s.insert("p3"));
        assert_eq!(s.len(), 30);
        assert!(s.remove("p3") && !s.contains("p3"));
        // "Aa" and "BB" share a hash: insertion order within the bucket.
        let mut t = HashSetOrder::default();
        t.insert("BB");
        t.insert("Aa");
        assert_eq!(t.iter().collect::<Vec<_>>(), vec!["BB", "Aa"]);
    }
}
