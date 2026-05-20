# seaflowbdd

A native Rust engine for **Context-Free-Language Ordered Binary Decision Diagrams** (CFLOBDDs): a compressed representation of boolean functions over $2^k$ variables, due to Sistla, Chaudhuri, and Reps.

Like classical BDDs, CFLOBDDs let you build, combine, evaluate, and count satisfying assignments of boolean expressions. Unlike BDDs, their hierarchical structure can give exponential compression on many functions of practical interest.

The name is a phonetic spelling of **CFLOBDD** as you'd say it after you've stopped trying: *"sea-flow"*. *bdd* is just *bdd*.

## Status

**v0.1: unweighted boolean operations only.** The engine implements the full CFLOBDD pipeline (hash-cons canonical store, PairProduct, Reduce, ApplyAndReduce) for boolean-valued functions. Weighted variants (complex, big-float, Fourier) and parallelism are out of scope for this release.

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

Performance comparison and Phase 2 optimization work is in flight; numbers will appear in a future release once the engine is competitive with established BDD libraries on representative workloads.

## Roadmap

Possible directions, in no particular order:

- **Tighten `not`** to canonicalize the value-map ordering.
- **Garbage collection.** Without refcount, the canonical store grows monotonically. A mark-and-compact GC keyed on user-supplied roots is a natural fit and is sketched in `PLAN.md`.
- **Specialized fast paths** for small return-map sizes, common shape pairs (1×1, 1×2, 2×2), and SBO of B-connections inline in NodeRecord.
- **Weighted variants** (complex, big-float, Fourier) for quantum-circuit-style applications. The C++ engine ships these; we don't yet.
- **Bignum path counts** when `i128` overflows.

If any of these are interesting to you, see `CONTRIBUTING.md`.

## License

MIT. See `LICENSE`.

## References

- Sistla, M., Chaudhuri, S., and Reps, T. *CFLOBDDs: Context-Free-Language Ordered Binary Decision Diagrams.* ACM TOPLAS 46(2), 2024. [arXiv:2211.06818](https://arxiv.org/abs/2211.06818).
- The C++ reference engine: <https://github.com/trishullab/cflobdd>.
