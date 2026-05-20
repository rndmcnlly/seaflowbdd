//! Tests for Restrict, Compose, Exists, Forall.
//!
//! Two coverage axes:
//!
//! 1. Semantic truth-table correctness (LEVEL=3, 8 vars, 256 assignments)
//!    against a Rust shadow evaluator.
//! 2. Algebraic identities (compose distributes over restrict, etc.).

use seaflowbdd::Manager;

// -- Shadow expression type for ground truth -------------------------

#[derive(Clone, Debug)]
#[allow(dead_code)] // True / False are exhaustive-match arms, never directly constructed in this corpus.
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

    fn build(&self, m: &mut Manager) -> seaflowbdd::Bdd {
        match self {
            Expr::True => m.mk_true(),
            Expr::False => m.mk_false(),
            Expr::Var(i) => m.mk_proj(*i),
            Expr::Not(e) => {
                let b = e.build(m);
                m.not(b)
            }
            Expr::And(a, b) => {
                let x = a.build(m);
                let y = b.build(m);
                m.and(x, y)
            }
            Expr::Or(a, b) => {
                let x = a.build(m);
                let y = b.build(m);
                m.or(x, y)
            }
            Expr::Xor(a, b) => {
                let x = a.build(m);
                let y = b.build(m);
                m.xor(x, y)
            }
        }
    }
}

// -- Semantic correctness via truth tables (level 3, 8 vars) ---------

const LEVEL_SEM: u8 = 3;

fn assert_truth_table_eq(
    m: &mut Manager,
    bdd: seaflowbdd::Bdd,
    expected: impl Fn(&[bool]) -> bool,
) {
    let n = m.total_vars() as usize;
    for v in 0..(1u64 << n) {
        let asn: Vec<bool> = (0..n).map(|i| (v >> i) & 1 == 1).collect();
        let got = m.evaluate(bdd, &asn);
        let exp = expected(&asn);
        assert_eq!(
            got, exp,
            "truth-table mismatch at {:?}: expected {}, got {}",
            asn, exp, got
        );
    }
}

#[test]
fn restrict_self_yields_constant() {
    // restrict(x_i, x_i, v) == const(v) for every i, v.
    let mut m = Manager::new(LEVEL_SEM);
    let t = m.mk_true();
    let f = m.mk_false();
    for i in 0..m.total_vars() {
        let xi = m.mk_proj(i);
        let r_true = m.restrict(xi, i, true);
        assert_eq!(
            r_true, t,
            "restrict(x_{}, x_{}, true) must equal mk_true",
            i, i
        );
        let r_false = m.restrict(xi, i, false);
        assert_eq!(
            r_false, f,
            "restrict(x_{}, x_{}, false) must equal mk_false",
            i, i
        );
    }
}

#[test]
fn restrict_other_var_is_no_op() {
    // restrict(x_j, x_i, v) == x_j for j != i.
    let mut m = Manager::new(LEVEL_SEM);
    for i in 0..m.total_vars() {
        for j in 0..m.total_vars() {
            if i == j {
                continue;
            }
            let xj = m.mk_proj(j);
            for &v in &[true, false] {
                let r = m.restrict(xj, i, v);
                assert_eq!(
                    r, xj,
                    "restrict(x_{}, x_{}, {}) must equal x_{}",
                    j, i, v, j
                );
            }
        }
    }
}

#[test]
fn restrict_truth_table_random_expressions() {
    let mut m = Manager::new(LEVEL_SEM);
    let exprs = sample_expressions();
    for (label, e) in &exprs {
        let bdd = e.build(&mut m);
        for i in 0..m.total_vars() {
            for &v in &[true, false] {
                let r = m.restrict(bdd, i, v);
                assert_truth_table_eq(&mut m, r, |asn| {
                    let mut a = asn.to_vec();
                    a[i as usize] = v;
                    e.eval(&a)
                });
                let _ = label;
            }
        }
    }
}

fn sample_expressions() -> Vec<(&'static str, Expr)> {
    use Expr::*;
    let var = |i| Var(i);
    let and = |a, b| And(Box::new(a), Box::new(b));
    let or = |a, b| Or(Box::new(a), Box::new(b));
    let xor = |a, b| Xor(Box::new(a), Box::new(b));
    let not = |a| Not(Box::new(a));
    vec![
        ("x0 AND x1", and(var(0), var(1))),
        ("x0 OR x1", or(var(0), var(1))),
        ("x0 XOR x3", xor(var(0), var(3))),
        ("(x0 AND x1) OR x2", or(and(var(0), var(1)), var(2))),
        ("NOT (x0 AND x1)", not(and(var(0), var(1)))),
        (
            "(x0 XOR x4) AND (x2 OR x5)",
            and(xor(var(0), var(4)), or(var(2), var(5))),
        ),
        (
            "majority(x0,x3,x6)",
            or(
                or(and(var(0), var(3)), and(var(3), var(6))),
                and(var(0), var(6)),
            ),
        ),
        (
            "parity(x0..x4)",
            xor(xor(xor(xor(var(0), var(1)), var(2)), var(3)), var(4)),
        ),
        (
            "(x0 IFF x1) AND (x2 OR NOT x7)",
            and(
                xor(not(var(0)), not(xor(var(0), var(1)))), // identity rewrite, just to mix nots
                or(var(2), not(var(7))),
            ),
        ),
        (
            "ladder(x0..x6)",
            or(
                or(and(var(0), var(1)), and(var(2), var(3))),
                or(and(var(4), var(5)), var(6)),
            ),
        ),
    ]
}

// -- Compose ---------------------------------------------------------

#[test]
fn compose_constant_g_equals_restrict() {
    // compose(f, x_i, true)  == restrict(f, x_i, true)
    // compose(f, x_i, false) == restrict(f, x_i, false)
    let mut m = Manager::new(LEVEL_SEM);
    let exprs = sample_expressions();
    for (_label, e) in &exprs {
        let f = e.build(&mut m);
        let t = m.mk_true();
        let fa = m.mk_false();
        for i in 0..m.total_vars() {
            let c_t = m.compose(f, i, t);
            let r_t = m.restrict(f, i, true);
            assert_eq!(
                c_t, r_t,
                "compose(f, x_{}, true) must equal restrict(f, x_{}, true)",
                i, i
            );
            let c_f = m.compose(f, i, fa);
            let r_f = m.restrict(f, i, false);
            assert_eq!(
                c_f, r_f,
                "compose(f, x_{}, false) must equal restrict(f, x_{}, false)",
                i, i
            );
        }
    }
}

#[test]
fn compose_self_is_identity() {
    // compose(f, x_i, x_i) == f
    let mut m = Manager::new(LEVEL_SEM);
    let exprs = sample_expressions();
    for (label, e) in &exprs {
        let f = e.build(&mut m);
        for i in 0..m.total_vars() {
            let xi = m.mk_proj(i);
            let c = m.compose(f, i, xi);
            assert_eq!(c, f, "compose({}, x_{}, x_{}) must equal f", label, i, i);
        }
    }
}

#[test]
fn compose_truth_table() {
    // compose(f, x_i, g) on assignment a evaluates as: f on the
    // assignment a' that equals a everywhere except a'[i] = g(a).
    let mut m = Manager::new(LEVEL_SEM);
    let f_expr = Expr::Or(
        Box::new(Expr::And(Box::new(Expr::Var(0)), Box::new(Expr::Var(1)))),
        Box::new(Expr::Var(2)),
    );
    let g_exprs = vec![
        Expr::And(Box::new(Expr::Var(3)), Box::new(Expr::Var(4))),
        Expr::Xor(Box::new(Expr::Var(5)), Box::new(Expr::Var(6))),
        Expr::Not(Box::new(Expr::Var(7))),
        Expr::Or(Box::new(Expr::Var(0)), Box::new(Expr::Var(7))),
    ];
    for g_expr in &g_exprs {
        let f = f_expr.build(&mut m);
        let g = g_expr.build(&mut m);
        for i in 0..m.total_vars() {
            let c = m.compose(f, i, g);
            assert_truth_table_eq(&mut m, c, |asn| {
                let mut a2 = asn.to_vec();
                a2[i as usize] = g_expr.eval(asn);
                f_expr.eval(&a2)
            });
        }
    }
}

#[test]
fn compose_commutes_with_disjoint_restrict() {
    // For j != i: compose(f, x_i, g) | x_j = compose(f|x_j, x_i, g|x_j)
    let mut m = Manager::new(LEVEL_SEM);
    let f_expr = Expr::Or(
        Box::new(Expr::And(Box::new(Expr::Var(0)), Box::new(Expr::Var(1)))),
        Box::new(Expr::Xor(Box::new(Expr::Var(2)), Box::new(Expr::Var(5)))),
    );
    let g_expr = Expr::And(Box::new(Expr::Var(3)), Box::new(Expr::Var(6)));
    let f = f_expr.build(&mut m);
    let g = g_expr.build(&mut m);

    for i in 0..m.total_vars() {
        for j in 0..m.total_vars() {
            if i == j {
                continue;
            }
            for &v in &[true, false] {
                let lhs = {
                    let c = m.compose(f, i, g);
                    m.restrict(c, j, v)
                };
                let rhs = {
                    let f_rj = m.restrict(f, j, v);
                    let g_rj = m.restrict(g, j, v);
                    m.compose(f_rj, i, g_rj)
                };
                assert_eq!(
                    lhs, rhs,
                    "compose-restrict commute failed for i={}, j={}, v={}",
                    i, j, v
                );
            }
        }
    }
}

// -- Exists / Forall -------------------------------------------------

#[test]
fn exists_forall_constants() {
    let mut m = Manager::new(LEVEL_SEM);
    let t = m.mk_true();
    let f = m.mk_false();
    for i in 0..m.total_vars() {
        let et = m.exists(t, i);
        assert_eq!(et, t, "exists(true, x_{}) must equal true", i);
        let ef = m.exists(f, i);
        assert_eq!(ef, f, "exists(false, x_{}) must equal false", i);
        let at = m.forall(t, i);
        assert_eq!(at, t, "forall(true, x_{}) must equal true", i);
        let af = m.forall(f, i);
        assert_eq!(af, f, "forall(false, x_{}) must equal false", i);
    }
}

#[test]
fn exists_forall_self() {
    let mut m = Manager::new(LEVEL_SEM);
    let t = m.mk_true();
    let f = m.mk_false();
    for i in 0..m.total_vars() {
        let xi = m.mk_proj(i);
        let e = m.exists(xi, i);
        assert_eq!(e, t, "exists(x_{}, x_{}) must equal true", i, i);
        let a = m.forall(xi, i);
        assert_eq!(a, f, "forall(x_{}, x_{}) must equal false", i, i);
    }
}

#[test]
fn exists_forall_truth_table() {
    let mut m = Manager::new(LEVEL_SEM);
    let exprs = sample_expressions();
    for (label, e) in &exprs {
        let bdd = e.build(&mut m);
        for i in 0..m.total_vars() {
            let exi = m.exists(bdd, i);
            assert_truth_table_eq(&mut m, exi, |asn| {
                let mut at = asn.to_vec();
                at[i as usize] = true;
                let mut af = asn.to_vec();
                af[i as usize] = false;
                e.eval(&at) || e.eval(&af)
            });
            let _ = label;
            let fxi = m.forall(bdd, i);
            assert_truth_table_eq(&mut m, fxi, |asn| {
                let mut at = asn.to_vec();
                at[i as usize] = true;
                let mut af = asn.to_vec();
                af[i as usize] = false;
                e.eval(&at) && e.eval(&af)
            });
        }
    }
}
