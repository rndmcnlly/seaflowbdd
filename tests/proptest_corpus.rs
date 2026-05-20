//! Property-based correctness corpus.
//!
//! For each randomly-generated boolean expression we check two things:
//!
//! 1. **Semantic correctness.** seaflowbdd's `evaluate` agrees with a
//!    Rust shadow eval of the expression on every assignment. Run at
//!    `SEM_LEVEL=3` (8 vars), so we enumerate all 256 assignments per
//!    case.
//! 2. **Algebraic identities.** Idempotence, identity, annihilator,
//!    commutativity, associativity, distributivity. These are checked
//!    via canonical `Bdd == Bdd` (id comparison after hash-consing).
//!    Double negation is checked semantically (via `evaluate`) only,
//!    because seaflowbdd's `not` may produce a non-canonical value-map
//!    order: see the comment on `pair_product.rs::not`.

use proptest::prelude::*;
use seaflowbdd::{Bdd, Manager};

/// Level for semantic / algebraic-identity tests. 8 vars ⇒ full truth
/// table is 256 entries, cheap.
const SEM_LEVEL: u8 = 3;
const SEM_VARS: u32 = 1 << SEM_LEVEL; // 8

/// Bound the recursion depth of generated expressions. Five levels of
/// nesting gives a healthy mix of leaves and internal operators while
/// keeping shrinking tractable.
const MAX_DEPTH: u32 = 5;

/// Number of cases per `proptest!` block.
const CASES_FAST: u32 = 64;

#[derive(Clone, Debug)]
enum Expr {
    True,
    False,
    Var(u32),
    Not(Box<Expr>),
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
    Xor(Box<Expr>, Box<Expr>),
}

impl Expr {
    fn eval(&self, asn: &[bool]) -> bool {
        match self {
            Expr::True => true,
            Expr::False => false,
            Expr::Var(i) => asn[*i as usize],
            Expr::Not(e) => !e.eval(asn),
            Expr::And(a, b) => a.eval(asn) && b.eval(asn),
            Expr::Or(a, b) => a.eval(asn) || b.eval(asn),
            Expr::Xor(a, b) => a.eval(asn) ^ b.eval(asn),
        }
    }

    fn build_sf(&self, m: &mut Manager) -> Bdd {
        match self {
            Expr::True => m.mk_true(),
            Expr::False => m.mk_false(),
            Expr::Var(i) => m.mk_proj(*i),
            Expr::Not(e) => {
                let b = e.build_sf(m);
                m.not(b)
            }
            Expr::And(a, b) => {
                let x = a.build_sf(m);
                let y = b.build_sf(m);
                m.and(x, y)
            }
            Expr::Or(a, b) => {
                let x = a.build_sf(m);
                let y = b.build_sf(m);
                m.or(x, y)
            }
            Expr::Xor(a, b) => {
                let x = a.build_sf(m);
                let y = b.build_sf(m);
                m.xor(x, y)
            }
        }
    }
}

/// Strategy for boolean expressions over `total_vars` variables, with
/// recursion depth bounded by `max_depth`. Leaf weight is tuned to keep
/// average size modest.
fn arb_expr(total_vars: u32, max_depth: u32) -> impl Strategy<Value = Expr> {
    let leaf = prop_oneof![
        1 => Just(Expr::True),
        1 => Just(Expr::False),
        // Var is more likely than constants: most non-trivial structure
        // lives behind variable references.
        6 => (0..total_vars).prop_map(Expr::Var),
    ];
    leaf.prop_recursive(
        max_depth, // depth
        64,        // target total size
        4,         // expected branch factor at each internal node
        |inner| {
            prop_oneof![
                inner.clone().prop_map(|e| Expr::Not(Box::new(e))),
                (inner.clone(), inner.clone())
                    .prop_map(|(a, b)| Expr::And(Box::new(a), Box::new(b))),
                (inner.clone(), inner.clone())
                    .prop_map(|(a, b)| Expr::Or(Box::new(a), Box::new(b))),
                (inner.clone(), inner).prop_map(|(a, b)| Expr::Xor(Box::new(a), Box::new(b))),
            ]
        },
    )
}

/// Iterate over every assignment of `n` boolean variables.
fn all_assignments(n: u32) -> impl Iterator<Item = Vec<bool>> {
    let n = n as usize;
    (0..(1u64 << n)).map(move |v| (0..n).map(|i| (v >> i) & 1 == 1).collect())
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: CASES_FAST,
        // We intentionally don't persist failures: this corpus is the
        // discovery instrument, not a regression suite.
        failure_persistence: None,
        ..Default::default()
    })]

    /// Semantic correctness: shadow eval == seaflowbdd evaluate on
    /// every assignment.
    #[test]
    fn semantic_correctness(expr in arb_expr(SEM_VARS, MAX_DEPTH)) {
        let mut m = Manager::new(SEM_LEVEL);
        let bdd = expr.build_sf(&mut m);
        for asn in all_assignments(m.total_vars()) {
            let actual = m.evaluate(bdd, &asn);
            let expected = expr.eval(&asn);
            prop_assert_eq!(
                actual, expected,
                "expr {:?} on {:?}: expected {}, got {}",
                expr, asn, expected, actual
            );
        }
    }

    // -- Algebraic identities (canonical, where they hold) ---------------

    /// Idempotence: f AND f == f, f OR f == f.
    #[test]
    fn idempotence(expr in arb_expr(SEM_VARS, MAX_DEPTH)) {
        let mut m = Manager::new(SEM_LEVEL);
        let f = expr.build_sf(&mut m);
        let f_and_f = m.and(f, f);
        let f_or_f = m.or(f, f);
        prop_assert_eq!(f_and_f, f, "f AND f must equal f canonically");
        prop_assert_eq!(f_or_f, f, "f OR f must equal f canonically");
    }

    /// XOR-self annihilator: f XOR f == false.
    #[test]
    fn xor_self_is_false(expr in arb_expr(SEM_VARS, MAX_DEPTH)) {
        let mut m = Manager::new(SEM_LEVEL);
        let f = expr.build_sf(&mut m);
        let ff = m.xor(f, f);
        let fls = m.mk_false();
        prop_assert_eq!(ff, fls, "f XOR f must equal false canonically");
    }

    /// Identity / annihilator with constants.
    #[test]
    fn const_identities(expr in arb_expr(SEM_VARS, MAX_DEPTH)) {
        let mut m = Manager::new(SEM_LEVEL);
        let f = expr.build_sf(&mut m);
        let t = m.mk_true();
        let fls = m.mk_false();
        let f_and_t = m.and(f, t);
        let f_or_f0 = m.or(f, fls);
        let f_and_f0 = m.and(f, fls);
        let f_or_t = m.or(f, t);
        prop_assert_eq!(f_and_t, f, "f AND true == f");
        prop_assert_eq!(f_or_f0, f, "f OR false == f");
        prop_assert_eq!(f_and_f0, fls, "f AND false == false");
        prop_assert_eq!(f_or_t, t, "f OR true == true");
    }

    /// Commutativity: f OP g == g OP f for OP in {AND, OR, XOR}.
    #[test]
    fn commutativity(
        a in arb_expr(SEM_VARS, MAX_DEPTH),
        b in arb_expr(SEM_VARS, MAX_DEPTH),
    ) {
        let mut m = Manager::new(SEM_LEVEL);
        let f = a.build_sf(&mut m);
        let g = b.build_sf(&mut m);
        prop_assert_eq!(m.and(f, g), m.and(g, f), "AND commutative");
        prop_assert_eq!(m.or(f, g), m.or(g, f), "OR commutative");
        prop_assert_eq!(m.xor(f, g), m.xor(g, f), "XOR commutative");
    }

    /// Associativity: (f AND g) AND h == f AND (g AND h).
    /// Cap depth at 3 here since we generate three subexpressions.
    #[test]
    fn associativity_and(
        a in arb_expr(SEM_VARS, 3),
        b in arb_expr(SEM_VARS, 3),
        c in arb_expr(SEM_VARS, 3),
    ) {
        let mut m = Manager::new(SEM_LEVEL);
        let f = a.build_sf(&mut m);
        let g = b.build_sf(&mut m);
        let h = c.build_sf(&mut m);
        let fg = m.and(f, g);
        let fg_h = m.and(fg, h);
        let gh = m.and(g, h);
        let f_gh = m.and(f, gh);
        prop_assert_eq!(fg_h, f_gh, "AND associative");
    }

    /// Distributivity: f AND (g OR h) == (f AND g) OR (f AND h).
    #[test]
    fn distributivity_and_over_or(
        a in arb_expr(SEM_VARS, 3),
        b in arb_expr(SEM_VARS, 3),
        c in arb_expr(SEM_VARS, 3),
    ) {
        let mut m = Manager::new(SEM_LEVEL);
        let f = a.build_sf(&mut m);
        let g = b.build_sf(&mut m);
        let h = c.build_sf(&mut m);
        let g_or_h = m.or(g, h);
        let lhs = m.and(f, g_or_h);
        let f_and_g = m.and(f, g);
        let f_and_h = m.and(f, h);
        let rhs = m.or(f_and_g, f_and_h);
        prop_assert_eq!(lhs, rhs, "AND distributes over OR");
    }

    /// Double negation: not(not(f)) is *semantically* equivalent to f,
    /// but not necessarily *canonically* equal. seaflowbdd's `not`
    /// flips the value map without re-canonicalizing its order, so
    /// `not(not(f))` shares the same root as `f` but may have a
    /// distinct (still-equivalent) `values` id. We therefore check
    /// agreement via `evaluate` only.
    #[test]
    fn double_negation_semantic(expr in arb_expr(SEM_VARS, MAX_DEPTH)) {
        let mut m = Manager::new(SEM_LEVEL);
        let f = expr.build_sf(&mut m);
        let nf = m.not(f);
        let nnf = m.not(nf);
        for asn in all_assignments(m.total_vars()) {
            let v_f = m.evaluate(f, &asn);
            let v_nnf = m.evaluate(nnf, &asn);
            prop_assert_eq!(
                v_f, v_nnf,
                "not(not(f)) must agree with f on {:?} for {:?}",
                asn, expr
            );
        }
    }
}
