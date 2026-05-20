//! `Manager`: the aggregate root for the canonical CFLOBDD store, dedup
//! tables, and memo caches.
//!
//! All public operations are methods on `&mut Manager`. There is no
//! global state; multiple managers can coexist with disjoint id spaces.
//!
//! # Top-level representation
//!
//! Internally a CFLOBDD is split into:
//!
//! - A *grounded node* (`NodeId`) that gives the structure: which inputs
//!   funnel to which "exit slot." The level of the root determines how
//!   many variables are addressable: 2^level.
//! - A *value map* (`ReturnMapId`) that translates the root node's exit
//!   indices into the function's actual output values (here: 0 or 1).

use crate::node::{BConnVec, Connection, NodeId, NodeRecord, NodeStore, DONT_CARE, FORK};
use crate::pair_product::PairProductMemo;
use crate::path_count::PathCountCache;
use crate::return_map::{ReturnMapId, ReturnMapStore, ReturnMapVec};
use hashbrown::HashMap;

/// A user-facing CFLOBDD: a (root node, value map) pair, both interned.
///
/// `Bdd` is `Copy`: it's two `u32`s. Its meaning is only valid relative
/// to the `Manager` that produced it; mixing ids across managers is a
/// logic error.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Bdd {
    pub(crate) root: NodeId,
    pub(crate) values: ReturnMapId,
}

/// The aggregate root.
pub struct Manager {
    pub(crate) nodes: NodeStore,
    pub(crate) return_maps: ReturnMapStore,
    /// Compile-time max level. A `Bdd` constructed via `mk_*` operations
    /// at this manager has root at exactly `level`. `level` determines
    /// the variable count: `2^level`.
    level: u8,
    /// Cache of `NoDistinctionNode[k]` for k in 0..=level. Populated lazily.
    no_distinction: Vec<Option<NodeId>>,
    /// PairProduct memo cache: keyed on (n1, n2). Symmetric lookup
    /// (we also check (n2, n1) and flip).
    pub(crate) pair_product_cache: HashMap<(NodeId, NodeId), PairProductMemo>,
    /// Path-count memo cache (M7). Lazily populated on first
    /// `path_count` call; safe to keep across calls because canonical
    /// NodeIds are immutable for the manager's lifetime (no GC in
    /// Phase 1).
    pub(crate) path_counts: PathCountCache,
}

impl Manager {
    /// Create a manager fixed at the given top level. Total addressable
    /// variables = `2^level`.
    pub fn new(level: u8) -> Self {
        assert!(level >= 1, "level must be ≥ 1; level 0 is degenerate");
        assert!(
            level <= 30,
            "level > 30 implies > 1B variables; not supported"
        );
        Self {
            nodes: NodeStore::new(),
            return_maps: ReturnMapStore::new(),
            level,
            no_distinction: vec![None; (level + 1) as usize],
            pair_product_cache: HashMap::new(),
            path_counts: PathCountCache::new(),
        }
    }

    /// The top level this manager was constructed at.
    pub fn level(&self) -> u8 {
        self.level
    }

    /// Total addressable variables: $2^{\text{level}}$.
    pub fn total_vars(&self) -> u32 {
        1u32 << self.level
    }

    // -- Diagnostics ---------------------------------------------------

    pub fn node_count(&self) -> usize {
        self.nodes.count()
    }

    pub fn return_map_count(&self) -> usize {
        self.return_maps.count()
    }

    // -- NoDistinction helpers (level-k all-don't-care) ---------------

    /// `NoDistinctionNode[k]`: the level-$k$ node whose function is
    /// constant (all paths go to a single exit, exit 0).
    pub(crate) fn no_distinction(&mut self, k: u8) -> NodeId {
        if k == 0 {
            return DONT_CARE;
        }
        if let Some(id) = self.no_distinction[k as usize] {
            return id;
        }
        let child = self.no_distinction(k - 1);
        let map = self.return_maps.singleton(0);
        let a = Connection {
            target: child,
            return_map: map,
        };
        let mut b_conns = BConnVec::new();
        b_conns.push(Connection {
            target: child,
            return_map: map,
        });
        let id = self.nodes.intern_internal(k, 1, a, b_conns);
        self.no_distinction[k as usize] = Some(id);
        id
    }

    // -- Public constants ---------------------------------------------

    /// The constant-true CFLOBDD over `2^level` variables.
    pub fn mk_true(&mut self) -> Bdd {
        let root = self.no_distinction(self.level);
        let values = self.return_maps.singleton(1);
        Bdd { root, values }
    }

    /// The constant-false CFLOBDD over `2^level` variables.
    pub fn mk_false(&mut self) -> Bdd {
        let root = self.no_distinction(self.level);
        let values = self.return_maps.singleton(0);
        Bdd { root, values }
    }

    // -- Projection ---------------------------------------------------

    /// `mk_proj(i)`: the CFLOBDD representing the literal $x_i$ over
    /// `2^level` variables.
    ///
    /// Variable indices are 0 ≤ i < 2^level. Out-of-range panics in
    /// debug builds.
    pub fn mk_proj(&mut self, var_index: u32) -> Bdd {
        assert!(
            var_index < self.total_vars(),
            "var_index {} out of range (total_vars = {})",
            var_index,
            self.total_vars()
        );
        let root = self.mk_distinction(self.level, var_index);
        // Value map is the identity [0, 1]: exit 0 of the structural node
        // means "x_i = 0", exit 1 means "x_i = 1".
        let values = self.return_maps.identity(2);
        Bdd { root, values }
    }

    /// MkDistinction(level, i): the structural level-$k$ node whose
    /// function is $x_i$ where $i \in [0, 2^k)$. Two exits.
    fn mk_distinction(&mut self, level: u8, i: u32) -> NodeId {
        if level == 0 {
            debug_assert!(i == 0, "level-0 distinction requires i = 0");
            return FORK;
        }
        let half = 1u32 << (level - 1);
        if i < half {
            // i falls in the A-connection range.
            let a_target = self.mk_distinction(level - 1, i);
            let id_map = self.return_maps.identity(2);
            let a = Connection {
                target: a_target,
                return_map: id_map,
            };
            let nd = self.no_distinction(level - 1);
            let map_0: ReturnMapId = {
                let mut v = ReturnMapVec::new();
                v.push(0);
                self.return_maps.intern(v)
            };
            let map_1: ReturnMapId = {
                let mut v = ReturnMapVec::new();
                v.push(1);
                self.return_maps.intern(v)
            };
            let mut b_conns = BConnVec::new();
            b_conns.push(Connection {
                target: nd,
                return_map: map_0,
            });
            b_conns.push(Connection {
                target: nd,
                return_map: map_1,
            });
            self.nodes.intern_internal(level, 2, a, b_conns)
        } else {
            // i falls in the B-connection range; mask off the high bit.
            let nd = self.no_distinction(level - 1);
            let id1 = self.return_maps.singleton(0);
            let a = Connection {
                target: nd,
                return_map: id1,
            };
            let b_target = self.mk_distinction(level - 1, i ^ half);
            let id2 = self.return_maps.identity(2);
            let mut b_conns = BConnVec::new();
            b_conns.push(Connection {
                target: b_target,
                return_map: id2,
            });
            self.nodes.intern_internal(level, 2, a, b_conns)
        }
    }

    // -- Equality on canonical Bdds ----------------------------------

    /// Structural equality. Because the canonical store hash-conses
    /// every node and return map, equality reduces to id equality.
    pub fn eq(&self, f: Bdd, g: Bdd) -> bool {
        f == g
    }

    /// Evaluate a Bdd on a truth assignment.
    ///
    /// `assignment[i]` is the value of variable $x_i$. The slice must
    /// have length exactly `total_vars()`.
    pub fn evaluate(&self, bdd: Bdd, assignment: &[bool]) -> bool {
        assert_eq!(
            assignment.len() as u32,
            self.total_vars(),
            "assignment length must equal total_vars"
        );
        let exit = self.evaluate_node(bdd.root, assignment);
        let v = self.return_maps.body(bdd.values)[exit as usize];
        v != 0
    }

    /// Recursive evaluation returning the exit index of the structural node.
    fn evaluate_node(&self, node: NodeId, assignment: &[bool]) -> u32 {
        match self.nodes.record(node) {
            NodeRecord::DontCare => {
                debug_assert_eq!(assignment.len(), 1);
                0
            }
            NodeRecord::Fork => {
                debug_assert_eq!(assignment.len(), 1);
                if assignment[0] {
                    1
                } else {
                    0
                }
            }
            NodeRecord::Internal {
                level,
                a_conn,
                b_conns,
                ..
            } => {
                let n = 1usize << level; // total vars at this level
                debug_assert_eq!(assignment.len(), n);
                let half = n / 2;
                let (left, right) = assignment.split_at(half);
                // Walk A-connection on the left half.
                let a_exit = self.evaluate_node(a_conn.target, left);
                // a_exit is an exit of the A-conn target; translate
                // through a_conn.return_map to pick a B-connection
                // index of *this* node.
                let b_idx = self.return_maps.body(a_conn.return_map)[a_exit as usize];
                let b = b_conns[b_idx as usize];
                // Walk that B-connection on the right half.
                let b_exit = self.evaluate_node(b.target, right);
                // Translate through b.return_map to get this node's exit.
                self.return_maps.body(b.return_map)[b_exit as usize]
            }
        }
    }

    // -- Diagnostics: structural size of a Bdd ------------------------

    /// Count distinct nodes reachable from this Bdd's root, including
    /// the level-0 leaves (DontCare and Fork) when reachable.
    pub fn reachable_node_count(&self, bdd: Bdd) -> usize {
        let mut seen = hashbrown::HashSet::new();
        self.walk(bdd.root, &mut seen);
        seen.len()
    }

    /// Count edges (A-connections + B-connections) over the reachable
    /// subgraph. Each distinct internal node contributes `1 + |B-conns|`.
    /// Leaves contribute zero.
    pub fn reachable_edge_count(&self, bdd: Bdd) -> usize {
        let mut seen = hashbrown::HashSet::new();
        self.walk(bdd.root, &mut seen);
        seen.iter()
            .map(|id| match self.nodes.record(*id) {
                NodeRecord::Internal { b_conns, .. } => 1 + b_conns.len(),
                _ => 0,
            })
            .sum()
    }

    fn walk(&self, id: NodeId, seen: &mut hashbrown::HashSet<NodeId>) {
        if !seen.insert(id) {
            return;
        }
        if let NodeRecord::Internal {
            a_conn, b_conns, ..
        } = self.nodes.record(id)
        {
            self.walk(a_conn.target, seen);
            for b in b_conns {
                self.walk(b.target, seen);
            }
        }
    }
}
