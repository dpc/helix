use super::*;

#[test]
fn aligned_highlight_containment_preserves_nonfirst_primary() {
    let text = helix_core::Rope::from("a\u{301}b");
    let ranges = [helix_core::Range::new(2, 3), helix_core::Range::new(1, 2)];

    assert!(!ranges[1].contains(0));
    assert_eq!(aligned_primary_index(text.slice(..), &ranges, 0), 1);
}

#[test]
fn aligned_point_containment_preserves_primary() {
    let text = helix_core::Rope::from("a\u{301}b");
    let ranges = [helix_core::Range::new(2, 3), helix_core::Range::point(1)];

    assert_eq!(aligned_primary_index(text.slice(..), &ranges, 0), 1);
}

#[test]
fn code_action_diagnostics_use_half_open_point_ownership() {
    let diagnostic = helix_core::Range::new(2, 5);
    assert!(!range_owns_diagnostic(
        helix_core::Range::point(1),
        diagnostic
    ));
    assert!(range_owns_diagnostic(
        helix_core::Range::point(2),
        diagnostic
    ));
    assert!(range_owns_diagnostic(
        helix_core::Range::point(4),
        diagnostic
    ));
    assert!(!range_owns_diagnostic(
        helix_core::Range::point(5),
        diagnostic
    ));
    assert!(range_owns_diagnostic(
        helix_core::Range::point(5),
        helix_core::Range::point(5)
    ));
    assert!(range_owns_diagnostic(
        helix_core::Range::new(6, 4),
        diagnostic
    ));
}

#[test]
fn rename_prefill_distinguishes_points_from_one_scalar_selections() {
    let text = helix_core::Rope::from("word");
    assert_eq!(
        rename_prefill(text.slice(..), helix_core::Range::point(0)),
        "word"
    );
    assert_eq!(
        rename_prefill(text.slice(..), helix_core::Range::new(0, 1)),
        "w"
    );
    assert_eq!(
        rename_prefill(text.slice(..), helix_core::Range::new(1, 0)),
        "w"
    );
    assert_eq!(
        rename_prefill(text.slice(..), helix_core::Range::point(4)),
        ""
    );
}
