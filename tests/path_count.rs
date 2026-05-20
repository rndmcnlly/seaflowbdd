//! Path count semantic tests (M7).
//!
//! Ground truth comes from a Rust shadow expression evaluator that
//! enumerates all `2^total_vars` assignments and counts those the
//! expression evaluates to true on (same pattern as
//! `eval_truth_tables.rs`). Path counts must match this enumeration.
//!
//! We restrict to LEVEL = 3 (8 variables, 256 assignments) to keep
//! enumeration cheap.

use seaflowbdd::Manager;

const LEVEL: u8 = 3;

#[derive(Clone, Debug)]
#[allow(dead_code)]
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

/// Enumerate all assignments and count those that satisfy `expr`.
fn truth_table_count(expr: &Expr, n_vars: u32) -> i128 {
    let mut count: i128 = 0;
    for v in 0..(1u64 << n_vars) {
        let asn: Vec<bool> = (0..n_vars).map(|i| (v >> i) & 1 == 1).collect();
        if expr.eval(&asn) {
            count += 1;
        }
    }
    count
}

fn check_against_enum(expr: &Expr, m: &mut Manager) {
    let bdd = expr.build(m);
    let n = m.total_vars();
    let expected = truth_table_count(expr, n);
    let actual = m.path_count(bdd);
    assert_eq!(
        actual, expected,
        "expr {:?}: expected {} satisfying assignments, got {}",
        expr, expected, actual
    );
}

#[test]
fn constants_path_count() {
    let mut m = Manager::new(LEVEL);
    let total: i128 = 1i128 << m.total_vars(); // 2^8 = 256
    let t = m.mk_true();
    let f = m.mk_false();
    assert_eq!(m.path_count(t), total);
    assert_eq!(m.path_count(f), 0);
}

#[test]
fn single_projection_path_count() {
    // Each x_i is satisfied by exactly half of the 2^total_vars
    // assignments.
    let mut m = Manager::new(LEVEL);
    let total = m.total_vars();
    let half: i128 = 1i128 << (total - 1);
    for i in 0..total {
        let p = m.mk_proj(i);
        assert_eq!(
            m.path_count(p),
            half,
            "x_{} should have 2^(n-1) = {} satisfying assignments",
            i,
            half
        );
    }
}

#[test]
fn and_of_n_distinct_projections() {
    // x_0 AND x_1 AND ... AND x_{k-1} should have 2^(total - k)
    // satisfying assignments.
    let mut m = Manager::new(LEVEL);
    let total = m.total_vars();
    for k in 1..=total.min(5) {
        let mut acc = m.mk_proj(0);
        for i in 1..k {
            let p = m.mk_proj(i);
            acc = m.and(acc, p);
        }
        let expected = 1i128 << (total - k);
        assert_eq!(
            m.path_count(acc),
            expected,
            "AND of x_0..x_{} should be 2^({} - {}) = {}",
            k - 1,
            total,
            k,
            expected
        );
    }
}

#[test]
fn or_of_n_distinct_projections() {
    // x_0 OR x_1 OR ... OR x_{k-1} satisfied by all assignments
    // *except* the all-zero-on-those-vars ones: 2^total - 2^(total - k).
    let mut m = Manager::new(LEVEL);
    let total = m.total_vars();
    for k in 1..=total.min(5) {
        let mut acc = m.mk_proj(0);
        for i in 1..k {
            let p = m.mk_proj(i);
            acc = m.or(acc, p);
        }
        let expected = (1i128 << total) - (1i128 << (total - k));
        assert_eq!(m.path_count(acc), expected, "OR of x_0..x_{}", k - 1);
    }
}

#[test]
fn xor_of_two_projections() {
    // x_i XOR x_j satisfied by exactly half of all assignments
    // (when i != j).
    let mut m = Manager::new(LEVEL);
    let total = m.total_vars();
    let half: i128 = 1i128 << (total - 1);
    for i in 0..total {
        for j in (i + 1)..total {
            let pi = m.mk_proj(i);
            let pj = m.mk_proj(j);
            let x = m.xor(pi, pj);
            assert_eq!(m.path_count(x), half, "x_{} XOR x_{} != half", i, j);
        }
    }
}

#[test]
fn negation_complements_count() {
    // For any f, path_count(f) + path_count(NOT f) = 2^total_vars.
    let mut m = Manager::new(LEVEL);
    let total: i128 = 1i128 << m.total_vars();

    let exprs = vec![
        Expr::Var(0),
        Expr::And(Box::new(Expr::Var(0)), Box::new(Expr::Var(1))),
        Expr::Or(Box::new(Expr::Var(2)), Box::new(Expr::Var(3))),
        Expr::Xor(Box::new(Expr::Var(0)), Box::new(Expr::Var(7))),
    ];
    for e in &exprs {
        let f = e.build(&mut m);
        let nf = m.not(f);
        let cf = m.path_count(f);
        let cnf = m.path_count(nf);
        assert_eq!(
            cf + cnf,
            total,
            "complement law failed for {:?}: {} + {} != {}",
            e,
            cf,
            cnf,
            total
        );
    }
}

#[test]
fn random_expressions_match_truth_table() {
    // A handful of constructed expressions cross-checked against
    // brute-force enumeration over 256 assignments.
    let mut m = Manager::new(LEVEL);

    // Helpers (Box-y syntax keeps these compact).
    fn v(i: u32) -> Expr {
        Expr::Var(i)
    }
    fn n(e: Expr) -> Expr {
        Expr::Not(Box::new(e))
    }
    fn a(x: Expr, y: Expr) -> Expr {
        Expr::And(Box::new(x), Box::new(y))
    }
    fn o(x: Expr, y: Expr) -> Expr {
        Expr::Or(Box::new(x), Box::new(y))
    }
    fn xo(x: Expr, y: Expr) -> Expr {
        Expr::Xor(Box::new(x), Box::new(y))
    }

    let exprs: Vec<Expr> = vec![
        // Distributivity LHS
        a(v(0), o(v(1), v(2))),
        // De Morgan rhs
        o(n(v(0)), n(v(1))),
        // Majority of 3 vars: at least 2 of x0, x1, x2 true
        o(a(v(0), v(1)), o(a(v(0), v(2)), a(v(1), v(2)))),
        // Parity of 4 vars
        xo(xo(v(0), v(1)), xo(v(2), v(3))),
        // Mixed
        a(o(v(0), n(v(1))), xo(v(2), v(7))),
        // (x0 -> x1) -> x0 == x0  (Pierce-like)
        o(n(o(n(v(0)), v(1))), v(0)),
        // tautology over a subset
        o(v(4), n(v(4))),
        // contradiction
        a(v(5), n(v(5))),
        // long AND
        a(v(0), a(v(1), a(v(2), a(v(3), v(4))))),
        // long XOR
        xo(v(0), xo(v(1), xo(v(2), xo(v(3), v(4))))),
    ];

    for e in &exprs {
        check_against_enum(e, &mut m);
    }
}

#[test]
fn cache_returns_consistent_results() {
    // Calling path_count twice on the same Bdd must return the same
    // answer (the second call exercises the memo cache).
    let mut m = Manager::new(LEVEL);
    let e = Expr::And(
        Box::new(Expr::Var(0)),
        Box::new(Expr::Or(Box::new(Expr::Var(1)), Box::new(Expr::Var(2)))),
    );
    let bdd = e.build(&mut m);
    let first = m.path_count(bdd);
    let second = m.path_count(bdd);
    assert_eq!(first, second);
    let expected = truth_table_count(&e, m.total_vars());
    assert_eq!(first, expected);
}

#[test]
fn level_one_constants() {
    // Total vars at level 1 = 2; expect 2^2 = 4 satisfying assignments
    // for true, 0 for false.
    let mut m = Manager::new(1);
    let t = m.mk_true();
    let f = m.mk_false();
    assert_eq!(m.path_count(t), 4);
    assert_eq!(m.path_count(f), 0);
}

#[test]
fn level_six_true_total_assignments() {
    // 2^64 fits in i128; verify the constant-true count at the largest
    // supported level.
    let mut m = Manager::new(6);
    let t = m.mk_true();
    assert_eq!(m.path_count(t), 1i128 << 64);
}

#[test]
#[should_panic(expected = "exceeds the i128 fast path")]
fn level_seven_panics() {
    let mut m = Manager::new(7);
    let t = m.mk_true();
    let _ = m.path_count(t);
}
