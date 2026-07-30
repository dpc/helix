use super::*;

use std::sync::Arc;

use arc_swap::ArcSwap;
use helix_core::{syntax::Loader, Rope};
use helix_view::{
    editor::{Config, GutterConfig},
    theme::DEFAULT_THEME,
    view::ViewPosition,
    DocumentId,
};

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

#[test]
fn cursor_render_plan_covers_focus_kind_and_primary_matrix() {
    for kind in [
        CursorKind::Bar,
        CursorKind::Underline,
        CursorKind::Block,
        CursorKind::Hidden,
    ] {
        for terminal_focused in [false, true] {
            let plan = CursorRenderPlan::new(kind, terminal_focused);

            assert!(
                plan.renders_manually(false),
                "{kind:?}, focused={terminal_focused}"
            );
            assert_eq!(
                plan.renders_manually(true),
                !terminal_focused || kind == CursorKind::Block,
                "{kind:?}, focused={terminal_focused}"
            );
            assert_eq!(
                plan.terminal_kind,
                if terminal_focused && kind != CursorKind::Block {
                    kind
                } else {
                    CursorKind::Hidden
                },
                "{kind:?}, focused={terminal_focused}"
            );
        }
    }
}

#[test]
fn eof_cursor_marker_does_not_invent_a_document_span() {
    let text = helix_core::Rope::from("a");
    assert_eq!(
        ManualCursorMarker::at(text.slice(..), 0),
        ManualCursorMarker::Document(0..1)
    );
    assert_eq!(
        ManualCursorMarker::at(text.slice(..), 1),
        ManualCursorMarker::Eof(1)
    );
}

#[test]
fn mouse_yank_gesture_state_handles_click_drag_return_and_stale_release() {
    let mut press = None;
    let mut dragged = false;

    begin_mouse_selection(&mut press, &mut dragged, Range::point(1));
    assert!(!finish_mouse_selection(
        &mut press,
        &mut dragged,
        Range::point(1)
    ));

    begin_mouse_selection(&mut press, &mut dragged, Range::point(1));
    drag_mouse_selection(press, &mut dragged);
    assert!(finish_mouse_selection(
        &mut press,
        &mut dragged,
        Range::new(1, 2)
    ));

    begin_mouse_selection(&mut press, &mut dragged, Range::point(2));
    drag_mouse_selection(press, &mut dragged);
    assert!(finish_mouse_selection(
        &mut press,
        &mut dragged,
        Range::new(2, 1)
    ));

    // A Select-mode drag remains a gesture after returning to its press range.
    let selected = Range::new(0, 1);
    begin_mouse_selection(&mut press, &mut dragged, selected);
    drag_mouse_selection(press, &mut dragged);
    assert!(finish_mouse_selection(&mut press, &mut dragged, selected));

    assert!(!finish_mouse_selection(&mut press, &mut dragged, selected));
    begin_mouse_selection(&mut press, &mut dragged, selected);
    cancel_mouse_selection(&mut press, &mut dragged);
    assert!(!finish_mouse_selection(&mut press, &mut dragged, selected));
}

#[test]
fn select_mode_mouse_extension_preserves_secondary_ranges() {
    let text = Rope::from("abcd");
    let selection = Selection::single(0, 1).push(Range::point(3));
    let primary = selection.primary();
    let secondary = selection
        .iter()
        .copied()
        .find(|range| *range != primary)
        .unwrap();
    let extended = extend_primary_selection(selection, text.slice(..), 4);

    assert_eq!(extended.len(), 2);
    assert!(extended.iter().any(|range| *range == secondary));
    assert_eq!(extended.primary(), Range::new(3, 4));
}

#[test]
fn selection_highlights_render_one_unfocused_primary_fallback() {
    let config = Arc::new(ArcSwap::new(Arc::new(Config::default())));
    let mut doc = Document::from(
        Rope::from("a"),
        None,
        config.clone(),
        Arc::new(ArcSwap::from_pointee(Loader::default())),
    );
    let view = View::new(DocumentId::default(), GutterConfig::default());
    doc.ensure_view_init(view.id);
    doc.set_selection(
        view.id,
        Selection::point(0).push(Range::point(doc.text().len_chars())),
    );

    for (name, kind) in [
        ("bar", CursorKind::Bar),
        ("underline", CursorKind::Underline),
        ("block", CursorKind::Block),
        ("hidden", CursorKind::Hidden),
    ] {
        let cursor_shape: CursorShapeConfig =
            toml::from_str(&format!(r#"normal = "{name}""#)).unwrap();
        for terminal_focused in [false, true] {
            let highlights = EditorView::doc_selection_highlights(
                Mode::Normal,
                &doc,
                &view,
                &DEFAULT_THEME,
                &cursor_shape,
                terminal_focused,
            );
            let OverlayHighlights::Heterogenous {
                highlights: document_cursors,
            } = &highlights.overlays[0]
            else {
                panic!("expected secondary document cursor");
            };
            assert_eq!(
                document_cursors.len(),
                1,
                "{kind:?}, focused={terminal_focused}"
            );
            assert_eq!(
                highlights.eof_cursors.len(),
                usize::from(!terminal_focused || kind == CursorKind::Block),
                "{kind:?}, focused={terminal_focused}"
            );
            if let Some(marker) = highlights.eof_cursors.first() {
                assert_eq!(marker.position, doc.text().len_chars());
            }
        }
    }
}

#[test]
fn manual_cursor_markers_clip_to_the_viewport_and_split_origin() {
    let inner = Rect::new(40, 5, 3, 2);
    assert_eq!(
        cursor_surface_position(inner, Position::new(0, 0)),
        Some((40, 5))
    );
    assert_eq!(
        cursor_surface_position(inner, Position::new(1, 2)),
        Some((42, 6))
    );
    assert_eq!(cursor_surface_position(inner, Position::new(0, 3)), None);
    assert_eq!(cursor_surface_position(inner, Position::new(2, 0)), None);
    assert_eq!(
        cursor_surface_position(inner, Position::new(0, usize::MAX)),
        None
    );

    let mut surface = Surface::empty(Rect::new(0, 0, 50, 10));
    let style = Style::default().bg(Color::Red);
    assert!(render_manual_cursor(
        &mut surface,
        inner,
        Position::new(1, 2),
        style,
    ));
    assert_eq!(surface[(42, 6)].style().bg, style.bg);
    assert!(!render_manual_cursor(
        &mut surface,
        inner,
        Position::new(0, 3),
        style,
    ));
}

#[test]
fn colliding_manual_cursor_markers_are_deduplicated() {
    let config = Arc::new(ArcSwap::new(Arc::new(Config::default())));
    let mut doc = Document::from(
        Rope::from("ab"),
        None,
        config.clone(),
        Arc::new(ArcSwap::from_pointee(Loader::default())),
    );
    let view = View::new(DocumentId::default(), GutterConfig::default());
    doc.ensure_view_init(view.id);

    // Adjacent forward and backward ranges can share the same head.
    doc.set_selection(view.id, Selection::single(2, 1).push(Range::new(0, 1)));
    let highlights = EditorView::doc_selection_highlights(
        Mode::Normal,
        &doc,
        &view,
        &DEFAULT_THEME,
        &config.load().cursor_shape,
        false,
    );
    let OverlayHighlights::Heterogenous {
        highlights: cursors,
    } = &highlights.overlays[1]
    else {
        panic!("expected cursor overlays");
    };
    assert_eq!(cursors.len(), 1);
    assert_eq!(cursors[0].1, 1..2);
    assert_eq!(
        cursors[0].0,
        DEFAULT_THEME
            .find_highlight("ui.cursor.primary.normal")
            .unwrap()
    );

    let block: CursorShapeConfig = toml::from_str(r#"normal = "block""#).unwrap();
    let highlights = EditorView::doc_selection_highlights(
        Mode::Normal,
        &doc,
        &view,
        &DEFAULT_THEME,
        &block,
        true,
    );
    let OverlayHighlights::Heterogenous {
        highlights: cursors,
    } = &highlights.overlays[1]
    else {
        panic!("expected cursor overlays");
    };
    assert_eq!(cursors.len(), 1);
    assert_eq!(cursors[0].1, 1..2);

    // A point at a range's upper edge is distinct but shares its EOF marker.
    doc.set_selection(
        view.id,
        Selection::single(0, 2).push(Range::point(doc.text().len_chars())),
    );
    let highlights = EditorView::doc_selection_highlights(
        Mode::Normal,
        &doc,
        &view,
        &DEFAULT_THEME,
        &config.load().cursor_shape,
        false,
    );
    assert_eq!(highlights.eof_cursors.len(), 1);
    assert_eq!(
        highlights.eof_cursors[0].highlight,
        DEFAULT_THEME
            .find_highlight("ui.cursor.primary.normal")
            .unwrap()
    );
}

#[test]
fn secondary_eof_marker_is_rejected_left_of_a_scrolled_view() {
    let config = Arc::new(ArcSwap::new(Arc::new(Config::default())));
    let mut doc = Document::from(
        Rope::from("0123456789\nx"),
        None,
        config.clone(),
        Arc::new(ArcSwap::from_pointee(Loader::default())),
    );
    let mut view = View::new(DocumentId::default(), GutterConfig::default());
    view.area = Rect::new(20, 4, 10, 4);
    doc.ensure_view_init(view.id);
    doc.set_view_offset(
        view.id,
        ViewPosition {
            anchor: 0,
            horizontal_offset: 5,
            ..ViewPosition::default()
        },
    );
    doc.set_selection(
        view.id,
        Selection::point(doc.text().len_chars()).push(Range::point(0)),
    );

    let highlights = EditorView::doc_selection_highlights(
        Mode::Normal,
        &doc,
        &view,
        &DEFAULT_THEME,
        &config.load().cursor_shape,
        true,
    );
    assert_eq!(highlights.eof_cursors.len(), 1);
    let marker = &highlights.eof_cursors[0];
    assert_eq!(
        view.screen_coords_at_pos(&doc, doc.text().slice(..), marker.position),
        None
    );
}
