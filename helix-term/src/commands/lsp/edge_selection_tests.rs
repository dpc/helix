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
