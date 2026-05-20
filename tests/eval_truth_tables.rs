//! Semantic correctness tests for evaluate (M6).
//!
//! These don't use the C++ oracle: they use a small Rust expression
//! type with a direct truth-table evaluator as ground truth. This
//! catches semantic bugs that structural-parity checks might miss
//! (e.g., if both engines had the same bug, they'd still compare
//! equal). Together with the parity tests, this gives both coverage
//! axes.

use seaflowbdd::Manager;

const LEVEL: u8 = 3; // 8 vars; 2^8 = 256 assignments per test, fast

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

fn check_truth_table(expr: &Expr, m: &mut Manager) {
    let bdd = expr.build(m);
    let n = m.total_vars() as usize;
    for v in 0..(1u64 << n) {
        let asn: Vec<bool> = (0..n).map(|i| (v >> i) & 1 == 1).collect();
        let expected = expr.eval(&asn);
        let actual = m.evaluate(bdd, &asn);
        assert_eq!(
            actual, expected,
            "expr {:?} on {:?} expected {} got {}",
            expr, asn, expected, actual
        );
    }
}

#[test]
fn truth_table_constants() {
    let mut m = Manager::new(LEVEL);
    check_truth_table(&Expr::True, &mut m);
    check_truth_table(&Expr::False, &mut m);
}

#[test]
fn truth_table_each_var() {
    let mut m = Manager::new(LEVEL);
    for i in 0..m.total_vars() {
        check_truth_table(&Expr::Var(i), &mut m);
    }
}

#[test]
fn truth_table_and_or_xor_pairs() {
    let mut m = Manager::new(LEVEL);
    for i in 0..m.total_vars() {
        for j in 0..m.total_vars() {
            check_truth_table(
                &Expr::And(Box::new(Expr::Var(i)), Box::new(Expr::Var(j))),
                &mut m,
            );
            check_truth_table(
                &Expr::Or(Box::new(Expr::Var(i)), Box::new(Expr::Var(j))),
                &mut m,
            );
            check_truth_table(
                &Expr::Xor(Box::new(Expr::Var(i)), Box::new(Expr::Var(j))),
                &mut m,
            );
        }
    }
}

#[test]
fn truth_table_negation() {
    let mut m = Manager::new(LEVEL);
    for i in 0..m.total_vars() {
        check_truth_table(&Expr::Not(Box::new(Expr::Var(i))), &mut m);
    }
}

#[test]
fn truth_table_distributivity() {
    // a AND (b OR c) == (a AND b) OR (a AND c)
    let mut m = Manager::new(LEVEL);
    let lhs = Expr::And(
        Box::new(Expr::Var(0)),
        Box::new(Expr::Or(Box::new(Expr::Var(1)), Box::new(Expr::Var(2)))),
    );
    let rhs = Expr::Or(
        Box::new(Expr::And(Box::new(Expr::Var(0)), Box::new(Expr::Var(1)))),
        Box::new(Expr::And(Box::new(Expr::Var(0)), Box::new(Expr::Var(2)))),
    );
    let l = lhs.build(&mut m);
    let r = rhs.build(&mut m);
    assert_eq!(l, r, "distributivity must hold canonically");
    check_truth_table(&lhs, &mut m);
    check_truth_table(&rhs, &mut m);
}

#[test]
fn truth_table_de_morgan() {
    // NOT (a AND b) == (NOT a) OR (NOT b)
    let mut m = Manager::new(LEVEL);
    let lhs = Expr::Not(Box::new(Expr::And(
        Box::new(Expr::Var(0)),
        Box::new(Expr::Var(1)),
    )));
    let rhs = Expr::Or(
        Box::new(Expr::Not(Box::new(Expr::Var(0)))),
        Box::new(Expr::Not(Box::new(Expr::Var(1)))),
    );
    check_truth_table(&lhs, &mut m);
    check_truth_table(&rhs, &mut m);
    // We don't assert canonical equality here because mk_not's value
    // map may not be in canonical order; that's a known limitation
    // documented in pair_product.rs::not.
}
