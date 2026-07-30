use std::sync::Arc;

use arc_swap::{access::Map, ArcSwap};
use helix_core::syntax;
use helix_loader::workspace_trust::WorkspaceTrust;
use helix_view::{
    graphics::{CursorKind, Rect},
    theme, Editor,
};

use crate::{compositor::Component, config::Config, handlers};

use super::{Picker, PickerColumn, Prompt};

fn editor_with_block_insert_cursor() -> Editor {
    let mut config = Config::default();
    config.editor.cursor_shape = toml::from_str(r#"insert = "block""#).unwrap();
    let config = Arc::new(ArcSwap::from_pointee(config));
    let handlers = handlers::setup(config.clone());

    Editor::new(
        Rect::new(0, 0, 120, 40),
        Arc::new(theme::Loader::new(&[])),
        Arc::new(ArcSwap::from_pointee(syntax::Loader::default())),
        Arc::new(Map::new(Arc::clone(&config), |config: &Config| {
            &config.editor
        })),
        handlers,
        WorkspaceTrust::fully_trusted(),
    )
}

#[tokio::test]
async fn prompt_and_picker_ignore_document_cursor_shape_overrides() {
    let editor = editor_with_block_insert_cursor();
    assert_eq!(
        editor
            .config()
            .cursor_shape
            .from_mode(helix_view::document::Mode::Insert),
        CursorKind::Block
    );

    let prompt = Prompt::new(">".into(), None, |_, _| Vec::new(), |_, _, _| {});
    assert_eq!(
        prompt.cursor(Rect::new(0, 0, 40, 1), &editor).1,
        CursorKind::Bar
    );

    let picker = Picker::new(
        [PickerColumn::new("item", |item: &String, _data: &()| {
            item.as_str().into()
        })],
        0,
        ["item".to_owned()],
        (),
        |_context, _item, _action| {},
    );
    assert_eq!(
        picker.cursor(Rect::new(0, 0, 80, 20), &editor).1,
        CursorKind::Bar
    );
}
