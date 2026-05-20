//! Restrict + Compose (M8).
//!
//! `restrict(f, x_i, v)` produces the CFLOBDD of `f | x_i = v`. The
//! algorithm mirrors `Restrict` in
//! `cflobdd-upstream/CFLOBDD/cflobdd_node.cpp:2033-2193` and the
//! top-level wrapping at `cflobdd_top_node_int.cpp:439-458`.
//!
//! # Shape of the recursive interface
//!
//! The recursive helper returns `(NodeId, MapHandle)`:
//!
//! - `NodeId`: the structurally-restricted node at the same level.
//! - `MapHandle`: a `Vec<u32>` whose length equals the new node's
//!   `num_exits`. Entry `i` is the *original* node's exit index that
//!   the new exit `i` collapses to. Equivalently, MapHandle is the
//!   return map needed to translate new-exits back into old-exits;
//!   the caller composes this with whatever return-map points at the
//!   recursed-on child.
//!
//! At the top level we compose `MapHandle` with `f.values` to obtain
//! the new value sequence; if that sequence has duplicates, we Reduce
//! to keep the Bdd canonical (the C++ engine accepts the redundancy at
//! the TopNode level; we don't, since our `Bdd` equality is on
//! `(NodeId, ReturnMapId)`).
//!
//! # Compose
//!
//! Phase 1 implementation: derived from Restrict + Apply via Bryant's
//! identity (see `MkComposeTop` in cflobdd_top_node_int.cpp:477-497):
//!
//! ```text
//! compose(f, x_i, g) = (g AND restrict(f, x_i, true))
//!                   OR (NOT g AND restrict(f, x_i, false))
//! ```
//!
//! A native fused Compose is a Phase 2 perf opportunity.

use crate::manager::{Bdd, Manager};
use crate::node::{BConnVec, Connection, NodeId, NodeRecord, DONT_CARE};
use crate::return_map::{ReturnMapId, ReturnMapVec};
use rustc_hash::FxHashMap;

impl Manager {
    /// `restrict(f, x_i, v)`: substitute the constant `v` for variable
    /// `x_i` in `f`. The result is canonical.
    pub fn restrict(&mut self, f: Bdd, var_index: u32, value: bool) -> Bdd {
        assert!(
            var_index < self.total_vars(),
            "var_index {} out of range (total_vars = {})",
            var_index,
            self.total_vars()
        );

        // Recurse on the structural root.
        let mut map_handle: Vec<u32> = Vec::new();
        let new_root = self.restrict_node(f.root, var_index, value, &mut map_handle);

        // Compose MapHandle with the original value sequence.
        // (cflobdd_top_node_int.cpp:445-455)
        //
        // new_values_seq[i] = f.values[MapHandle[i]]
        let f_values_body = ReturnMapVec::from_slice(self.return_maps.body(f.values));
        let composed: Vec<u32> = map_handle
            .iter()
            .map(|&d| f_values_body[d as usize])
            .collect();

        // Dedupe in first-seen order; build a reduction map so we can
        // collapse the structural node accordingly. This is the step
        // the C++ engine omits (it leaves the TopNode's returnMap
        // possibly non-minimal); we add it because our `Bdd` equality
        // is purely on canonical ids.
        let (new_values_body, reduction) = dedupe_first_seen(&composed);
        let new_num_exits = new_values_body.len() as u32;
        let new_values_id = self
            .return_maps
            .intern(new_values_body.into_iter().collect());
        let reduced_root = self.reduce(new_root, &reduction, new_num_exits);

        Bdd {
            root: reduced_root,
            values: new_values_id,
        }
    }

    /// Workhorse mirroring `Restrict(CFLOBDDInternalNode*, ...)` and the
    /// public NodeHandle dispatcher in cflobdd_node.cpp:2035-2193.
    ///
    /// `map_handle` is an output parameter (mutated): on return it
    /// holds the parent-side translation from new-exits back to
    /// original-exits of `node`.
    fn restrict_node(
        &mut self,
        node: NodeId,
        var_index: u32,
        value: bool,
        map_handle: &mut Vec<u32>,
    ) -> NodeId {
        match self.nodes.record(node).clone() {
            NodeRecord::DontCare => {
                // cflobdd_node.cpp:2058-2062
                map_handle.push(0);
                DONT_CARE
            }
            NodeRecord::Fork => {
                // cflobdd_node.cpp:2046-2056
                if value {
                    map_handle.push(1);
                } else {
                    map_handle.push(0);
                }
                DONT_CARE
            }
            NodeRecord::Internal {
                level,
                num_exits: _,
                a_conn,
                b_conns,
            } => {
                // Shortcut: NoDistinctionNode is invariant under restrict.
                // cflobdd_node.cpp:2070-2074
                let nd = self.no_distinction(level);
                if node == nd {
                    map_handle.push(0);
                    return node;
                }

                let half = 1u32 << (level - 1);
                if var_index < half {
                    self.restrict_a_case(level, a_conn, &b_conns, var_index, value, map_handle)
                } else {
                    self.restrict_b_case(
                        level,
                        a_conn,
                        &b_conns,
                        var_index - half,
                        value,
                        map_handle,
                    )
                }
            }
        }
    }

    /// Variable falls in the A-connection's range
    /// (cflobdd_node.cpp:2082-2117).
    ///
    /// We recurse into the A-conn target, getting back a new A-target
    /// and an `a_map` describing how the new A-exits map to original
    /// child-exits (= B-connection indices of the original node since
    /// the A-connection's return map is identity in canonical form).
    /// We then build new B-connections directly from the selected
    /// original B-connections, growing `map_handle` as we discover
    /// each new (parent-exit) value.
    fn restrict_a_case(
        &mut self,
        level: u8,
        a_conn: Connection,
        b_conns: &[Connection],
        var_index: u32,
        value: bool,
        map_handle: &mut Vec<u32>,
    ) -> NodeId {
        let mut a_map: Vec<u32> = Vec::new();
        let new_a_target = self.restrict_node(a_conn.target, var_index, value, &mut a_map);

        // Translate a_map (which is in the A-conn target's exit space)
        // through a_conn.return_map to land in the parent's B-conn
        // index space.
        let a_conn_map_body = ReturnMapVec::from_slice(self.return_maps.body(a_conn.return_map));
        let b_indices: Vec<u32> = a_map
            .iter()
            .map(|&child_exit| a_conn_map_body[child_exit as usize])
            .collect();

        // Build new B-conns: one per entry in b_indices. Each takes
        // the original B-conn's target unchanged; its return map is
        // rewritten by walking the original return map and looking up
        // (or extending) `map_handle`.
        let mut new_b_conns = BConnVec::new();
        let mut cur_exit: u32 = 0;
        for &b in &b_indices {
            let orig = b_conns[b as usize];
            let orig_rm = ReturnMapVec::from_slice(self.return_maps.body(orig.return_map));
            let mut new_rm = ReturnMapVec::new();
            for &c in &orig_rm {
                let pos = match map_handle.iter().position(|&x| x == c) {
                    Some(p) => p as u32,
                    None => {
                        map_handle.push(c);
                        let p = cur_exit;
                        cur_exit += 1;
                        p
                    }
                };
                new_rm.push(pos);
            }
            let new_rm_id = self.return_maps.intern(new_rm);
            new_b_conns.push(Connection {
                target: orig.target,
                return_map: new_rm_id,
            });
        }

        // The new A-conn return map is the identity of length |a_map|
        // (each new A-exit selects a distinct new B-conn slot).
        let new_a_return_map = self.return_maps.identity(a_map.len() as u32);
        let new_num_exits = cur_exit;

        // Note: the C++ engine constructs the node directly without a
        // Reduce pass in the A-case (cflobdd_node.cpp:2117); we rely
        // on the same invariant (the new B-conns are pairwise distinct
        // by inheritance from `g`'s canonicity, since their rewritten
        // return maps are deterministic functions of the originals).
        self.nodes.intern_internal(
            level,
            new_num_exits,
            Connection {
                target: new_a_target,
                return_map: new_a_return_map,
            },
            new_b_conns,
        )
    }

    /// Variable falls in the B-connection's range
    /// (cflobdd_node.cpp:2119-2185).
    ///
    /// Recurse into each B-conn's target with the adjusted index.
    /// Each result may have fewer exits than the original; we dedupe
    /// the resulting B-conns by `(target, return_map)` and build an
    /// A-reduction map describing the dedup pattern. Finally we reduce
    /// the A-conn's target by composing its return map with the
    /// A-reduction map.
    fn restrict_b_case(
        &mut self,
        level: u8,
        a_conn: Connection,
        b_conns: &[Connection],
        adj_index: u32,
        value: bool,
        map_handle: &mut Vec<u32>,
    ) -> NodeId {
        let mut new_b_conns: BConnVec = BConnVec::new();
        let mut a_reduction: Vec<u32> = Vec::with_capacity(b_conns.len());
        let mut b_dedup: FxHashMap<(NodeId, ReturnMapId), u32> = FxHashMap::default();
        let mut cur_exit: u32 = 0;

        for orig_b in b_conns {
            let mut b_map: Vec<u32> = Vec::new();
            let m = self.restrict_node(orig_b.target, adj_index, value, &mut b_map);

            // Compose b_map with this B-conn's return map to get the
            // global parent-exit index for each new exit of `m`.
            // (cflobdd_node.cpp:2140)
            let orig_rm = ReturnMapVec::from_slice(self.return_maps.body(orig_b.return_map));
            let mut induced = ReturnMapVec::new();
            for &child_exit in &b_map {
                let c = orig_rm[child_exit as usize];
                let pos = match map_handle.iter().position(|&x| x == c) {
                    Some(p) => p as u32,
                    None => {
                        map_handle.push(c);
                        let p = cur_exit;
                        cur_exit += 1;
                        p
                    }
                };
                induced.push(pos);
            }
            let induced_id = self.return_maps.intern(induced);
            let key = (m, induced_id);
            let position = if let Some(&p) = b_dedup.get(&key) {
                p
            } else {
                let p = new_b_conns.len() as u32;
                new_b_conns.push(Connection {
                    target: m,
                    return_map: induced_id,
                });
                b_dedup.insert(key, p);
                p
            };
            a_reduction.push(position);
        }

        // Reduce the A-conn w.r.t. the dedup pattern.
        // (cflobdd_node.cpp:2179-2184)
        //
        // Conceptually: compose a_conn.return_map with a_reduction,
        // producing reduced_a_return_map of length = original A-conn
        // num_exits. Then dedupe (induced_a_reduction over a_conn's
        // child) and Reduce the A-conn's target.
        let a_conn_rm = ReturnMapVec::from_slice(self.return_maps.body(a_conn.return_map));
        let composed_a_rm: Vec<u32> = a_conn_rm
            .iter()
            .map(|&old_b_idx| a_reduction[old_b_idx as usize])
            .collect();
        let (induced_a_rm_body, induced_a_reduction) = dedupe_first_seen(&composed_a_rm);
        let induced_a_rm_id = self
            .return_maps
            .intern(induced_a_rm_body.into_iter().collect());
        let new_a_num_exits = self.return_maps.body(induced_a_rm_id).len() as u32;
        let reduced_a_target = self.reduce(a_conn.target, &induced_a_reduction, new_a_num_exits);

        self.nodes.intern_internal(
            level,
            cur_exit,
            Connection {
                target: reduced_a_target,
                return_map: induced_a_rm_id,
            },
            new_b_conns,
        )
    }

    /// `compose(f, x_i, g)`: substitute `g` for `x_i` in `f`.
    ///
    /// Phase 1 derivation (Bryant 1986; see
    /// cflobdd_top_node_int.cpp:477-497):
    ///
    /// `compose(f, x_i, g) = (g AND f|x_i=1) OR (¬g AND f|x_i=0)`
    pub fn compose(&mut self, f: Bdd, var_index: u32, g: Bdd) -> Bdd {
        let f_true = self.restrict(f, var_index, true);
        let f_false = self.restrict(f, var_index, false);
        let not_g = self.not(g);
        let left = self.and(g, f_true);
        let right = self.and(not_g, f_false);
        self.or(left, right)
    }

    /// `exists(f, x_i) = f|x_i=1 OR f|x_i=0`.
    pub fn exists(&mut self, f: Bdd, var_index: u32) -> Bdd {
        let t = self.restrict(f, var_index, true);
        let fa = self.restrict(f, var_index, false);
        self.or(t, fa)
    }

    /// `forall(f, x_i) = f|x_i=1 AND f|x_i=0`.
    pub fn forall(&mut self, f: Bdd, var_index: u32) -> Bdd {
        let t = self.restrict(f, var_index, true);
        let fa = self.restrict(f, var_index, false);
        self.and(t, fa)
    }
}

/// Dedupe `seq` in first-seen order, returning the deduped body and a
/// reduction map (`reduction[i]` is the position of `seq[i]` in the
/// deduped sequence). Used both at the top level (collapse value-map
/// duplicates after MapHandle composition) and inside `restrict_b_case`
/// (collapse A-conn return-map duplicates after the dedup pattern of
/// B-connections has been applied).
fn dedupe_first_seen(seq: &[u32]) -> (Vec<u32>, Vec<u32>) {
    let mut body: Vec<u32> = Vec::new();
    let mut reduction: Vec<u32> = Vec::with_capacity(seq.len());
    for &v in seq {
        let pos = match body.iter().position(|&x| x == v) {
            Some(p) => p as u32,
            None => {
                let p = body.len() as u32;
                body.push(v);
                p
            }
        };
        reduction.push(pos);
    }
    (body, reduction)
}
