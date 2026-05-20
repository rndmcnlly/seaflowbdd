//! Reduce: canonicalization of CFLOBDD nodes.
//!
//! Reduce takes a node `n`, a reduction map `red` (which translates `n`'s
//! current exit indices into a smaller, contiguous set of new exit
//! indices), and the number `new_num_exits` of distinct values in `red`.
//! It returns a new canonical node whose exits have been merged
//! according to `red`.
//!
//! The two-step structure (compose return maps, then dedup B-conns) is
//! exactly the C++ `CFLOBDDInternalNode::Reduce` algorithm.

use crate::manager::Manager;
use crate::node::{BConnVec, Connection, NodeId, NodeRecord, DONT_CARE, FORK};
use crate::return_map::{ReturnMapId, ReturnMapVec};
use rustc_hash::FxHashMap;

impl Manager {
    /// Public entry point. `red` has length equal to `node`'s `num_exits`,
    /// and contains values in `[0, new_num_exits)`. Every value in
    /// `[0, new_num_exits)` must appear at least once (i.e., `red` is
    /// surjective onto its range), which is the standard ApplyAndReduce
    /// invariant.
    pub(crate) fn reduce(&mut self, node: NodeId, red: &[u32], new_num_exits: u32) -> NodeId {
        // Shortcut 1: collapsing to a single exit ⇒ NoDistinctionNode.
        if new_num_exits == 1 {
            let level = self.nodes.level(node);
            return self.no_distinction(level);
        }
        // Shortcut 2: identity reduction map ⇒ no change.
        if is_identity(red) {
            return node;
        }
        // Memo: intern the reduction map and check the cache. Mirrors C++
        // reduceCache in cflobdd_node.cpp:386-408.
        let red_id = {
            let body = ReturnMapVec::from_slice(red);
            self.return_maps.intern(body)
        };
        if let Some(&cached) = self.reduce_cache.get(&(node, red_id)) {
            return cached;
        }
        let result = self.reduce_inner(node, red, new_num_exits);
        self.reduce_cache.insert((node, red_id), result);
        result
    }

    fn reduce_inner(&mut self, node: NodeId, red: &[u32], new_num_exits: u32) -> NodeId {
        match self.nodes.record(node).clone() {
            NodeRecord::DontCare => {
                // A DontCare has 1 exit; if we hit reduce_inner here with
                // identity-or-collapse already eliminated, something is off.
                debug_assert_eq!(red.len(), 1);
                debug_assert_eq!(red[0], 0);
                debug_assert_eq!(new_num_exits, 1);
                DONT_CARE
            }
            NodeRecord::Fork => {
                // Fork has 2 exits. With identity reduction filtered out by
                // the caller and collapse handled, the only remaining case
                // is red mapping both exits to a single value (handled by
                // shortcut) or actually identity (also handled). So if we
                // reach here, it's [a, b] with a != b but not [0, 1].
                // The only valid surjection onto {0, 1} of length 2 is
                // identity or its reverse.
                debug_assert_eq!(red.len(), 2);
                if red[0] == red[1] {
                    // Both map to the same exit ⇒ NoDistinction[0] = DontCare.
                    DONT_CARE
                } else {
                    // [1, 0] case: behaviorally still a fork, but the
                    // ApplyAndReduce caller's value map will have been
                    // composed differently. Returning Fork keeps structure
                    // canonical because the value map carries the swap.
                    FORK
                }
            }
            NodeRecord::Internal {
                level,
                num_exits: _,
                a_conn,
                b_conns,
            } => {
                // Reduce each B-connection: compose its return map with
                // `red`, recurse with the induced reduction map, dedup by
                // (target, return_map) pair. Track which output position
                // each input B-conn lands at to build the parent's
                // A-reduction map.
                let mut new_b_conns: BConnVec = BConnVec::new();
                let mut a_red: ReturnMapVec = ReturnMapVec::new();
                let mut seen: FxHashMap<(NodeId, ReturnMapId), u32> = FxHashMap::default();

                for b in &b_conns {
                    let child_map_body =
                        ReturnMapVec::from_slice(self.return_maps.body(b.return_map));
                    let (induced_red, induced_return_map_id, induced_num_exits) =
                        self.compose_and_reduce(&child_map_body, red);
                    let new_target = self.reduce(b.target, &induced_red, induced_num_exits);
                    let key = (new_target, induced_return_map_id);
                    let position = if let Some(&pos) = seen.get(&key) {
                        pos
                    } else {
                        let pos = new_b_conns.len() as u32;
                        new_b_conns.push(Connection {
                            target: new_target,
                            return_map: induced_return_map_id,
                        });
                        seen.insert(key, pos);
                        pos
                    };
                    a_red.push(position);
                }
                // a_red now has length numBConnections (the original); it
                // is the reduction map for the A-connection's exit space.
                let new_a_num_exits = new_b_conns.len() as u32;
                let a_child_map_body =
                    ReturnMapVec::from_slice(self.return_maps.body(a_conn.return_map));
                let (induced_a_red, new_a_return_map_id, new_a_num_exits_after_compose) =
                    self.compose_and_reduce(&a_child_map_body, &a_red);
                let new_a_target =
                    self.reduce(a_conn.target, &induced_a_red, new_a_num_exits_after_compose);
                debug_assert_eq!(new_a_num_exits_after_compose, new_a_num_exits);

                self.nodes.intern_internal(
                    level,
                    new_num_exits,
                    Connection {
                        target: new_a_target,
                        return_map: new_a_return_map_id,
                    },
                    new_b_conns,
                )
            }
        }
    }

    /// ComposeAndReduce: given `map` (a return-map body, length =
    /// child's num_exits) and `red` (a reduction map, indexed by child's
    /// exit values), produce:
    ///
    /// 1. `induced_red`: a vector of length `map.len()`, where
    ///    `induced_red[i]` is the position in the dedup'd output that
    ///    corresponds to `map[i]`.
    /// 2. `output_id`: the interned id of the dedup'd composition,
    ///    `[red[map[0]], red[map[1]], ...]` with duplicates removed in
    ///    first-seen order.
    /// 3. `output_len`: the number of distinct values in the output,
    ///    which is the new exit count for the child after recursion.
    fn compose_and_reduce(&mut self, map: &[u32], red: &[u32]) -> (Vec<u32>, ReturnMapId, u32) {
        // Fast path: red is identity ⇒ output = map (unchanged), induced
        // red = identity of len map.len().
        if is_identity(red) {
            let id = {
                let body: ReturnMapVec = map.iter().copied().collect();
                self.return_maps.intern(body)
            };
            let induced_red: Vec<u32> = (0..map.len() as u32).collect();
            return (induced_red, id, map.len() as u32);
        }

        // Use a small dense array keyed on red's value space. Since red's
        // values are in [0, max_red+1) where max_red = max of red, we
        // can use a flat lookup. For typical CFLOBDD reductions max_red
        // is tiny (≤ a handful), so a SmallVec scan is fine; but the
        // flat array generalizes.
        let red_max = red.iter().copied().max().unwrap_or(0);
        let mut slot: Vec<i32> = vec![-1; (red_max + 1) as usize];
        let mut output: ReturnMapVec = ReturnMapVec::new();
        let mut induced_red: Vec<u32> = Vec::with_capacity(map.len());

        for &child_exit in map {
            let parent_new = red[child_exit as usize];
            let s = &mut slot[parent_new as usize];
            if *s < 0 {
                let pos = output.len() as u32;
                output.push(parent_new);
                *s = pos as i32;
                induced_red.push(pos);
            } else {
                induced_red.push(*s as u32);
            }
        }
        let output_len = output.len() as u32;
        let id = self.return_maps.intern(output);
        (induced_red, id, output_len)
    }
}

fn is_identity(red: &[u32]) -> bool {
    for (i, &v) in red.iter().enumerate() {
        if v != i as u32 {
            return false;
        }
    }
    true
}
