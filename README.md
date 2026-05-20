# seaflowbdd

A native Rust engine for **Context-Free-Language Ordered Binary Decision Diagrams** (CFLOBDDs): a compressed representation of boolean functions over $2^k$ variables, due to Sistla, Chaudhuri, and Reps.

Like classical BDDs, CFLOBDDs let you build, combine, evaluate, and count satisfying assignments of boolean expressions. Unlike BDDs, their hierarchical structure can give exponential compression on many functions of practical interest.

The name is a phonetic spelling of **CFLOBDD** as you'd say it after you've stopped trying: *"sea-flow"*. *bdd* is just *bdd*.

## Status

**v0.2: unweighted boolean operations, optimized.** The engine implements the full CFLOBDD pipeline (hash-cons canonical store, PairProduct, Reduce, ApplyAndReduce) for boolean-valued functions. Phase 2 optimizations have brought wall-clock performance to parity with or better than the C++ reference engine on every measured workload, with at least one workload running ≥10× faster (see [Performance](#performance)). Weighted variants (complex, big-float, Fourier) and parallelism are out of scope for this release.

The codebase started as a Rust port of the algorithms from [trishullab/cflobdd](https://github.com/trishullab/cflobdd); algorithm provenance is credited in source comments where applicable. seaflowbdd is its own implementation with its own data layout and lifecycle.

## Install

```toml
[dependencies]
seaflowbdd = "0.1"
```

The crate is pure Rust. No C++ toolchain or system libraries required. Two small dependencies: `hashbrown` and `smallvec`.

## Quick start

```rust
use seaflowbdd::Manager;

// A manager fixed at level 2 represents functions over 2^2 = 4 variables.
let mut m = Manager::new(2);

let x0 = m.mk_proj(0);            // x_0
let x1 = m.mk_proj(1);            // x_1
let f = m.and(x0, x1);            // x_0 AND x_1

// Evaluate on the assignment [x_0 = true, x_1 = true, x_2 = false, x_3 = false]:
assert!( m.evaluate(f, &[true,  true,  false, false]));
assert!(!m.evaluate(f, &[false, true,  true,  true ]));

// Count satisfying assignments. Out of 16 total assignments, exactly
// those with x_0 = x_1 = 1 satisfy f, regardless of x_2 / x_3.
assert_eq!(m.path_count(f), 4);

// Bdd equality is canonical: structurally equal Bdds compare by id.
let f_again = {
    let a = m.mk_proj(0);
    let b = m.mk_proj(1);
    m.and(a, b)
};
assert_eq!(f, f_again);
```

See `examples/hello.rs` and `examples/majority.rs` for complete runnable programs.

## API summary

All operations are methods on `&mut Manager` (or `&Manager` for read-only ops). `Bdd` is a `Copy` value handle; the actual graph lives on the manager.

| Method | Notes |
|---|---|
| `Manager::new(level: u8)` | Create an engine. Total variables = $2^{\text{level}}$. |
| `mk_true()`, `mk_false()`, `mk_proj(i)` | Primitives. |
| `and`, `or`, `xor`, `not`, `nand`, `nor`, `iff`, `implies` | Pointwise boolean ops. |
| `apply(f, g, op: BoolOp)` | Arbitrary 2-input op via a 2x2 truth table. |
| `restrict(f, var, value)` | Substitute a constant for a variable. |
| `compose(f, var, g)` | Substitute a Bdd for a variable. |
| `exists(f, var)`, `forall(f, var)` | Variable quantification. |
| `evaluate(f, &[bool])` | Evaluate on an assignment. |
| `path_count(f) -> i128` | Count satisfying assignments. Lazily memoized. |
| `eq(f, g)` | Canonical equality (id comparison after hash-consing). |
| `reachable_node_count(f)`, `reachable_edge_count(f)` | Structural diagnostics. |

## Design

- **Hash-consed canonical store.** Internal nodes are interned: structurally equal subgraphs share a single `NodeId`. Equality is canonical and reduces to id comparison.
- **No global state.** Multiple `Manager`s coexist with disjoint id spaces. No singletons, no `static mut`, no thread-local nonsense.
- **`Bdd` is `Copy`.** A handle is two `u32`s. Passing Bdds around is free; the graph lives on the manager.
- **Single-threaded.** No locks, no atomics on the hot path. Concurrency is out of scope for now.
- **i128 path counts.** Path counting uses 128-bit integers in linear space, valid up to $2^{128}$ paths (covers level ≤ 7). Other engines store path counts in log space using `long double`; we don't need to.
- **Memoization.** PairProduct, Reduce, and path-count results are memoized per-manager.

See `PLAN.md` in the repo for the full design document, including the bounded-context analysis, aggregate boundaries, and ubiquitous-language glossary.

## Caveats

- **`not(not(f))` is semantically equal to `f` but not currently *canonically* equal.** `not` flips the value-map bits without re-canonicalizing their order, so the resulting `Bdd`s share a root id but may have distinct value-map ids. Compare via `evaluate` or by canonicalizing through any boolean op when this matters. To be tightened in a future release.
- **Levels are bounded by overflow.** `path_count` panics on i128 overflow, which can happen at level ≥ 7 only if the function has near-$2^{128}$ satisfying assignments. `Manager::new` accepts levels up to 30 (i.e. up to ~1 billion variables) but path counting at level ≥ 7 may overflow.
- **No serialization yet.** Bdds are not directly persistable across processes. Build them in memory, use them, drop the manager.

## Performance

Microbenchmarks comparing seaflowbdd v0.2 against the trishullab/cflobdd C++ reference engine, single-threaded on Apple Silicon (M3 Max). Negative numbers favor seaflowbdd.

Two cuts. **`from_scratch`** builds a fresh seaflowbdd `Manager` per iteration (cold caches every time); the C++ engine has no cache-reset API, so its caches stay warm across iterations within a benchmark. This biases toward the oracle. **`warm`** pre-builds projections outside the iter loop so both engines run on warm memo caches; this isolates the boolean-apply cost from projection construction.

| Workload | seaflowbdd | C++ oracle | ratio |
|---|---:|---:|---:|
| `projection_construction_32` | 3.00 µs | 35.36 µs | **0.08× (11.8× faster)** |
| `and_tree/32 from_scratch` | 34.34 µs | 40.94 µs | 0.84× (1.2× faster) |
| `and_tree/8 from_scratch` | 15.56 µs | 10.22 µs | 1.52× |
| `and_tree/4 from_scratch` | 8.44 µs | 5.07 µs | 1.66× |
| `and_tree/32 warm` | 1.58 µs | 4.68 µs | 0.34× (2.9× faster) |
| `or_tree/32 warm` | 1.61 µs | 4.67 µs | 0.34× (2.9× faster) |
| `xor_tree/32 warm` | 1.63 µs | 4.76 µs | 0.34× (2.9× faster) |
| `repeated_apply` (16 vars × 100 iters) | 76 µs | 226 µs | 0.34× (3.0× faster) |

Every measured op is within 2× of the C++ engine's wall-clock, and most are several times faster.

The principal Phase 2 wins were:

- **Reduce memo cache.** The C++ engine has one; v0.1 didn't. Adding it turned warm-bench costs from "rebuild from scratch every call" into "lookup in a memo table". (~16× speedup on warm benches.)
- **Hash-table dedup without key cloning.** The node dedup table now uses hashbrown's `HashTable` with hash + eq closures that probe directly against existing records, instead of a `HashMap<NodeRecord, NodeId>` that cloned the key on every insert.
- **Precomputed common return maps.** The maps `[0]`, `[1]`, `[0, 1]` show up everywhere in construction; preinterning at `Manager::new` removes a HashMap lookup per use.
- **Stack-allocated SmallVec snapshots** instead of `to_vec()` heap allocations on hot recursion paths in PairProduct, Reduce, and Restrict.
- **FxHash for memo cache keys.** `(NodeId, NodeId)` and `(NodeId, ReturnMapId)` are u64-equivalent; FxHash beats a randomized hasher on these.
- **Custom `Hash` impl** for `NodeRecord` that hashes packed `u64` connection fields directly, instead of the derived per-field hashing.

## Roadmap

Possible directions, in no particular order:

- **Tighten `not`** to canonicalize the value-map ordering.
- **Garbage collection.** Without refcount, the canonical store grows monotonically. A mark-and-compact GC keyed on user-supplied roots is a natural fit and is sketched in `PLAN.md`.
- **Specialized fast paths** for small return-map sizes, common shape pairs (1×1, 1×2, 2×2), and SBO of B-connections inline in NodeRecord.
- **Weighted variants** (complex, big-float, Fourier) for quantum-circuit-style applications. The C++ engine ships these; we don't yet.
- **Bignum path counts** when `i128` overflows.
- **Further perf work.** v0.2 hit parity-or-better on every measured workload, with at least one workload running ≥10× faster. The remaining gap on small `from_scratch` workloads (1.5×–1.7× slower than C++ on cold-cache AND-trees of 4–8 variables) is dominated by HashTable churn during structure construction; a flat-array dedup for tiny BConn shapes would close this.

If any of these are interesting to you, see `CONTRIBUTING.md`.

## License

MIT. See `LICENSE`.

## References

- Sistla, M., Chaudhuri, S., and Reps, T. *CFLOBDDs: Context-Free-Language Ordered Binary Decision Diagrams.* ACM TOPLAS 46(2), 2024. [arXiv:2211.06818](https://arxiv.org/abs/2211.06818).
- The C++ reference engine: <https://github.com/trishullab/cflobdd>.
