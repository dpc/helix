use super::*;

#[test]
fn line_end_newline_position_stays_within_document() {
    for (source, line, expected) in [
        ("abc\n", 0, 4),
        ("abc\r\n", 0, 5),
        ("abc", 0, 3),
        ("abc\nlast", 1, 8),
    ] {
        let text = Rope::from(source);
        assert_eq!(line_end_newline_position(text.slice(..), line), expected);
        assert!(expected <= text.len_chars());
    }
}

#[test]
fn empty_hunks_are_points_at_their_anchor() {
    let text = Rope::from("first\nmiddle\nlast");

    for (line, expected) in [(0, 0), (1, 6), (3, 17)] {
        let hunk = Hunk {
            before: line..line + 1,
            after: line..line,
        };
        assert_eq!(hunk_range(hunk, text.slice(..)), Range::point(expected));
    }
}

#[test]
fn copy_range_to_rows_preserves_edge_shape_and_direction() {
    let text = Rope::from("abcd\nabcd\nx\nabcd");
    let text = text.slice(..);

    assert_eq!(
        copy_range_to_rows(text, Range::new(1, 3), 1, 1, 4),
        Some(Range::new(6, 8))
    );
    assert_eq!(
        copy_range_to_rows(text, Range::new(3, 1), 1, 1, 4),
        Some(Range::new(8, 6))
    );
    assert_eq!(
        copy_range_to_rows(text, Range::point(2), 1, 1, 4),
        Some(Range::point(7))
    );
    assert_eq!(copy_range_to_rows(text, Range::new(1, 3), 2, 2, 4), None);
}

#[test]
fn copy_range_to_rows_preserves_unicode_and_crlf_edges() {
    let text = Rope::from("a\u{301}b\r\na\u{301}b\r\n");
    assert_eq!(
        copy_range_to_rows(text.slice(..), Range::new(0, 2), 1, 1, 4),
        Some(Range::new(5, 7))
    );
}

#[test]
fn copied_range_height_uses_half_open_line_ownership() {
    for (source, range) in [
        ("a\nb\nc\n", Range::new(0, 2)),
        ("a\r\nb\r\nc\r\n", Range::new(0, 3)),
    ] {
        let text = Rope::from(source);
        assert_eq!(copied_range_height(text.slice(..), range), 1);
        assert_eq!(
            copy_range_to_rows(text.slice(..), range, 1, 2, 4),
            Some(Range::new(range.to(), range.to() * 2))
        );
    }
}

#[test]
fn directional_command_targets_apply_mode_and_travel_head() {
    let input = Range::new(2, 5);
    let target = Range::new(10, 14);

    assert_eq!(
        select_directional_target(input, target, Direction::Forward, Mode::Normal),
        Range::new(10, 14)
    );
    assert_eq!(
        select_directional_target(input, target, Direction::Backward, Mode::Normal),
        Range::new(14, 10)
    );
    assert_eq!(
        select_directional_target(input, target, Direction::Forward, Mode::Select),
        Range::new(2, 14)
    );
    assert_eq!(
        select_directional_target(input, target, Direction::Backward, Mode::Select),
        Range::new(2, 10)
    );
}
