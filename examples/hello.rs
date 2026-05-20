//! Minimal example: build, evaluate, and count satisfying assignments.
//!
//! Run with: `cargo run --example hello`

use seaflowbdd::Manager;

fn main() {
    // A manager fixed at level 2 represents boolean functions over
    // 2^2 = 4 variables: x_0, x_1, x_2, x_3.
    let mut m = Manager::new(2);

    // Build x_0 AND x_1.
    let x0 = m.mk_proj(0);
    let x1 = m.mk_proj(1);
    let f = m.and(x0, x1);

    // Evaluate on a few assignments.
    // The assignment slice is [x_0, x_1, x_2, x_3].
    println!("f = x_0 AND x_1");
    println!(
        "f([1, 1, 0, 0]) = {}",
        m.evaluate(f, &[true, true, false, false])
    );
    println!(
        "f([1, 0, 1, 1]) = {}",
        m.evaluate(f, &[true, false, true, true])
    );
    println!(
        "f([0, 1, 1, 1]) = {}",
        m.evaluate(f, &[false, true, true, true])
    );

    // Count satisfying assignments. Out of 2^4 = 16 total assignments,
    // exactly those with x_0 = x_1 = 1 satisfy f, regardless of x_2/x_3.
    // That's 4 satisfying assignments.
    println!("path_count(f) = {}", m.path_count(f));
    assert_eq!(m.path_count(f), 4);

    // Restrict: substitute x_0 = true. The result is just x_1 lifted
    // back to a 4-var function. Path count is now 8.
    let f_x0_true = m.restrict(f, 0, true);
    println!("path_count(f | x_0 = 1) = {}", m.path_count(f_x0_true));
    assert_eq!(m.path_count(f_x0_true), 8);

    // Bdd equality is canonical: structurally equal Bdds compare equal
    // by id, no traversal needed.
    let f_again = {
        let a = m.mk_proj(0);
        let b = m.mk_proj(1);
        m.and(a, b)
    };
    assert_eq!(f, f_again);

    println!(
        "\nstructural size of f: {} nodes, {} edges",
        m.reachable_node_count(f),
        m.reachable_edge_count(f)
    );
}
