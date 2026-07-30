use super::*;

fn parse(input: &str) -> CursorShapeConfig {
    toml::from_str(input).unwrap()
}

#[test]
fn empty_cursor_shape_map_uses_bar_for_every_mode() {
    assert_eq!(parse(""), CursorShapeConfig::default());
}

#[test]
fn partial_cursor_shape_map_uses_bar_for_omitted_modes() {
    let config = parse(r#"normal = "block""#);
    assert_eq!(config.from_mode(Mode::Normal), CursorKind::Block);
    assert_eq!(config.from_mode(Mode::Select), CursorKind::Bar);
    assert_eq!(config.from_mode(Mode::Insert), CursorKind::Bar);
}

#[test]
fn complete_cursor_shape_map_round_trips_every_mode() {
    let config = parse(
        r#"
normal = "block"
select = "underline"
insert = "hidden"
"#,
    );
    assert_eq!(config.from_mode(Mode::Normal), CursorKind::Block);
    assert_eq!(config.from_mode(Mode::Select), CursorKind::Underline);
    assert_eq!(config.from_mode(Mode::Insert), CursorKind::Hidden);

    let serialized = toml::to_string(&config).unwrap();
    assert_eq!(parse(&serialized), config);
    assert!(serialized.contains("normal = \"block\""));
    assert!(serialized.contains("select = \"underline\""));
    assert!(serialized.contains("insert = \"hidden\""));
}

#[test]
fn default_cursor_shape_serializes_explicit_bar_modes() {
    let serialized = toml::to_string(&CursorShapeConfig::default()).unwrap();
    assert_eq!(parse(&serialized), CursorShapeConfig::default());
    for mode in ["normal", "select", "insert"] {
        assert!(serialized.contains(&format!("{mode} = \"bar\"")));
    }
}
