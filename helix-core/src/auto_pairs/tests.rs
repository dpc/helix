use super::*;

#[test]
fn paired_backspace_keeps_point_at_deleted_pair_start() {
    for (source, point, deletion, result) in [
        ("()", 1, (0, 2), Range::point(0)),
        ("x()", 2, (1, 3), Range::point(1)),
        ("{\n\n}", 2, (1, 3), Range::point(1)),
    ] {
        let text = Rope::from(source);
        assert_eq!(
            handle_delete(&text, &Range::point(point)),
            Some((deletion, result))
        );
    }
}
