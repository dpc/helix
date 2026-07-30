use super::*;
fn diagnostic(start: usize, end: usize) -> helix_core::diagnostic::Range {
    helix_core::diagnostic::Range { start, end }
}

#[test]
fn diagnostics_use_half_open_edges_and_retain_zero_width_eof() {
    let left = diagnostic(0, 2);
    let right = diagnostic(2, 4);
    let eof = diagnostic(4, 4);

    assert!(diagnostic_range_owns_edge(left, 0));
    assert!(!diagnostic_range_owns_edge(left, 2));
    assert!(diagnostic_range_owns_edge(right, 2));
    assert!(!diagnostic_range_owns_edge(right, 4));
    assert!(diagnostic_range_owns_edge(eof, 4));
}
