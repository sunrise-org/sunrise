//! The order of Guava's `Comparators.least(k, comparator)` (`TopKSelector`): the `k` least of a stream, which keeps
//! a buffer of `2k`, cuts it back to `k` by quickselect each time it fills, and sorts what is left. Equal elements
//! come out in the order the buffer's partitions leave them, not the stream's: vanilla's choice of which of several
//! equally near chunks a player is sent first (`PlayerChunkSender.collectChunksToSend`) follows it.

/// `stream.collect(Comparators.least(k, comparing(key)))`.
pub fn least<T: Copy, K: Ord>(
    items: impl IntoIterator<Item = T>,
    k: usize,
    key: impl Fn(T) -> K,
) -> Vec<T> {
    if k == 0 {
        return Vec::new();
    }
    let mut buffer: Vec<T> = Vec::with_capacity(2 * k);
    let mut threshold: Option<T> = None;
    for elem in items {
        match threshold {
            None => {
                buffer.push(elem);
                threshold = Some(elem);
            }
            Some(t) if buffer.len() < k => {
                buffer.push(elem);
                if key(elem) > key(t) {
                    threshold = Some(elem);
                }
            }
            Some(t) => {
                if key(elem) < key(t) {
                    buffer.push(elem);
                    if buffer.len() == 2 * k {
                        threshold = Some(trim(&mut buffer, k, &key));
                    }
                }
            }
        }
    }
    // `topK`: `Arrays.sort` is stable, as `sort_by_key` is.
    buffer.sort_by_key(|&e| key(e));
    buffer.truncate(k);
    buffer
}

/// `TopKSelector.trim`: the buffer cut back to its `k` least; the new threshold, the greatest of them.
fn trim<T: Copy, K: Ord>(buffer: &mut Vec<T>, k: usize, key: &impl Fn(T) -> K) -> T {
    let (mut left, mut right) = (0usize, 2 * k - 1);
    let mut min_threshold_position = 0;
    let mut iterations = 0;
    // `IntMath.log2(right - left, CEILING) * 3`.
    let max_iterations = (usize::BITS - (right - left - 1).leading_zeros()) as usize * 3;
    while left < right {
        let pivot_index = (left + right + 1) >> 1;
        let pivot_new_index = partition(buffer, left, right, pivot_index, key);
        if pivot_new_index > k {
            right = pivot_new_index - 1;
        } else if pivot_new_index < k {
            left = pivot_new_index.max(left + 1);
            min_threshold_position = pivot_new_index;
        } else {
            break;
        }
        iterations += 1;
        if iterations >= max_iterations {
            buffer[left..=right].sort_by_key(|&e| key(e));
            break;
        }
    }
    buffer.truncate(k);
    let mut threshold = buffer[min_threshold_position];
    for &e in &buffer[min_threshold_position + 1..k] {
        if key(e) > key(threshold) {
            threshold = e;
        }
    }
    threshold
}

/// `TopKSelector.partition`: the elements of `left..right` less than the pivot moved before it.
fn partition<T: Copy, K: Ord>(
    buffer: &mut [T],
    left: usize,
    right: usize,
    pivot_index: usize,
    key: &impl Fn(T) -> K,
) -> usize {
    let pivot = buffer[pivot_index];
    buffer[pivot_index] = buffer[right];
    let mut pivot_new_index = left;
    for i in left..right {
        if key(buffer[i]) < key(pivot) {
            buffer.swap(pivot_new_index, i);
            pivot_new_index += 1;
        }
    }
    buffer[right] = buffer[pivot_new_index];
    buffer[pivot_new_index] = pivot;
    pivot_new_index
}

#[cfg(test)]
mod tests {
    use super::least;

    #[test]
    fn the_k_least_come_out_sorted_whatever_the_stream_order() {
        // Distinct keys: the result is the sorted prefix, through every buffer trim.
        let items: Vec<i32> = (0..500).map(|i| (i * 7919) % 1009).collect();
        let mut sorted = items.clone();
        sorted.sort();
        for k in [1, 2, 3, 10, 64, 499, 500, 600] {
            assert_eq!(
                least(items.iter().copied(), k, |x| x),
                sorted[..k.min(items.len())],
                "k {k}"
            );
        }
        assert!(least(items.iter().copied(), 0, |x| x).is_empty());
    }
}
