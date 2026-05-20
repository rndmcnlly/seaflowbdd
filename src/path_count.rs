//! Path counting (M7).
//!
//! For each non-leaf node we want, per-exit, the number of input
//! assignments to that node's variables that funnel to that exit. The
//! C++ engine stores this on each node directly, in `long double` log2
//! space (see `cflobdd_node.cpp` lines 1774-1818). We diverge per
//! `PLAN.md` decision §4: store path counts in `i128` linear space, two
//! to three integer ops per merge instead of two libm calls.
//!
//! # Storage decision: lazy cache on Manager (option b)
//!
//! Rather than threading `paths: Vec<i128>` through `NodeRecord::Internal`
//! (option a), we keep the canonical store free of derived data and
//! memoize per-NodeId on the manager. The cache is a `HashMap<NodeId,
//! Vec<i128>>`. Trade-offs:
//!
//! - Pro: `NodeRecord` stays compact and pure; intern-time stays cheap.
//! - Pro: Path counts are only computed when asked, and shared across
//!   every Bdd that references the same root.
//! - Pro: The cache can be invalidated wholesale if (future) GC ever
//!   remaps NodeIds, by clearing one map.
//! - Con: First call on a Bdd does a full bottom-up pass; later C++
//!   parity work would need to amortize this against C++'s eager
//!   InstallPathCounts. Acceptable for v1.
//!
//! # Recursion
//!
//! Leaves:
//! - `DontCare` (level 0, 1 exit): `paths = [2]`. Reading C++:
//!   `numPathsToExit[0] = 1` (which is log2(2) = 1 in their log space),
//!   confirming the linear value is 2. Both input bits at that variable
//!   reach the constant exit.
//! - `Fork` (level 0, 2 exits): `paths = [1, 1]`. C++ stores `[0, 0]`
//!   = log2(1).
//!
//! Internal at level k with numExits exits:
//!
//! ```text
//! paths[k] = sum over (i, j) such that
//!     b_conns[a_conn.return_map[i]].return_map[j] == k
//!   of  a_paths[i] * b_paths_for_that_b_conn[j]
//! ```
//!
//! Per `pair_product.rs::is_identity_return_map` and the canonical-form
//! invariant, the A-connection's return map is always identity, so
//! `a_conn.return_map[i] == i`; the i-th outer iteration uses
//! `b_conns[i]`. We `debug_assert!` that.
//!
//! # Overflow
//!
//! `i128::MAX ≈ 1.7e38`. Total assignments at level k is `2^(2^k)`:
//! - level 5: 2^32 ≈ 4.3e9 (fine)
//! - level 6: 2^64 ≈ 1.8e19 (fine)
//! - level 7: 2^128 (overflow, equal to i128 max + 1)
//!
//! For Phase 1 we restrict to `level ≤ 6`. Multiplication and addition
//! use `checked_*` and panic on overflow with a clear message.

use crate::manager::{Bdd, Manager};
use crate::node::{NodeId, NodeRecord, DONT_CARE, FORK};
use hashbrown::HashMap;

/// Memo cache keyed by NodeId. Owned by the Manager.
#[derive(Default)]
pub(crate) struct PathCountCache {
    paths: HashMap<NodeId, Vec<i128>>,
}

impl PathCountCache {
    pub(crate) fn new() -> Self {
        Self {
            paths: HashMap::new(),
        }
    }
}

impl Manager {
    /// Number of satisfying assignments for `bdd`.
    ///
    /// Sums per-exit path counts for exits whose value-map entry is `1`.
    /// Subsequent calls referring to the same node are O(exits) thanks
    /// to the per-NodeId memo cache.
    ///
    /// # Panics
    ///
    /// - If the manager's `level > 6` (path counts may overflow `i128`).
    /// - On i128 overflow during accumulation (only possible above the
    ///   level limit; the level guard makes this unreachable).
    pub fn path_count(&mut self, bdd: Bdd) -> i128 {
        assert!(
            self.level() <= 6,
            "path_count: level {} exceeds the i128 fast path (max level 6, \
             which gives 2^64 assignments). Phase 1 does not implement the \
             num-bigint fallback.",
            self.level()
        );
        let root_paths = self.compute_paths(bdd.root);
        let values = self.return_maps.body(bdd.values).to_vec();
        debug_assert_eq!(
            values.len(),
            root_paths.len(),
            "value-map length must equal the root node's exit count"
        );
        let mut acc: i128 = 0;
        for (exit_idx, &v) in values.iter().enumerate() {
            if v == 1 {
                acc = acc
                    .checked_add(root_paths[exit_idx])
                    .expect("path_count: i128 overflow summing exit contributions");
            }
        }
        acc
    }

    /// Compute (or fetch) the per-exit path-count vector for a node.
    fn compute_paths(&mut self, id: NodeId) -> Vec<i128> {
        // Cache hit?
        if let Some(p) = self.path_counts.paths.get(&id) {
            return p.clone();
        }
        let result = match id {
            DONT_CARE => vec![2i128],
            FORK => vec![1i128, 1i128],
            _ => {
                // Internal: recurse on A-target and each B-target, then
                // merge per the InstallPathCounts loop.
                let record = self.nodes.record(id).clone();
                match record {
                    NodeRecord::Internal {
                        num_exits,
                        a_conn,
                        b_conns,
                        ..
                    } => {
                        let a_paths = self.compute_paths(a_conn.target);
                        // Canonical-form invariant: A-conn return map is
                        // identity, so the outer loop's `i` indexes both
                        // a_paths and b_conns directly.
                        debug_assert!(
                            is_identity_return_map(self.return_maps.body(a_conn.return_map)),
                            "A-connection return map must be identity in canonical form"
                        );
                        debug_assert_eq!(
                            a_paths.len(),
                            b_conns.len(),
                            "A-target exit count must equal B-connection count"
                        );

                        let mut out = vec![0i128; num_exits as usize];
                        for (i, b_conn) in b_conns.iter().enumerate() {
                            let b_paths = self.compute_paths(b_conn.target);
                            // Snapshot the B-conn return map (immutable borrow
                            // of return_maps would otherwise outlive the loop).
                            let b_rm: Vec<u32> = self.return_maps.body(b_conn.return_map).to_vec();
                            debug_assert_eq!(
                                b_paths.len(),
                                b_rm.len(),
                                "B-target exit count must equal its return-map length"
                            );
                            let a_paths_i = a_paths[i];
                            for (j, &k) in b_rm.iter().enumerate() {
                                let term = a_paths_i.checked_mul(b_paths[j]).expect(
                                    "path_count: i128 overflow multiplying \
                                     A-path × B-path",
                                );
                                let slot = &mut out[k as usize];
                                *slot = slot.checked_add(term).expect(
                                    "path_count: i128 overflow accumulating \
                                     into exit bucket",
                                );
                            }
                        }
                        out
                    }
                    NodeRecord::DontCare | NodeRecord::Fork => {
                        // Unreachable: handled above by NodeId match on the
                        // sentinel ids. Keeping a defensive arm.
                        unreachable!("leaf nodes have fixed sentinel NodeIds");
                    }
                }
            }
        };
        self.path_counts.paths.insert(id, result.clone());
        result
    }
}

fn is_identity_return_map(body: &[u32]) -> bool {
    body.iter().enumerate().all(|(i, &v)| v == i as u32)
}
