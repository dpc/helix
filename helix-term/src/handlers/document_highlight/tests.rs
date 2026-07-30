use super::*;
use helix_core::Rope;

#[test]
fn highlight_ranges_preserve_points_boundaries_and_multibyte_offsets() {
    let text = Rope::from("a🦀b");
    let highlight = |start, end| lsp::DocumentHighlight {
        range: lsp::Range::new(lsp::Position::new(0, start), lsp::Position::new(0, end)),
        kind: None,
    };

    assert_eq!(
        document_highlight_ranges(
            &text,
            OffsetEncoding::Utf8,
            vec![highlight(0, 0), highlight(1, 5), highlight(6, 6)]
        ),
        vec![0..0, 1..2, 3..3]
    );
    assert_eq!(
        document_highlight_ranges(
            &text,
            OffsetEncoding::Utf16,
            vec![highlight(1, 1), highlight(1, 3), highlight(4, 4)]
        ),
        vec![1..1, 1..2, 3..3]
    );
    assert_eq!(
        document_highlight_ranges(
            &text,
            OffsetEncoding::Utf32,
            vec![
                highlight(0, 3),
                highlight(1, 1),
                highlight(1, 2),
                highlight(1, 1),
                highlight(3, 3),
            ]
        ),
        vec![0..3, 1..1, 1..2, 1..1, 3..3]
    );
    assert_eq!(
        document_highlight_ranges(
            &text,
            OffsetEncoding::Utf32,
            vec![highlight(0, 1), highlight(1, 1), highlight(1, 2)]
        ),
        vec![0..1, 1..1, 1..2]
    );
}
