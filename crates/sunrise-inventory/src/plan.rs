//! The plain left clicks that sort a run of slots, so a vanilla server accepts them.

/// A stack's place in the order: its creative-inventory place, then its shown name.
pub(crate) type Rank = (u32, String);

/// A stack as the plan needs it.
#[derive(Clone, Debug)]
pub(crate) struct Item {
    /// Equal for stacks that merge.
    pub(crate) kind: u64,
    pub(crate) count: u32,
    pub(crate) max: u32,
    pub(crate) rank: Rank,
}

impl PartialEq for Item {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind && self.count == other.count
    }
}

/// A hand and slots under `AbstractContainerMenu.doClick`'s left `PICKUP`.
struct Sim {
    slots: Vec<Option<Item>>,
    hand: Option<Item>,
    clicks: Vec<usize>,
}

impl Sim {
    fn click(&mut self, i: usize) {
        self.clicks.push(i);
        let slot = self.slots[i].take();
        match (self.hand.take(), slot) {
            // Take the stack, or put the hand's down.
            (None, s) => self.hand = s,
            (Some(h), None) => self.slots[i] = Some(h),
            // The same kind: as much as fits joins the slot.
            (Some(mut h), Some(mut s)) if s.kind == h.kind => {
                let n = s.max.saturating_sub(s.count).min(h.count);
                s.count += n;
                h.count -= n;
                self.slots[i] = Some(s);
                self.hand = (h.count > 0).then_some(h);
            }
            // Another kind: they swap.
            (Some(h), Some(s)) => {
                self.slots[i] = Some(h);
                self.hand = Some(s);
            }
        }
    }
    fn partial(&self, i: usize) -> Option<&Item> {
        self.slots[i].as_ref().filter(|s| s.count < s.max)
    }
}

/// The slots to left-click, in order, to sort; empty when sorted or unreachable (overfull stacks).
pub(crate) fn plan(slots: &[Option<Item>]) -> Vec<usize> {
    let n = slots.len();
    let mut sim = Sim {
        slots: slots.to_vec(),
        hand: None,
        clicks: Vec::new(),
    };
    // Merge: each later partial stack tops up the kind's first one; what doesn't fit goes back.
    for i in 0..n {
        for j in i + 1..n {
            let Some(s) = sim.partial(i) else {
                break;
            };
            if !sim.partial(j).is_some_and(|t| t.kind == s.kind) {
                continue;
            }
            sim.click(j);
            sim.click(i);
            if sim.hand.is_some() {
                sim.click(j);
            }
        }
    }
    let stacks: Vec<Item> = sim.slots.iter().flatten().cloned().collect();
    let mut order: Vec<usize> = (0..stacks.len()).collect();
    order.sort_by(|&a, &b| {
        (stacks[a].rank)
            .cmp(&stacks[b].rank)
            .then(stacks[b].count.cmp(&stacks[a].count))
    });
    let target: Vec<Option<Item>> = order
        .iter()
        .map(|&i| Some(stacks[i].clone()))
        .chain(std::iter::repeat(None))
        .take(n)
        .collect();
    // Order: each slot takes its stack from further on; what it held goes where that stack was.
    for i in 0..n {
        if sim.slots[i] == target[i] {
            continue;
        }
        let Some(j) = (i + 1..n).find(|&j| sim.slots[j] == target[i]) else {
            return Vec::new();
        };
        sim.click(j);
        sim.click(i);
        if sim.hand.is_some() {
            sim.click(j);
        }
    }
    if sim.hand.is_some() || sim.slots != target {
        return Vec::new();
    }
    sim.clicks
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORDER: &[&str] = &["stone", "dirt", "diamond_sword", "ender_pearl"];

    fn stack(item: &str, count: u32) -> Option<Item> {
        let rank = ORDER
            .iter()
            .position(|o| *o == item)
            .unwrap_or(u32::MAX as usize);
        let max = match item {
            "ender_pearl" => 16,
            "diamond_sword" => 1,
            _ => 64,
        };
        let kind = item
            .bytes()
            .fold(0u64, |h, b| h.wrapping_mul(31).wrapping_add(u64::from(b)));
        Some(Item {
            kind,
            count,
            max,
            rank: (rank as u32, item.to_owned()),
        })
    }

    #[test]
    fn merges_partial_stacks_then_orders_full_stacks_first() {
        let slots = vec![
            stack("ender_pearl", 3),
            None,
            stack("dirt", 40),
            stack("stone", 64),
            stack("dirt", 40),
            stack("ender_pearl", 15),
            stack("diamond_sword", 1),
            stack("zombie_head", 1),
        ];
        let mut sim = Sim {
            slots: slots.clone(),
            hand: None,
            clicks: Vec::new(),
        };
        for i in plan(&slots) {
            sim.click(i);
        }
        assert_eq!(sim.hand, None);
        assert_eq!(
            sim.slots,
            vec![
                stack("stone", 64),
                stack("dirt", 64),
                stack("dirt", 16),
                stack("diamond_sword", 1),
                stack("ender_pearl", 16),
                stack("ender_pearl", 2),
                stack("zombie_head", 1),
                None,
            ]
        );
        // Sorted slots need no clicks.
        assert!(plan(&sim.slots).is_empty());
    }
}
