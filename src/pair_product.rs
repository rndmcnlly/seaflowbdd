//! PairProduct + ApplyAndReduce.
//!
//! PairProduct is the structural cross-product of two CFLOBDDs at the
//! same level. It returns a single combined node whose exits are pairs
//! `(i, j)` of original exit indices (one from each operand). The
//! caller (ApplyAndReduce) then collapses these pairs by applying a
//! pointwise operator to the corresponding values.
//!
//! Implementation: straight recursion mirroring `cross_product.cpp`.
//! Memoization lives on `Manager`.

use crate::manager::{Bdd, Manager};
use crate::node::{BConnVec, Connection, NodeId, NodeRecord, DONT_CARE, FORK};
use crate::return_map::ReturnMapVec;
use hashbrown::HashMap;

/// A pair-product map: a list of `(i, j)` pairs, where the result
/// node's exit `k` corresponds to original-operand exits `(i, j)`.
pub(crate) type PairProductMap = smallvec::SmallVec<[(u32, u32); 4]>;

/// Cached PairProduct result.
#[derive(Clone)]
pub(crate) struct PairProductMemo {
    pub node: NodeId,
    pub pair_map: PairProductMap,
}

/// Boolean op as a flat 2x2 truth table. Index `op[a][b]` for
/// `a, b ∈ {0, 1}`.
pub type BoolOp = [[u32; 2]; 2];

pub const AND_OP: BoolOp = [[0, 0], [0, 1]];
pub const OR_OP: BoolOp = [[0, 1], [1, 1]];
pub const XOR_OP: BoolOp = [[0, 1], [1, 0]];
pub const IFF_OP: BoolOp = [[1, 0], [0, 1]];
pub const IMPLIES_OP: BoolOp = [[1, 1], [0, 1]];
pub const NAND_OP: BoolOp = [[1, 1], [1, 0]];
pub const NOR_OP: BoolOp = [[1, 0], [0, 0]];

impl Manager {
    /// PairProduct at the node level. Memoized; symmetric in (n1, n2)
    /// because the cache also checks `(n2, n1)` and flips the pair map.
    fn pair_product(&mut self, n1: NodeId, n2: NodeId) -> (NodeId, PairProductMap) {
        // Check both orderings in the cache.
        if let Some(memo) = self.pair_product_cache.get(&(n1, n2)) {
            return (memo.node, memo.pair_map.clone());
        }
        if let Some(memo) = self.pair_product_cache.get(&(n2, n1)) {
            let flipped: PairProductMap = memo.pair_map.iter().map(|&(a, b)| (b, a)).collect();
            return (memo.node, flipped);
        }

        let (node, pair_map) = self.pair_product_uncached(n1, n2);

        let memo = PairProductMemo {
            node,
            pair_map: pair_map.clone(),
        };
        self.pair_product_cache.insert((n1, n2), memo);
        (node, pair_map)
    }

    fn pair_product_uncached(&mut self, n1: NodeId, n2: NodeId) -> (NodeId, PairProductMap) {
        // Dispatch by node kind.
        let r1 = self.nodes.record(n1).clone();
        let r2 = self.nodes.record(n2).clone();
        match (r1, r2) {
            (NodeRecord::DontCare, NodeRecord::DontCare) => {
                let mut m = PairProductMap::new();
                m.push((0, 0));
                (DONT_CARE, m)
            }
            (NodeRecord::Fork, NodeRecord::Fork) => {
                let mut m = PairProductMap::new();
                m.push((0, 0));
                m.push((1, 1));
                (FORK, m)
            }
            (NodeRecord::Fork, NodeRecord::DontCare) => {
                let mut m = PairProductMap::new();
                m.push((0, 0));
                m.push((1, 0));
                (FORK, m)
            }
            (NodeRecord::DontCare, NodeRecord::Fork) => {
                let mut m = PairProductMap::new();
                m.push((0, 0));
                m.push((0, 1));
                (FORK, m)
            }
            // One Internal, one leaf: shouldn't happen at matched levels.
            (NodeRecord::Internal { .. }, NodeRecord::DontCare)
            | (NodeRecord::Internal { .. }, NodeRecord::Fork)
            | (NodeRecord::DontCare, NodeRecord::Internal { .. })
            | (NodeRecord::Fork, NodeRecord::Internal { .. }) => {
                panic!(
                    "PairProduct received nodes at mismatched levels: {} and {}",
                    self.nodes.level(n1),
                    self.nodes.level(n2)
                );
            }
            (
                NodeRecord::Internal {
                    level: l1,
                    num_exits: ne1,
                    a_conn: a1,
                    b_conns: b1,
                },
                NodeRecord::Internal {
                    level: l2,
                    num_exits: ne2,
                    a_conn: a2,
                    b_conns: b2,
                },
            ) => {
                debug_assert_eq!(l1, l2, "PairProduct: levels must match");
                let level = l1;

                // NoDistinction shortcuts: if either side is the
                // all-don't-care node at this level, the result is the
                // other side, with a pair map [(0, k) for k in 0..ne_other]
                // or [(k, 0) for k in 0..ne_self].
                let nd = self.no_distinction(level);
                if n1 == nd && n2 == nd {
                    let mut m = PairProductMap::new();
                    m.push((0, 0));
                    return (n1, m);
                }
                if n1 == nd {
                    let mut m = PairProductMap::new();
                    for k in 0..ne2 {
                        m.push((0, k));
                    }
                    return (n2, m);
                }
                if n2 == nd {
                    let mut m = PairProductMap::new();
                    for k in 0..ne1 {
                        m.push((k, 0));
                    }
                    return (n1, m);
                }

                // Cross product on A-connections.
                let (a_node, a_pair_map) = self.pair_product(a1.target, a2.target);

                // Build A-connection: identity return map of length |a_pair_map|.
                let a_return_map = self.return_maps.identity(a_pair_map.len() as u32);

                // For each entry in a_pair_map, recurse on B-connections.
                // Build the result's B-conn list and the parent's exit
                // pair map (i.e. the per-result-exit (i, j) tuple).
                let mut new_b_conns = BConnVec::new();
                let mut pair_map = PairProductMap::new();
                // Flat lookup: dedup result exits by (c1, c2) pairs.
                // The space is ne1 * ne2; for boolean ops these are
                // tiny. Use a HashMap for generality; flat 2D would
                // be a micro-opt for Phase 2.
                let mut exit_dedup: HashMap<(u32, u32), u32> = HashMap::new();

                let a1_map_body: Vec<u32> = self.return_maps.body(a1.return_map).to_vec();
                let a2_map_body: Vec<u32> = self.return_maps.body(a2.return_map).to_vec();
                debug_assert!(
                    is_identity_return_map(&a1_map_body),
                    "A-conn return map of n1 must be identity (got {:?})",
                    a1_map_body
                );
                debug_assert!(
                    is_identity_return_map(&a2_map_body),
                    "A-conn return map of n2 must be identity (got {:?})",
                    a2_map_body
                );

                for &(b1_idx, b2_idx) in &a_pair_map {
                    let b1c = b1[b1_idx as usize];
                    let b2c = b2[b2_idx as usize];
                    let (b_node, b_pair_map) = self.pair_product(b1c.target, b2c.target);
                    // For the B-conn's return map: each entry of
                    // b_pair_map corresponds to a child exit. We need
                    // to map that child exit through the original
                    // return maps and dedup against the running pair_map.
                    let b1c_map_body: Vec<u32> = self.return_maps.body(b1c.return_map).to_vec();
                    let b2c_map_body: Vec<u32> = self.return_maps.body(b2c.return_map).to_vec();

                    let mut b_return_map = ReturnMapVec::new();
                    for &(c1, c2) in &b_pair_map {
                        let parent_exit_1 = b1c_map_body[c1 as usize];
                        let parent_exit_2 = b2c_map_body[c2 as usize];
                        let key = (parent_exit_1, parent_exit_2);
                        let pos = if let Some(&p) = exit_dedup.get(&key) {
                            p
                        } else {
                            let p = pair_map.len() as u32;
                            pair_map.push(key);
                            exit_dedup.insert(key, p);
                            p
                        };
                        b_return_map.push(pos);
                    }
                    let b_return_id = self.return_maps.intern(b_return_map);
                    new_b_conns.push(Connection {
                        target: b_node,
                        return_map: b_return_id,
                    });
                }

                let num_exits = pair_map.len() as u32;
                let result = self.nodes.intern_internal(
                    level,
                    num_exits,
                    Connection {
                        target: a_node,
                        return_map: a_return_map,
                    },
                    new_b_conns,
                );
                let _ = (ne1, ne2); // silence unused warnings
                (result, pair_map)
            }
        }
    }

    /// Apply a binary boolean op pointwise to two CFLOBDDs and reduce
    /// the result. The two operands must be at this manager's top
    /// level.
    pub fn apply(&mut self, f: Bdd, g: Bdd, op: BoolOp) -> Bdd {
        // PairProduct on roots.
        let (combined, pair_map) = self.pair_product(f.root, g.root);

        // Build the new value map and the reduction map. For each entry
        // in pair_map, apply op to the corresponding values in f.values
        // and g.values, then dedup against the running new value list.
        let f_vals: Vec<u32> = self.return_maps.body(f.values).to_vec();
        let g_vals: Vec<u32> = self.return_maps.body(g.values).to_vec();

        // For boolean ops the range has at most 2 values; track slot
        // indices for 0 and 1 explicitly (matching the C++ trick).
        let mut new_vals: ReturnMapVec = ReturnMapVec::new();
        let mut reduction: Vec<u32> = Vec::with_capacity(pair_map.len());
        let mut slot: [i32; 2] = [-1, -1];
        for &(i, j) in &pair_map {
            let v1 = f_vals[i as usize];
            let v2 = g_vals[j as usize];
            debug_assert!(v1 < 2 && v2 < 2, "boolean op operands must be 0/1");
            let val = op[v1 as usize][v2 as usize];
            let s = &mut slot[val as usize];
            let pos = if *s < 0 {
                let p = new_vals.len() as u32;
                new_vals.push(val);
                *s = p as i32;
                p
            } else {
                *s as u32
            };
            reduction.push(pos);
        }

        let new_num_exits = new_vals.len() as u32;
        let new_values_id = self.return_maps.intern(new_vals);

        // Reduce the combined node by the reduction map.
        let reduced = self.reduce(combined, &reduction, new_num_exits);

        Bdd {
            root: reduced,
            values: new_values_id,
        }
    }

    pub fn and(&mut self, f: Bdd, g: Bdd) -> Bdd {
        self.apply(f, g, AND_OP)
    }
    pub fn or(&mut self, f: Bdd, g: Bdd) -> Bdd {
        self.apply(f, g, OR_OP)
    }
    pub fn xor(&mut self, f: Bdd, g: Bdd) -> Bdd {
        self.apply(f, g, XOR_OP)
    }
    pub fn iff(&mut self, f: Bdd, g: Bdd) -> Bdd {
        self.apply(f, g, IFF_OP)
    }
    pub fn implies(&mut self, f: Bdd, g: Bdd) -> Bdd {
        self.apply(f, g, IMPLIES_OP)
    }
    pub fn nand(&mut self, f: Bdd, g: Bdd) -> Bdd {
        self.apply(f, g, NAND_OP)
    }
    pub fn nor(&mut self, f: Bdd, g: Bdd) -> Bdd {
        self.apply(f, g, NOR_OP)
    }

    /// Logical negation: swap the value map. No structural work.
    pub fn not(&mut self, f: Bdd) -> Bdd {
        let body: ReturnMapVec = self
            .return_maps
            .body(f.values)
            .iter()
            .map(|&v| 1 - v)
            .collect();
        // For identity-or-non-identity ordering, we need to also dedup
        // and possibly recanonicalize via reduce. But because the value
        // map already had distinct entries (canonical form invariant
        // for boolean ops: 1 or 2 distinct values), flipping each bit
        // preserves distinctness. So a straight swap is correct.
        //
        // However: the C++ engine's MkNot sometimes produces a Bdd
        // whose value map is in non-canonical order (e.g. [1,0] instead
        // of [0,1]). seaflowbdd's `Bdd` equality is on (root, values_id),
        // so two Bdds that semantically are negations of each other
        // would not necessarily be equal. The simplest fix: after a
        // not, if the values list comes out in a non-canonical order
        // (e.g. [1, 0]), apply Reduce + permutation. For boolean v1
        // we take the simple route: hash-cons the swapped map and
        // accept that two equivalent Bdds may have different `values`
        // ids depending on construction history, and rely on a
        // canonicalizing path through `apply` where it matters.
        let new_id = self.return_maps.intern(body);
        Bdd {
            root: f.root,
            values: new_id,
        }
    }
}

fn is_identity_return_map(body: &[u32]) -> bool {
    for (i, &v) in body.iter().enumerate() {
        if v != i as u32 {
            return false;
        }
    }
    true
}
