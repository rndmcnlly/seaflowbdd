# seaflowbdd

A native Rust CFLOBDD engine. Name reads as *sea-flow* (C-F-L-O), the way "cflobdd" is pronounced when you've stopped trying.

> **Historical note (v0.1 release).** This document is the original design plan, written before any code existed. It frames the verification strategy in terms of an "oracle" (the C++ trishullab/cflobdd engine), parity testing, and a development workflow that wires both engines together. The released crate has shipped *without* that oracle: structural parity testing was extracted into a private dev tree, and the public release is verified by truth-table semantic correctness, algebraic identity tests, proptest property generation, and brute-force path-count cross-checks. The design rationale in this document still applies; the verification scaffolding sections are best read as a record of how Phase 1 was developed, not as a description of the public test suite.

## Intent

Build a serial Rust CFLOBDD library that:

1. **Phase 1 (parity).** Covers the unweighted CFLOBDD operations (Mk constants, projection, ApplyAndReduce with boolean ops, Restrict, Compose, Reduce, equality, evaluation, path counts, basic matrix/vector ops where they don't require weights) at within one order of magnitude of the C++ engine's wall-clock performance on matched workloads. Correctness is verified node-for-node against the C++ engine via the existing `cflobdd/` bindgen wrapper, which serves as the oracle.

2. **Phase 2 (exceed).** Outperform the C++ engine by an order of magnitude on the same unweighted operations, serially, on a single Apple Silicon core. If Phase 1 already lands inside the C++ engine's performance envelope (plausible given how much of its memory traffic is pre-2010 prototype-tax), Phase 2 reduces to "stack the structural wins until we're 10x ahead."

Weighted operations (the path quantum benchmarks exercise) and parallelism are explicit non-goals for v1. They become candidates for v2 once the unweighted core is solid and fast.

## Reference point

`cflobdd-upstream/` (trishullab/cflobdd, C++) is the correctness and performance oracle. Specifically:

- **Correctness**: every public seaflowbdd operation produces a CFLOBDD whose canonical structure matches what the C++ engine produces for the same inputs. We compare via traversal, not pointer identity. The wrapper at `cflobdd/` (Rust bindgen over the C++ lib) is the bridge we use to drive the oracle from Rust test code.
- **Performance**: same workloads (the existing benchmarks at the repo root: `bench_mult.py`, `bench_cflobdd_mult.py`, `bench_memory.py`, plus whatever microbenchmarks we add) timed against both engines.

The C++ engine has been audited (see prior session notes); its hot path on unweighted PairProduct is partially modernized (flat 2D exit-pair lookup, packed intpairs, static per-level scratch), but is dragged down by:

- Chained `Hashset<>`/`Hashtable<>` with per-bucket `List<ListNode>` heap fragmentation.
- Virtual `IncrRef`/`DecrRef` on every handle copy.
- Per-node heap allocation of `BConnection[]` (almost always ≤3 entries).
- `numPathsToExit` heap-allocated separately per node.
- `std::map` on `MatMultMapBody` (not on the unweighted critical path, but illustrative).
- Recursive Reduce that does two passes over the same structure (Reduce + InstallPathCounts).

These are the structural inefficiencies seaflowbdd is designed to skip from the start, not migrate away from later.

## Bounded contexts and the oracle relationship

Four contexts, kept distinct on purpose:

1. **Construction context** (`seaflowbdd` proper). The engine: builds, reduces, applies, evaluates CFLOBDDs. Owns the canonical store, memo caches, arena machinery.
2. **Oracle context** (`cflobdd/` bindgen wrapper around the C++ engine). A foreign system with its own representation (refcounted handles, virtual dispatch, prototype-tax data structures).
3. **Verification context** (`seaflowbdd/tests/`). Drives both engines on the same abstract specification (formula, truth table, constructor sequence) and compares results.
4. **Benchmarking context** (`seaflowbdd/benches/`). Measures wall-clock cost of fixed operations on fixed inputs against both engines through a uniform façade.

**Anti-corruption layer.** The C++ engine's representation never enters seaflowbdd's core. The verification context contains a small oracle module (`tests/oracle.rs`) that exposes only the operations we need (`oracle_mk_proj`, `oracle_apply_and`, `oracle_eval`, `oracle_path_count`, `oracle_traverse_for_comparison`) with seaflowbdd-shaped argument and return types. Inside, it calls bindgen. Outside, the C++ engine's pointers don't exist.

Concrete enforcement: **`seaflowbdd/Cargo.toml` does not list `cflobdd` as a dependency, even in dev-dependencies.** Only `seaflowbdd/tests/` and `seaflowbdd/benches/` directories may depend on the wrapper crate (via test-only or bench-only Cargo entries). The compiler enforces the boundary.

The temptation will be high to add "just one helper" that takes a C++ handle directly. That helper compounds. We say no.

## Aggregates, entities, value objects

The whole canonical graph plus its caches is **one aggregate**, with `Manager` as the **aggregate root**. Justifications:

- `NodeId` identity is local to a single `Manager`; a `NodeId` from one Manager is gibberish to another. Textbook aggregate boundary.
- Hash-consing is a global invariant (no two distinct `NodeId`s reference structurally equal nodes). Maintaining it touches the dedup table on every node creation. Cross-node invariant ⇒ same aggregate.
- Memo caches reference `NodeId`s and are only valid relative to the canonical store. Same aggregate.
- GC operates on the whole graph at once. Same aggregate.

In Rust this is exactly what the borrow checker wants: `&mut Manager` for mutating ops, `&Manager` for read-only ones.

**The Manager is the only entity in the system.** It has identity (you can have two distinct Managers), mutable state (caches grow, GC compacts), and a lifecycle.

**Everything else is a value object.** Nodes, connections, return maps: defined by their attributes, immutable once interned, interchangeable when structurally equal. We *implement* equality as `NodeId` comparison for speed, but the public contract is structural equality. This matters for any future cross-Manager comparison or persistence.

Consequences:

- No node lifecycle to model. No "deactivate," no state machine.
- No mutable per-node state. If you want to cache something keyed on a node, it goes in the Manager's caches.
- The `NodeRecord` storage `Vec` and the dedup tables are **module-private**. Only `NodeId`s leak out, and the only producers are the factory methods (`intern_internal`, `intern_return_map`). This is what protects the hash-consing invariant. The compiler enforces it via module visibility, which is stronger than DDD's usual team-discipline guarantee.

Domain services (operations that don't belong to a single value object): `apply_and_reduce`, `compose`, `evaluate`, `path_count`. In Rust these are methods on `&[mut] Manager`. Read-side takes `&Manager`, write-side takes `&mut Manager`; the type system makes the distinction explicit.

No repositories, no domain events. No persistence in v1, so no repository pattern. No published events; the closest analog is GC's NodeId remap table, which is a concrete return value.

**Strategic distillation.** The *core* subdomain is PairProduct + Reduce + the hash-cons primitive + arena-deferred canonicalization. This is where the 10x lives and where we invest the most design effort. *Supporting*: ApplyAndReduce/Restrict/Compose (built on the core), evaluation, path-count maintenance, the Manager skeleton; standard implementations are fine. *Generic*: hash maps, arenas, smallvec, criterion, proptest; we use libraries (`hashbrown`, `bumpalo`, `smallvec`) and do not write our own.

The project lives or dies on PairProduct. Everything else follows from that.

## Ubiquitous language

Worth nailing now because the C++ engine and the CFLOBDD papers and ordinary programming intuition all use overlapping but distinct vocabulary.

| Term | Meaning in seaflowbdd |
|---|---|
| **Node** | A vertex in the canonical DAG. Either Internal, Fork, or DontCare. Identified by `NodeId`. |
| **NodeId** | A `u32` index into the canonical store. The only handle type callers see. `Copy`, no lifetime. |
| **Internal node** | A node at level ≥ 1 with one A-connection and one or more B-connections. |
| **Fork node** | The unique level-0 node representing $f(x) = x$. Singleton. |
| **DontCare node** | The unique level-0 node representing a constant function. Singleton. |
| **Connection** | A pair (target NodeId, ReturnMap) describing how a parent edge maps its child's exits to its own exits. |
| **ReturnMap** | A function from child-exit-index to parent-exit-index, stored as a small contiguous array. |
| **Level** | Depth in the recursion: a level-$k$ CFLOBDD represents a function over $2^k$ variables. |
| **Exit** | An output classification of a node. A node with $E$ exits partitions its inputs into $E$ equivalence classes. |
| **Canonical form** | The unique structural representation; structurally equal CFLOBDDs at the same level share the same `NodeId`. |
| **Apply** | Pointwise binary operation lifted to CFLOBDDs (AND, OR, XOR, etc.). |
| **PairProduct** | The recursive helper inside Apply that builds the cross-product structure. |
| **Reduce** | The canonicalization pass: dedup B-connections that go to equivalent subgraphs and collapse the parent's exit space accordingly. |
| **Manager** | The aggregate root owning the canonical store, dedup tables, and memo caches. All operations are methods on `&[mut] Manager`. |
| **Arena** | A scratch allocator for a single top-level operation; lifetime ends when the operation returns. Intermediates live here, not in the canonical store. |

Two terms we deliberately avoid:

- **"Handle."** The C++ engine uses it for refcounted pointer wrappers; we don't have those. `NodeId` is the only thing.
- **"Node" as a heap object.** In the C++ codebase, "node" sometimes means the C++ object instance with refcount, vtable, and identity-as-address. In seaflowbdd, "node" always means the abstract graph vertex; its storage is `NodeRecord`, and its identity is `NodeId`. This separation is the architectural premise; the language enforces it.

## Strategic design decisions

### 1. Stable-index handles, structure-of-arrays canonical store

The canonical store is a `Vec<NodeRecord>`; handles are `NodeId(u32)`, `Copy`, no refcount. A separate `HashMap<NodeKey, NodeId>` (Swiss table via `hashbrown`/std) does hash-consing: `NodeKey` is the structural content of a node, hashed and compared by value.

`NodeRecord` is a sum type, packed:

```rust
enum NodeRecord {
    Internal { level: u8, a_conn: ConnId, b_conns: BConnRange },
    Fork,
    DontCare,
}
```

`Fork` and `DontCare` are unit variants (zero payload). `Internal` is the only one carrying data. The discriminant is 1 byte; with reasonable layout the whole record fits in 8-16 bytes depending on how we encode `BConnRange`.

`BConnRange` is `(u32 start, u8 len)` indexing into a separate `Vec<BConn>` arena; with `len ≤ 3` covering the vast majority of internal nodes, we can SBO-inline up to 3 `BConn`s and use the range only for the long tail. (Initial implementation: just use the indirection; SBO when profiles say so.)

Connections (`a_conn` and each `b_conn`) themselves are indices into a small connection table that pairs a `NodeId` with a `ReturnMapId`. Return maps are also hash-consed in their own `Vec<ReturnMap>` + dedup table. Bodies are `Vec<u32>` (indices into a leaf-value table) for unweighted; this keeps the canonical store homogeneous and small.

**No `Rc`, no `Arc`, no `Box<dyn>` on the hot path.** The borrow checker manages the canonical store as a single owner; everyone else passes `NodeId`s.

### 2. Per-operation arena + deferred canonicalization

Top-level operations (`apply_and_reduce`, `compose`, `mat_mult` later) build intermediate structures into a per-call bump arena (`bumpalo`). Intermediates are not hash-consed during construction. At the end of the operation, a single bottom-up walk canonicalizes survivors into the global store, returning the final `NodeId`.

This kills the dominant allocation cost of the C++ engine, which canonicalizes every intermediate eagerly.

Memoization keys for PairProduct/Reduce live in a `HashMap` keyed on `(NodeId, NodeId)` packed into `u64`. This is correct because the operands are already canonical (they came from the global store) before the op begins.

### 3. Dispatch by tag, not vtable

`match` on the `NodeRecord` discriminant. The compiler turns this into a jump table or a small branch tree. No virtual dispatch anywhere on the hot path. Fork and DontCare are unit variants, so their cases are essentially branchless.

### 4. Path counts computed in `i128`, not `long double` + log/exp

The C++ engine's `InstallPathCounts` uses `pow(2, ...)` and `log2l(...)` to avoid bignum allocation. We use `i128` directly: 2-3 instructions per merge instead of two libm calls. For values that overflow `i128` (level ≥ 7 with maximally-deep CFLOBDDs), fall back to `num-bigint` lazily. The fast path is assumed.

Path counts are computed in the same recursion as Reduce/canonicalization, not as a separate pass.

### 5. ReturnMap as `SmallVec<[u32; 4]>`

Most return maps have ≤4 entries. SBO via `smallvec` puts them on the stack; dedup table key is a content hash. Compose, LookupInv, equality all become straight loops over a small contiguous slice; the autovectorizer handles the easy cases, explicit `std::simd` covers the rest if profiling demands it.

### 6. No global mutable state in the public API

The C++ engine has global static caches (`pairProductCache`, `tripleProductCache`, `reduceCache`, `flatLookup[level]`). Our equivalent state lives on a `Manager` struct that owns the canonical store, the dedup tables, and the memoization caches. Operations take `&mut Manager`. No singletons, no `static mut`, no "flush caches" bug surface. Multiple `Manager`s can coexist (useful for tests and oracle comparison) and they don't share canonical state.

### 7. Garbage collection as a phase, not a per-op concern

Without refcount, the canonical store grows monotonically until `Manager::gc(roots: &[NodeId])` is called. GC walks reachable nodes from the roots, builds a remap table, compacts the `Vec<NodeRecord>` in place, rewrites all internal references. Memo caches are cleared (their keys reference old NodeIds). Callers update any external roots using the returned remap.

Phase 1 ships without GC: just grow until the test process exits. GC is added when memory pressure shows up in benchmarks.

### 8. Const-generic shape specialization is a Phase 2 lever, not Phase 1 default

We will *not* pre-specialize on `numBConnections` or return-map size in Phase 1. The generic version is the baseline. If Phase 2 perf demands it, we add specialized fast paths for the common shapes (1×1, 1×2, 2×2 PairProduct; ReturnMap len ≤ 4) using `const N: usize` generics and a small dispatch trampoline. Profile-driven, not speculative.

### 9. SIMD is opt-in, profile-driven

`std::simd` (or `wide` on stable) for ReturnMap compose/equality and (eventually) MatMultMap merge. Not in Phase 1. The first version uses plain Rust loops and trusts LLVM's autovectorizer; we add explicit SIMD only where the profile says it matters.

### 10. Single file until it hurts

`src/lib.rs` is the whole engine in Phase 1. Pedagogical legibility over modular architecture. We split into modules when navigation becomes a problem, not before. (User preference: single-file designs optimized for pedagogical value.)

## Phase 1 milestones (parity within 10x)

The project's risk concentrates in PairProduct (milestone 5). Everything before it exists to make PairProduct verifiable on day one; everything after it exists because PairProduct made it possible.

In rough order:

1. **Manager skeleton**: `Manager` struct, `NodeId`, `NodeRecord`, dedup table, hash-cons primitive `intern_internal(level, a_conn, b_conns) -> NodeId`. Module-private storage; only `NodeId`s in the public surface. Just structure, no algorithms yet.
2. **Constants and projection**: `mk_true`, `mk_false`, `mk_proj(var, num_vars)`. Build the smallest non-trivial CFLOBDDs. Round-trip through the canonical store.
3. **Oracle ACL + traversal-based equivalence check**: `tests/oracle.rs` exposing the minimal C++-engine surface in seaflowbdd-shaped types; a `structurally_equal(seaflow_id, oracle_handle)` predicate that walks both graphs and compares value-by-value (no pointer or NodeId identity assumed across engines). This is the gate that makes every later milestone verifiable. Lands before any non-trivial algorithm.
4. **Reduce**: bottom-up reduction with the fused path-count installation. Cross-check against the oracle on small constructed nodes via the milestone-3 harness.
5. **PairProduct + ApplyAndReduce**: the workhorse, and the project's risk concentration. Memoized recursion over canonical operands, intermediates built into a `bumpalo` arena, single bottom-up canonicalization pass at the end. Cross-check on AND/OR/XOR over projection inputs at increasing var counts.
6. **Evaluation**: `evaluate(node, &assignment) -> bool`. Trivial recursion; useful for property-based correctness tests.
7. **Path count**: returns the cached `i128`. Compare against C++ engine's `NumSatisfyingAssignments`.
8. **Restrict, Compose**: built on PairProduct + Reduce.
9. **Property-based test corpus**: `proptest` generating random CFLOBDD-buildable boolean expressions, comparing seaflowbdd vs oracle results through the milestone-3 harness. (The harness exists from milestone 3; this milestone scales it up to a real corpus.)
10. **Microbenchmarks**: `criterion` measuring AND-tree, XOR-tree, projection products at varying sizes. Compare wall-clock against the C++ engine via the bindgen wrapper.

**Exit criterion for Phase 1**: all unweighted ops produce structurally identical CFLOBDDs to the C++ engine on the property-based corpus, and microbench times are within 10× of the C++ engine on every measured op. We expect parity or better on most ops; the 10× envelope is for ops we haven't yet specialized.

## Phase 2 milestones (exceed by 10x)

Driven by profile data from Phase 1. Likely sequence:

1. **Profile**: identify which ops are within 10× but not within 1×, and what the dominant cost is in each.
2. **Memo cache tuning**: hashbrown defaults are good but we can pre-size, FxHash, and dense-pack keys.
3. **BConn SBO**: inline ≤3 BConns into the `NodeRecord` itself. Eliminates an indirection on every internal-node visit.
4. **Shape specialization**: const-generic fast paths for 1×1, 1×2, 2×2 PairProduct; ReturnMap len ≤ 4.
5. **SIMD**: explicit `std::simd` for ReturnMap operations identified as hot.
6. **Single-use elision**: detect intermediates with refcount-equivalent = 1 (via arena ownership) and skip canonicalization for them.
7. **Layout tuning**: cache-line align the `NodeRecord` `Vec`, ensure dedup table buckets are sized for L1.

**Exit criterion for Phase 2**: every measured unweighted op runs at least 10× faster than the C++ engine on the same workload, single-threaded, on Apple Silicon. If we got there earlier, declare victory and document.

## Out of scope (for now)

- Weighted CFLOBDDs (complex, fourier, big-float). Phase 3 candidate.
- Matrix/vector quantum-style ops that depend on weighted nodes.
- Parallelism. Single-threaded throughout v1 and v2.
- GPU offload. Not in this engine's design space.
- Stable serialization format. Useful eventually; not now.
- Python bindings. The cflobdd C++ wrapper exists; if we want a Python face for seaflowbdd we'll do PyO3 later.

## Open questions to resolve as we go

- Exact `NodeRecord` layout: do we get to 8 bytes, or do we settle for 16? Depends on whether we can fit `BConnRange` + `level` + tag in 56 bits.
- Whether `ReturnMap` should be a separate hash-consed type or inlined per-Connection. The C++ engine hash-conses it; whether that's worth it serially in Rust depends on how much sharing actually exists. Measure.
- Whether the dedup table benefits from a custom Robin-Hood implementation over hashbrown. Probably not, but worth measuring once Phase 1 lands.
- Whether the oracle ACL should split into a `tests/oracle.rs` module (simple) or a separate `seaflowbdd-oracle` crate (cleaner boundary, more ceremony). Default to the module; promote to a crate only if the benchmarking context starts duplicating the same wrapping.
