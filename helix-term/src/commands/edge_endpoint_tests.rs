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
