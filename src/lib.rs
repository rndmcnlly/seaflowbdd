//! Native Rust engine for **Context-Free-Language Ordered Binary
//! Decision Diagrams** (CFLOBDDs): a compressed representation of
//! boolean functions over $2^k$ variables, due to Sistla, Chaudhuri,
//! and Reps.
//!
//! Like classical BDDs, CFLOBDDs let you build, combine, evaluate, and
//! count satisfying assignments of boolean expressions; unlike BDDs,
//! their hierarchical structure can give exponential compression on
//! many functions of practical interest.
//!
//! # Quick start
//!
//! ```
//! use seaflowbdd::Manager;
//!
//! // A manager fixed at level 2 represents functions over 2^2 = 4 variables.
//! let mut m = Manager::new(2);
//!
//! let x0 = m.mk_proj(0);            // x_0
//! let x1 = m.mk_proj(1);            // x_1
//! let conj = m.and(x0, x1);         // x_0 AND x_1
//!
//! // Evaluate on the assignment [x_0 = true, x_1 = true, x_2 = false, x_3 = false].
//! assert!( m.evaluate(conj, &[true,  true,  false, false]));
//! assert!(!m.evaluate(conj, &[true,  false, false, false]));
//!
//! // Count satisfying assignments. x_0 AND x_1 is satisfied by exactly
//! // one quarter of the 16 total assignments.
//! assert_eq!(m.path_count(conj), 4);
//! ```
//!
//! # API tour
//!
//! - [`Manager::new`]: create an engine fixed at a top level. The total
//!   number of addressable variables is `2^level`.
//! - [`Manager::mk_true`], [`Manager::mk_false`], [`Manager::mk_proj`]:
//!   the three primitive Bdds (constants and individual variables).
//! - [`Manager::and`], [`Manager::or`], [`Manager::xor`],
//!   [`Manager::not`], [`Manager::nand`], [`Manager::nor`],
//!   [`Manager::iff`], [`Manager::implies`]: pointwise boolean
//!   operations. All return a new canonical [`Bdd`].
//! - [`Manager::apply`]: arbitrary 2-input boolean ops via a 2x2 truth
//!   table ([`BoolOp`]).
//! - [`Manager::restrict`]: substitute a constant for a variable.
//! - [`Manager::compose`]: substitute another Bdd for a variable.
//! - [`Manager::exists`], [`Manager::forall`]: variable quantification.
//! - [`Manager::evaluate`]: evaluate a Bdd on a given assignment.
//! - [`Manager::path_count`]: count satisfying assignments.
//! - [`Manager::reachable_node_count`], [`Manager::reachable_edge_count`]:
//!   structural size diagnostics.
//!
//! # Design overview
//!
//! - Bdds are `Copy` value handles (two `u32`s); the actual graph lives
//!   on the [`Manager`].
//! - Internal nodes are hash-consed: structurally equal subgraphs share
//!   a single `NodeId`, so `Bdd` equality is canonical and is just an
//!   id comparison.
//! - There is no global state. Multiple managers can coexist with
//!   disjoint id spaces.
//! - The engine is single-threaded and serial. Concurrency is out of
//!   scope for now.
//!
//! See `CONTRIBUTING.md` for development notes, and the source of
//! [`Manager`] for the full method list.

mod manager;
mod node;
mod pair_product;
mod path_count;
mod reduce;
mod restrict;
mod return_map;

pub use manager::{Bdd, Manager};
pub use pair_product::{BoolOp, AND_OP, IFF_OP, IMPLIES_OP, NAND_OP, NOR_OP, OR_OP, XOR_OP};
