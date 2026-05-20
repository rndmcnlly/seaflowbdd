//! Canonical node store. NodeId is a stable index; NodeRecord is the
//! payload. Hash-consing is enforced by making the storage and dedup
//! table module-private: only this module produces `NodeId`s.
//!
//! Layout decisions (Phase 1):
//!
//! - `NodeRecord` is an enum with three variants: `Internal`, `Fork`,
//!   `DontCare`. Fork and DontCare are unit variants. Internal carries
//!   level + AConnection + a `SmallVec` of BConnections.
//! - A `Connection` is `(NodeId, ReturnMapId)`, packed as a `[u32; 2]`.
//!   For internal nodes, BConnections almost always number ≤ 4, so SBO
//!   keeps them inline in the record.
//!
//! This is deliberately not the most cache-compact layout possible: the
//! enum discriminant + SmallVec header bloats internal nodes to ~64
//! bytes. Phase 2 will explore packing the SmallVec inline payload into
//! a side arena indexed by `(start, len)` pair if profiling shows
//! per-node bytes mattering. For now: legibility first.

use crate::return_map::ReturnMapId;
use hashbrown::HashTable;
use rustc_hash::FxHasher;
use smallvec::SmallVec;
use std::hash::{BuildHasher, BuildHasherDefault, Hasher};

/// Stable, copyable index into a `Manager`'s node store.
///
/// `NodeId(0)` and `NodeId(1)` are reserved for the level-0 leaves
/// (DontCare and Fork respectively); see `NodeStore::new`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct NodeId(pub(crate) u32);

/// A directed edge from a parent to a child. The `target` is the child
/// node; the `return_map` translates the child's exit indices into the
/// parent's exit space (or, in the case of A-connection, into B-connection
/// indices).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Connection {
    pub target: NodeId,
    pub return_map: ReturnMapId,
}

impl Connection {
    /// Pack into a u64: target in low 32 bits, return_map in high 32.
    /// Used for fast hashing.
    #[inline(always)]
    fn pack(&self) -> u64 {
        (self.target.0 as u64) | ((self.return_map.0 as u64) << 32)
    }
}

impl std::hash::Hash for Connection {
    #[inline]
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        state.write_u64(self.pack());
    }
}

/// Inline storage for B-connections. ≤ 4 covers typical boolean ops
/// (numExits is at most 2 for boolean CFLOBDDs after Reduce).
pub(crate) type BConnVec = SmallVec<[Connection; 4]>;

/// The node payload. Internal carries data; the two leaves are unit
/// variants.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) enum NodeRecord {
    /// Level-0 don't-care leaf: $f(x) = c$ for some constant. Singleton.
    DontCare,
    /// Level-0 fork leaf: $f(x) = x$. Singleton.
    Fork,
    /// Level-$k$ internal node ($k \geq 1$). `num_exits` is cached so we
    /// don't have to walk the return maps to recover it.
    Internal {
        level: u8,
        num_exits: u32,
        a_conn: Connection,
        b_conns: BConnVec,
    },
}

impl std::hash::Hash for NodeRecord {
    #[inline]
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        match self {
            NodeRecord::DontCare => state.write_u8(0),
            NodeRecord::Fork => state.write_u8(1),
            NodeRecord::Internal {
                level,
                num_exits,
                a_conn,
                b_conns,
            } => {
                // Mix level, num_exits, and a_conn into a single u64 + u32.
                state.write_u8(2);
                state.write_u32(*level as u32);
                state.write_u32(*num_exits);
                state.write_u64(a_conn.pack());
                // Hash B-conns as packed u64s. Length first to avoid
                // collision between e.g. [(a,b)] and [(a,b), zero].
                state.write_usize(b_conns.len());
                for b in b_conns {
                    state.write_u64(b.pack());
                }
            }
        }
    }
}

/// Canonical node store + dedup table. Module-private; only the factory
/// methods on `Manager` produce `NodeId`s.
///
/// The dedup table is a `HashTable` of `NodeId`s rather than a
/// `HashMap<NodeRecord, NodeId>`. This lets us look up by *probing*
/// against existing records in `records` without ever constructing a
/// `NodeRecord` clone for the key. Each lookup hashes the candidate
/// fields once, walks the bucket via Eq closure that compares fields
/// against `records[id]`. Insert appends to `records` and adds the
/// `NodeId` to the table.
pub(crate) struct NodeStore {
    /// Indexed by `NodeId.0`. Index 0 = DontCare, index 1 = Fork.
    records: Vec<NodeRecord>,
    /// Dedup table for internal nodes. Stores only ids; the records
    /// themselves live in `records`.
    index: HashTable<NodeId>,
    /// Hash builder used to derive bucket indices. We compute hashes
    /// directly on candidate fields and on existing `NodeRecord`s.
    hasher: BuildHasherDefault<FxHasher>,
}

/// Sentinel NodeIds for the two level-0 leaves.
pub(crate) const DONT_CARE: NodeId = NodeId(0);
pub(crate) const FORK: NodeId = NodeId(1);

impl NodeStore {
    pub(crate) fn new() -> Self {
        let records = vec![NodeRecord::DontCare, NodeRecord::Fork];
        Self {
            records,
            index: HashTable::new(),
            hasher: BuildHasherDefault::default(),
        }
    }

    /// Hash an Internal candidate without constructing a NodeRecord.
    #[inline]
    fn hash_candidate(
        &self,
        level: u8,
        num_exits: u32,
        a_conn: Connection,
        b_conns: &[Connection],
    ) -> u64 {
        let mut h = self.hasher.build_hasher();
        // Mirror the body of `Hash for NodeRecord::Internal`.
        h.write_u8(2);
        h.write_u32(level as u32);
        h.write_u32(num_exits);
        h.write_u64(a_conn.pack());
        h.write_usize(b_conns.len());
        for b in b_conns {
            h.write_u64(b.pack());
        }
        h.finish()
    }

    /// Hash-cons an internal node. Identical structure returns the same id.
    pub(crate) fn intern_internal(
        &mut self,
        level: u8,
        num_exits: u32,
        a_conn: Connection,
        b_conns: BConnVec,
    ) -> NodeId {
        debug_assert!(level >= 1, "internal nodes must be level ≥ 1");
        debug_assert!(num_exits >= 1, "internal nodes must have ≥ 1 exit");
        debug_assert!(
            !b_conns.is_empty(),
            "internal nodes must have ≥ 1 B-connection"
        );

        let hash = self.hash_candidate(level, num_exits, a_conn, &b_conns);
        // Probe by hash + field equality, without ever constructing a
        // NodeRecord for the lookup key.
        if let Some(&id) = self.index.find(hash, |&existing_id| {
            match &self.records[existing_id.0 as usize] {
                NodeRecord::Internal {
                    level: el,
                    num_exits: en,
                    a_conn: ea,
                    b_conns: eb,
                } => {
                    *el == level
                        && *en == num_exits
                        && *ea == a_conn
                        && eb.as_slice() == b_conns.as_slice()
                }
                _ => false,
            }
        }) {
            return id;
        }
        // Miss: build the record once, push to records, insert id.
        let id = NodeId(self.records.len() as u32);
        let record = NodeRecord::Internal {
            level,
            num_exits,
            a_conn,
            b_conns,
        };
        self.records.push(record);
        // Use the same hash; rehasher closure looks up the record by id.
        let records_ptr = &self.records;
        let hasher = &self.hasher;
        self.index.insert_unique(hash, id, |&existing_id| {
            let mut h = hasher.build_hasher();
            std::hash::Hash::hash(&records_ptr[existing_id.0 as usize], &mut h);
            h.finish()
        });
        id
    }

    pub(crate) fn record(&self, id: NodeId) -> &NodeRecord {
        &self.records[id.0 as usize]
    }

    /// Number of exits for any node (1 for DontCare, 2 for Fork, cached
    /// on Internal).
    #[allow(dead_code)]
    pub(crate) fn num_exits(&self, id: NodeId) -> u32 {
        match self.record(id) {
            NodeRecord::DontCare => 1,
            NodeRecord::Fork => 2,
            NodeRecord::Internal { num_exits, .. } => *num_exits,
        }
    }

    /// Level: 0 for both leaves, cached on Internal.
    pub(crate) fn level(&self, id: NodeId) -> u8 {
        match self.record(id) {
            NodeRecord::DontCare | NodeRecord::Fork => 0,
            NodeRecord::Internal { level, .. } => *level,
        }
    }

    pub(crate) fn count(&self) -> usize {
        self.records.len()
    }
}
