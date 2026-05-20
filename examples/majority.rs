//! Count satisfying assignments of a "majority-of-N" function.
//!
//! Builds the boolean function that is true exactly when at least half
//! of the input variables are true, then asks the engine how many
//! assignments satisfy it. Compares against the closed-form answer.
//!
//! Run with: `cargo run --example majority`

use seaflowbdd::Manager;

fn main() {
    // Level 3 ⇒ 8 variables. We'll work with the first 5 of them
    // (a five-bit majority function) but the Bdd is over all 8.
    const N: u32 = 5;
    let mut m = Manager::new(3);
    assert!(N <= m.total_vars());

    // Build f = "at least 3 of x_0..x_4 are true".
    //
    // Strategy: enumerate every 3-subset of the N variables, AND the
    // members of the subset, OR the resulting "all in this subset are
    // true" cubes together. This is O(C(N, k)) Bdds, fine for N=5.
    let threshold = N.div_ceil(2); // 3 for N=5
    let mut f = m.mk_false();
    for mask in 0..(1u32 << N) {
        if mask.count_ones() < threshold {
            continue;
        }
        // For this subset, AND together the variables whose bit is set
        // in `mask`. (The other N-k variables are unconstrained.)
        let mut cube = m.mk_true();
        for i in 0..N {
            if (mask >> i) & 1 == 1 {
                let xi = m.mk_proj(i);
                cube = m.and(cube, xi);
            }
        }
        f = m.or(f, cube);
    }

    // What we just built: "at least one k-subset is fully on", which is
    // equivalent to "at least k bits are on" (any k-or-more subset has
    // a k-subset fully on).
    let count = m.path_count(f);

    // Closed form: number of assignments to all 2^total_vars variables
    // where at least `threshold` of the first N are true.
    //
    // = (sum over j in threshold..=N of C(N, j)) * 2^(total_vars - N)
    let mut majority_assignments_of_n: u128 = 0;
    for j in threshold..=N {
        majority_assignments_of_n += binomial(N as u128, j as u128);
    }
    let total_vars = m.total_vars() as u128;
    let unconstrained_factor = 1u128 << (total_vars - N as u128);
    let expected = (majority_assignments_of_n * unconstrained_factor) as i128;

    println!("majority-of-{} over {} total vars", N, m.total_vars());
    println!("  satisfying assignments: {}", count);
    println!("  expected (closed form): {}", expected);
    assert_eq!(count, expected);

    // The Bdd's structural size is much smaller than the explicit
    // C(N, threshold) + ... cube enumeration we used to build it.
    println!(
        "  Bdd size: {} nodes, {} edges",
        m.reachable_node_count(f),
        m.reachable_edge_count(f)
    );

    // We can also ask: "given x_0 = true, how many of the remaining
    // assignments still satisfy?" by restricting and re-counting.
    let f_x0 = m.restrict(f, 0, true);
    let count_x0_true = m.path_count(f_x0);
    println!("  with x_0 = 1: {} satisfying assignments", count_x0_true);
}

fn binomial(n: u128, k: u128) -> u128 {
    if k > n {
        return 0;
    }
    let k = k.min(n - k);
    let mut result: u128 = 1;
    for i in 0..k {
        result = result * (n - i) / (i + 1);
    }
    result
}
