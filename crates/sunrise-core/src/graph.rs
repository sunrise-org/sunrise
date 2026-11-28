//! Shared algorithms for directed graphs indexed by node.

use std::collections::BTreeSet;

/// Partition a directed graph into strongly connected components.
///
/// Components are returned in topological order: an edge between different components points
/// from an earlier component to a later one. Traversal of node indices and ordered edge sets
/// makes the result deterministic. Both traversals use explicit stacks, so input-controlled
/// graph depth does not consume the call stack.
///
/// # Panics
///
/// Panics if an edge target is outside `edges`.
pub fn strongly_connected_components(edges: &[BTreeSet<usize>]) -> Vec<Vec<usize>> {
    let mut reverse = vec![Vec::new(); edges.len()];
    for (source, targets) in edges.iter().enumerate() {
        for &target in targets {
            reverse[target].push(source);
        }
    }
    let mut seen = vec![false; edges.len()];
    let mut finished = Vec::with_capacity(edges.len());
    let mut stack = Vec::new();
    for root in 0..edges.len() {
        stack.push((root, false));
        while let Some((index, leaving)) = stack.pop() {
            if leaving {
                finished.push(index);
            } else if !std::mem::replace(&mut seen[index], true) {
                stack.push((index, true));
                stack.extend(edges[index].iter().map(|&target| (target, false)));
            }
        }
    }
    seen.fill(false);
    let mut components = Vec::new();
    let mut stack = Vec::new();
    for root in finished.into_iter().rev() {
        if seen[root] {
            continue;
        }
        let mut component = Vec::new();
        stack.push(root);
        while let Some(index) = stack.pop() {
            if !std::mem::replace(&mut seen[index], true) {
                component.push(index);
                stack.extend(reverse[index].iter().copied());
            }
        }
        components.push(component);
    }
    components
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn components_separate_cycles_and_follow_edges_between_them() {
        let edges = [
            BTreeSet::from([1]),
            BTreeSet::from([0, 2]),
            BTreeSet::from([3]),
            BTreeSet::from([2, 4]),
            BTreeSet::new(),
        ];
        let mut components = strongly_connected_components(&edges);
        for component in &mut components {
            component.sort_unstable();
        }
        assert_eq!(components, vec![vec![0, 1], vec![2, 3], vec![4]]);
    }
}
