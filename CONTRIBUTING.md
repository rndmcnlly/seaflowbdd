# Contributing to seaflowbdd

Thanks for your interest. This document covers the development workflow, the verification strategy, and a few project-specific conventions.

## Build and test

```sh
cargo build
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

The repository's own CI runs all four. Pull requests that fail any of them will need fixing before review.

## Verification strategy

The crate's correctness is anchored by three test axes:

1. **Semantic correctness via truth tables.** For small variable counts (level 3 = 8 variables, level 4 = 16) we enumerate every possible assignment and check that `evaluate` matches a Rust shadow evaluator on each. See `tests/eval_truth_tables.rs` and `tests/restrict_compose.rs`.
2. **Algebraic identities.** Idempotence, commutativity, associativity, distributivity, etc., asserted via canonical `Bdd == Bdd` comparison. See `tests/proptest_corpus.rs`.
3. **Property-based generation.** A `proptest` corpus generates random boolean expressions and verifies semantic correctness across all assignments per case. See `tests/proptest_corpus.rs`.
4. **Path-count cross-checks.** Path counts are verified against brute-force enumeration of satisfying assignments at small scales. See `tests/path_count.rs`.

When adding new operations, please add tests at every applicable axis. Truth tables and proptest are inexpensive and catch a lot.

## Optional: parity testing against a reference implementation

This repository does not include the C++ reference engine ([trishullab/cflobdd](https://github.com/trishullab/cflobdd)) or any tests that compare against it. For maintainers doing structural-form verification when changing core algorithms (Reduce, PairProduct, Restrict), the project author maintains a private development tree that wires in the C++ engine via a bindgen wrapper and runs structural-parity tests. If you're working on those algorithms and want similar verification, you can:

1. Clone the C++ engine, build it as a static library with `CFLOBDD_MAX_LEVEL=5`.
2. Write a thin bindgen wrapper exposing `mk_true`, `mk_false`, `mk_proj`, `and`/`or`/`xor`/`not`, `restrict`, `count_nodes_edges`, `eq`, `clone`, `drop`.
3. Build seaflowbdd at the same level (`Manager::new(5)`) and compare `reachable_node_count` and `reachable_edge_count` outputs across matched constructions.

This is genuinely useful when bringing up a new core algorithm but is optional for everything else; the four axes above are sufficient for confidence in extensions.

## Code style and conventions

- **Module layout.** One module per major op or data structure. Keep `lib.rs` as the orchestrator; do not add public items there beyond re-exports.
- **No global mutable state.** All state lives on `Manager`. Avoid `static mut`, `lazy_static`, and thread-locals.
- **`Bdd` is `Copy`.** Don't add lifetime parameters or borrow semantics to it.
- **Borrow checker over `Rc`/`Arc`.** The single-owner manager + `NodeId` indices is the design. Resist requests to introduce reference counting unless there's a profile-driven reason.
- **Document algorithm provenance.** When porting or adapting an algorithm from prior work, cite the source in a source comment. Provenance is part of the documentation.
- **Avoid emdashes** in comments and prose; prefer colons, commas, parentheses, and endashes for ranges.

## Performance work

If you're submitting a performance change, please include criterion microbenchmarks demonstrating the effect. Don't optimize without measuring.

The author maintains separate (currently private) benchmarks comparing seaflowbdd against the C++ reference engine. If you're proposing changes that affect the performance story, mentioning the workloads you measured against (and on what hardware) is helpful.

## Where to start

- **Open the issue tracker** for ideas labeled `good first issue`.
- **Read `PLAN.md`** for the design rationale, especially the bounded-context analysis and the list of open questions.
- **Read the source.** Phase 1 fits in seven small modules; you can read it all in an afternoon.

## License

By contributing you agree your contribution will be licensed under the project's MIT license.
