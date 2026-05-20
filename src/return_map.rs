//! ReturnMap: a small contiguous array of `u32` exit indices, hash-consed.
//!
//! In CFLOBDD vocabulary, a return map on a `Connection` translates the
//! child's exit index into the parent's exit index. We canonicalize:
//! structurally equal return maps share a single `ReturnMapId`.
//!
//! Most return maps are tiny (≤ 4 entries), so we store the data inline
//! when small via `SmallVec`. Dedup is keyed on the contents.

use rustc_hash::FxHashMap;
use smallvec::SmallVec;

/// Stable, copyable index into a `Manager`'s return-map store.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ReturnMapId(pub(crate) u32);

/// Inline storage for small return maps. 4 entries covers the common case.
pub(crate) type ReturnMapVec = SmallVec<[u32; 4]>;

/// Canonical store for return maps. Stores each unique return-map body once;
/// hands out `ReturnMapId`s.
pub(crate) struct ReturnMapStore {
    /// Indexed by `ReturnMapId.0`.
    bodies: Vec<ReturnMapVec>,
    /// Dedup table: contents -> id. The key clones are unavoidable for the
    /// initial implementation; can be optimized later with a custom raw-entry
    /// scheme that hashes the slice without owning it.
    index: FxHashMap<ReturnMapVec, ReturnMapId>,
}

impl ReturnMapStore {
    pub(crate) fn new() -> Self {
        Self {
            bodies: Vec::new(),
            index: FxHashMap::default(),
        }
    }

    /// Hash-cons a return-map body. Identical contents return the same id.
    pub(crate) fn intern(&mut self, body: ReturnMapVec) -> ReturnMapId {
        if let Some(id) = self.index.get(&body) {
            return *id;
        }
        let id = ReturnMapId(self.bodies.len() as u32);
        self.index.insert(body.clone(), id);
        self.bodies.push(body);
        id
    }

    pub(crate) fn body(&self, id: ReturnMapId) -> &[u32] {
        &self.bodies[id.0 as usize]
    }

    #[allow(dead_code)]
    pub(crate) fn len(&self, id: ReturnMapId) -> usize {
        self.bodies[id.0 as usize].len()
    }

    /// Identity return map of length `n`: [0, 1, 2, ..., n-1].
    /// Used pervasively when constructing nodes.
    pub(crate) fn identity(&mut self, n: u32) -> ReturnMapId {
        let body: ReturnMapVec = (0..n).collect();
        self.intern(body)
    }

    pub(crate) fn singleton(&mut self, value: u32) -> ReturnMapId {
        let mut body = ReturnMapVec::new();
        body.push(value);
        self.intern(body)
    }

    pub(crate) fn count(&self) -> usize {
        self.bodies.len()
    }
}
