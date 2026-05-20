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
use hashbrown::HashMap;
use smallvec::SmallVec;

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
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct Connection {
    pub target: NodeId,
    pub return_map: ReturnMapId,
}

/// Inline storage for B-connections. ≤ 4 covers typical boolean ops
/// (numExits is at most 2 for boolean CFLOBDDs after Reduce).
pub(crate) type BConnVec = SmallVec<[Connection; 4]>;

/// The node payload. Internal carries data; the two leaves are unit
/// variants.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
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

/// Canonical node store + dedup table. Module-private; only the factory
/// methods on `Manager` produce `NodeId`s.
pub(crate) struct NodeStore {
    /// Indexed by `NodeId.0`. Index 0 = DontCare, index 1 = Fork.
    records: Vec<NodeRecord>,
    /// Dedup table for internal nodes. Leaves are not indexed here (they
    /// have fixed ids, looked up directly).
    index: HashMap<NodeRecord, NodeId>,
}

/// Sentinel NodeIds for the two level-0 leaves.
pub(crate) const DONT_CARE: NodeId = NodeId(0);
pub(crate) const FORK: NodeId = NodeId(1);

impl NodeStore {
    pub(crate) fn new() -> Self {
        let records = vec![NodeRecord::DontCare, NodeRecord::Fork];
        Self {
            records,
            index: HashMap::new(),
        }
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
        let key = NodeRecord::Internal {
            level,
            num_exits,
            a_conn,
            b_conns,
        };
        if let Some(id) = self.index.get(&key) {
            return *id;
        }
        let id = NodeId(self.records.len() as u32);
        self.index.insert(key.clone(), id);
        self.records.push(key);
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
