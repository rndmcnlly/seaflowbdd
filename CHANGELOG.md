# Changelog

## 0.2.0

Phase 2 optimization release. All measured boolean ops now run within 2× of (or faster than) the trishullab/cflobdd C++ reference engine, with `projection_construction_32` running 11.8× faster.

Internal changes:

- Added a Reduce memo cache (mirrors the C++ engine's `reduceCache`).
- Switched the node dedup table to hashbrown's `HashTable` with custom hash and eq closures, eliminating `NodeRecord` key cloning on lookups.
- Precomputed common return maps (`[0]`, `[1]`, `[0, 1]`) at `Manager::new`.
- Replaced `Vec<u32>::to_vec()` patterns with stack-allocated `SmallVec::from_slice` on hot recursion paths.
- Switched memo and dedup `HashMap`s to `FxHashMap`.
- Custom `Hash` impl on `NodeRecord` that packs `Connection` fields into `u64`s and feeds them to the hasher in a single `write_u64` per connection.

No public API changes; all v0.1 code is source-compatible.

## 0.1.0

Initial release. Unweighted boolean operations: AND, OR, XOR, NOT, NAND, NOR, IFF, IMPLIES, restrict, compose, exists, forall, evaluate, path_count.
