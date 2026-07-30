use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn insert_mode_cursor_position() -> anyhow::Result<()> {
    test(TestCase {
        in_text: String::new(),
        in_selection: Selection::single(0, 0),
        in_keys: "i".into(),
        out_text: String::new(),
        out_selection: Selection::single(0, 0),
        line_feed_handling: LineFeedHandling::AsIs,
    })
    .await?;

    // With edge-based selections, `i` collapses to head position (cursor)
    // Input: Range(0, 1) forward selection -> collapses to Range(1, 1) at head
    test(TestCase {
        in_text: "\n".into(),
        in_selection: Selection::single(0, 1),
        in_keys: "i".into(),
        out_text: "\n".into(),
        out_selection: Selection::single(1, 1),
        line_feed_handling: LineFeedHandling::AsIs,
    })
    .await?;

    test(TestCase {
        in_text: "\n".into(),
        in_selection: Selection::single(0, 1),
        in_keys: "i<esc>".into(),
        out_text: "\n".into(),
        out_selection: Selection::single(1, 1),
        line_feed_handling: LineFeedHandling::AsIs,
    })
    .await?;

    test(TestCase {
        in_text: "\n".into(),
        in_selection: Selection::single(0, 1),
        in_keys: "i<esc>i".into(),
        out_text: "\n".into(),
        out_selection: Selection::single(1, 1),
        line_feed_handling: LineFeedHandling::AsIs,
    })
    .await?;

    Ok(())
}

/// Range direction is preserved when escaping insert mode to normal
#[tokio::test(flavor = "multi_thread")]
async fn insert_to_normal_mode_cursor_position() -> anyhow::Result<()> {
    test(("#[f|]#oo\n", "vll<A-;><esc>", "#[|foo]#\n")).await?;
    test((
        indoc! {"\
                #[f|]#oo
                #(b|)#ar"
        },
        "vll<A-;><esc>",
        indoc! {"\
                #[|foo]#
                #(|bar)#"
        },
    ))
    .await?;

    // Append collapses every range to its head.
    test((
        indoc! {"\
                #[f|]#oo
                #(b|)#ar"
        },
        "a",
        indoc! {"\
                f#[|]#oo
                b#(|)#ar"
        },
    ))
    .await?;

    test((
        indoc! {"\
                #[f|]#oo
                #(b|)#ar"
        },
        "a<esc>",
        indoc! {"\
                f#[|]#oo
                b#(|)#ar"
        },
    ))
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn mode_transitions_preserve_edges_and_collapse_to_heads() -> anyhow::Result<()> {
    test(("#[ab|]#cd", "v", "#[ab|]#cd", LineFeedHandling::AsIs)).await?;
    test(("#[|ab]#cd", "v<esc>", "#[|ab]#cd", LineFeedHandling::AsIs)).await?;
    test(("  #[ab|]#cd", "I", "  #[|]#abcd", LineFeedHandling::AsIs)).await?;
    test(("  #[|ab]#cd", "A", "  abcd#[|]#", LineFeedHandling::AsIs)).await?;

    let text = "é\r\n🦀";
    let input = Selection::new(vec![Range::new(0, 1), Range::new(4, 3)].into(), 1);
    let points = Selection::new(vec![Range::point(1), Range::point(3)].into(), 1);
    for keys in ["i", "a", "i<esc>", "a<esc>"] {
        test(TestCase {
            in_text: text.into(),
            in_selection: input.clone(),
            in_keys: keys.into(),
            out_text: text.into(),
            out_selection: points.clone(),
            line_feed_handling: LineFeedHandling::AsIs,
        })
        .await?;
    }

    test(TestCase {
        in_text: "ab".into(),
        in_selection: Selection::new(vec![Range::new(0, 1), Range::new(2, 1)].into(), 1),
        in_keys: "i".into(),
        out_text: "ab".into(),
        out_selection: Selection::point(1),
        line_feed_handling: LineFeedHandling::AsIs,
    })
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "Stage 4: surround discovery requires the D8 adjacent-object affinity migration"]
async fn surround_by_character() -> anyhow::Result<()> {
    // Only pairs matching the passed character count
    test((
        "(so [many {go#[o|]#d} text] here)",
        "mi{",
        "(so [many {#[good|]#} text] here)",
    ))
    .await?;
    test((
        "(so [many {go#[o|]#d} text] here)",
        "mi[",
        "(so [#[many {good} text|]#] here)",
    ))
    .await?;
    test((
        "(so [many {go#[o|]#d} text] here)",
        "mi(",
        "(#[so [many {good} text] here|]#)",
    ))
    .await?;

    // Works with characters that aren't pairs too
    test((
        "'so 'many 'go#[o|]#d' text' here'",
        "mi'",
        "'so 'many '#[good|]#' text' here'",
    ))
    .await?;
    test((
        "'so 'many 'go#[o|]#d' text' here'",
        "2mi'",
        "'so '#[many 'good' text|]#' here'",
    ))
    .await?;
    test((
        "'so \"many 'go#[o|]#d' text\" here'",
        "mi\"",
        "'so \"#[many 'good' text|]#\" here'",
    ))
    .await?;

    // Selection direction is preserved
    test((
        "(so [many {go#[|od]#} text] here)",
        "mi{",
        "(so [many {#[|good]#} text] here)",
    ))
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "Stage 4: surround discovery requires the D8 adjacent-object affinity migration"]
async fn surround_inside_pair() -> anyhow::Result<()> {
    // Works at first character of buffer
    // TODO: Adjust test when opening pair failure is fixed
    test(("#[(|]#something)", "mim", "#[(|]#something)")).await?;

    // Inside a valid pair selects pair
    test(("some (#[t|]#ext) here", "mim", "some (#[text|]#) here")).await?;

    // On pair character selects pair
    // TODO: Opening pair character is a known failure case that needs addressing
    // test(("some #[(|]#text) here", "mim", "some (#[text|]#) here")).await?;
    test(("some (text#[)|]# here", "mim", "some (#[text|]#) here")).await?;

    // No valid pair does nothing
    test(("so#[m|]#e (text) here", "mim", "so#[m|]#e (text) here")).await?;

    // Count skips to outer pairs
    test((
        "(so (many (go#[o|]#d) text) here)",
        "1mim",
        "(so (many (#[good|]#) text) here)",
    ))
    .await?;
    test((
        "(so (many (go#[o|]#d) text) here)",
        "2mim",
        "(so (#[many (good) text|]#) here)",
    ))
    .await?;
    test((
        "(so (many (go#[o|]#d) text) here)",
        "3mim",
        "(#[so (many (good) text) here|]#)",
    ))
    .await?;

    // Matching pairs outside selection don't match
    test((
        "((so)((many) go#[o|]#d (text))(here))",
        "mim",
        "((so)(#[(many) good (text)|]#)(here))",
    ))
    .await?;
    test((
        "((so)((many) go#[o|]#d (text))(here))",
        "2mim",
        "(#[(so)((many) good (text))(here)|]#)",
    ))
    .await?;

    // Works with mixed braces
    test((
        "(so [many {go#[o|]#d} text] here)",
        "mim",
        "(so [many {#[good|]#} text] here)",
    ))
    .await?;
    test((
        "(so [many {go#[o|]#d} text] here)",
        "2mim",
        "(so [#[many {good} text|]#] here)",
    ))
    .await?;
    test((
        "(so [many {go#[o|]#d} text] here)",
        "3mim",
        "(#[so [many {good} text] here|]#)",
    ))
    .await?;

    // Selection direction is preserved
    test((
        "(so [many {go#[|od]#} text] here)",
        "mim",
        "(so [many {#[|good]#} text] here)",
    ))
    .await?;
    test((
        "(so [many {go#[|od]#} text] here)",
        "2mim",
        "(so [#[|many {good} text]#] here)",
    ))
    .await?;
    test((
        "(so [many {go#[|od]#} text] here)",
        "3mim",
        "(#[|so [many {good} text] here]#)",
    ))
    .await?;

    // Only pairs outside of full selection range are considered
    test((
        "(so (many (go#[od) |]#text) here)",
        "mim",
        "(so (#[many (good) text|]#) here)",
    ))
    .await?;
    test((
        "(so (many#[ (go|]#od) text) here)",
        "mim",
        "(so (#[many (good) text|]#) here)",
    ))
    .await?;
    test((
        "(so#[ (many (go|]#od) text) here)",
        "mim",
        "(#[so (many (good) text) here|]#)",
    ))
    .await?;
    test((
        "(so (many (go#[od) text) |]#here)",
        "mim",
        "(#[so (many (good) text) here|]#)",
    ))
    .await?;

    // Works with multiple cursors
    test((
        "(so (many (good) text) #[he|]#re\nso (many (good) text) #(|he)#re)",
        "mim",
        "(#[so (many (good) text) here\nso (many (good) text) here|]#)",
    ))
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "Stage 4: surround discovery requires the D8 adjacent-object affinity migration"]
async fn surround_around_pair() -> anyhow::Result<()> {
    // Works at first character of buffer
    // TODO: Adjust test when opening pair failure is fixed
    test(("#[(|]#something)", "mam", "#[(|]#something)")).await?;

    // Inside a valid pair selects pair
    test(("some (#[t|]#ext) here", "mam", "some #[(text)|]# here")).await?;

    // On pair character selects pair
    // TODO: Opening pair character is a known failure case that needs addressing
    // test(("some #[(|]#text) here", "mam", "some #[(text)|]# here")).await?;
    test(("some (text#[)|]# here", "mam", "some #[(text)|]# here")).await?;

    // No valid pair does nothing
    test(("so#[m|]#e (text) here", "mam", "so#[m|]#e (text) here")).await?;

    // Count skips to outer pairs
    test((
        "(so (many (go#[o|]#d) text) here)",
        "1mam",
        "(so (many #[(good)|]# text) here)",
    ))
    .await?;
    test((
        "(so (many (go#[o|]#d) text) here)",
        "2mam",
        "(so #[(many (good) text)|]# here)",
    ))
    .await?;
    test((
        "(so (many (go#[o|]#d) text) here)",
        "3mam",
        "#[(so (many (good) text) here)|]#",
    ))
    .await?;

    // Matching pairs outside selection don't match
    test((
        "((so)((many) go#[o|]#d (text))(here))",
        "mam",
        "((so)#[((many) good (text))|]#(here))",
    ))
    .await?;
    test((
        "((so)((many) go#[o|]#d (text))(here))",
        "2mam",
        "#[((so)((many) good (text))(here))|]#",
    ))
    .await?;

    // Works with mixed braces
    test((
        "(so [many {go#[o|]#d} text] here)",
        "mam",
        "(so [many #[{good}|]# text] here)",
    ))
    .await?;
    test((
        "(so [many {go#[o|]#d} text] here)",
        "2mam",
        "(so #[[many {good} text]|]# here)",
    ))
    .await?;
    test((
        "(so [many {go#[o|]#d} text] here)",
        "3mam",
        "#[(so [many {good} text] here)|]#",
    ))
    .await?;

    // Selection direction is preserved
    test((
        "(so [many {go#[|od]#} text] here)",
        "mam",
        "(so [many #[|{good}]# text] here)",
    ))
    .await?;
    test((
        "(so [many {go#[|od]#} text] here)",
        "2mam",
        "(so #[|[many {good} text]]# here)",
    ))
    .await?;
    test((
        "(so [many {go#[|od]#} text] here)",
        "3mam",
        "#[|(so [many {good} text] here)]#",
    ))
    .await?;

    // Only pairs outside of full selection range are considered
    test((
        "(so (many (go#[od) |]#text) here)",
        "mam",
        "(so #[(many (good) text)|]# here)",
    ))
    .await?;
    test((
        "(so (many#[ (go|]#od) text) here)",
        "mam",
        "(so #[(many (good) text)|]# here)",
    ))
    .await?;
    test((
        "(so#[ (many (go|]#od) text) here)",
        "mam",
        "#[(so (many (good) text) here)|]#",
    ))
    .await?;
    test((
        "(so (many (go#[od) text) |]#here)",
        "mam",
        "#[(so (many (good) text) here)|]#",
    ))
    .await?;

    // Works with multiple cursors
    test((
        "(so (many (good) text) #[he|]#re\nso (many (good) text) #(|he)#re)",
        "mam",
        "#[(so (many (good) text) here\nso (many (good) text) here)|]#",
    ))
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "Stage 4: match-around requires the D8 adjacent-object affinity migration"]
async fn match_around_closest_ts() -> anyhow::Result<()> {
    test_with_config(
        AppBuilder::new().with_file("foo.rs", None),
        (
            r#"fn main() {testing!{"f#[|oo]#)"};}"#,
            "mam",
            r#"fn main() {testing!{#[|"foo)"]#};}"#,
        ),
    )
    .await?;

    test_with_config(
        AppBuilder::new().with_file("foo.rs", None),
        (
            r##"fn main() { let _ = ("#[|1]#23", "#(|1)#23"); } "##,
            "3mam",
            r##"fn main() #[|{ let _ = ("123", "123"); }]# "##,
        ),
    )
    .await?;

    test_with_config(
        AppBuilder::new().with_file("foo.rs", None),
        (
            r##" fn main() { let _ = ("12#[|3", "12]#3"); } "##,
            "1mam",
            r##" fn main() { let _ = #[|("123", "123")]#; } "##,
        ),
    )
    .await?;

    Ok(())
}

/// Ensure the initial cursor in an opened file is a point at BOF.
#[tokio::test(flavor = "multi_thread")]
async fn cursor_position_newly_opened_file() -> anyhow::Result<()> {
    let test = |content: &str, expected_sel: Selection| -> anyhow::Result<()> {
        let file = helpers::temp_file_with_contents(content)?;
        let mut app = helpers::AppBuilder::new()
            .with_file(file.path(), None)
            .build()?;

        let (view, doc) = helix_view::current!(app.editor);
        let sel = doc.selection(view.id).clone();
        assert_eq!(expected_sel, sel);

        Ok(())
    };

    test("foo", Selection::point(0))?;
    test("👨‍👩‍👧‍👦 foo", Selection::point(0))?;
    test("", Selection::point(0))?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn cursor_position_append_eof() -> anyhow::Result<()> {
    // Append collapses a forward selection to its head.
    test(TestCase {
        in_text: "foo\n".into(),
        in_selection: Selection::single(0, 3), // forward selection of "foo", cursor at 3
        in_keys: "abar<esc>".into(),
        out_text: "foobar\n".into(),
        out_selection: Selection::point(6),
        line_feed_handling: LineFeedHandling::AsIs,
    })
    .await?;

    // Append collapses a backward selection to its head too.
    test(TestCase {
        in_text: "foo\n".into(),
        in_selection: Selection::single(3, 0), // backward selection, head at 0
        in_keys: "abar<esc>".into(),
        out_text: "barfoo\n".into(),
        out_selection: Selection::point(3),
        line_feed_handling: LineFeedHandling::AsIs,
    })
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "Stage 4: tree-sitter Select extension requires the object-affinity migration"]
async fn select_mode_tree_sitter_next_function_is_union_of_objects() -> anyhow::Result<()> {
    test_with_config(
        AppBuilder::new().with_file("foo.rs", None),
        (
            indoc! {"\
                #[/|]#// Increments
                fn inc(x: usize) -> usize { x + 1 }
                /// Decrements
                fn dec(x: usize) -> usize { x - 1 }
            "},
            "]fv]f",
            indoc! {"\
                 /// Increments
                 fn inc(x: usize) -> usize { x + 1 }#[
                 /// Decrements
                 fn dec(x: usize) -> usize { x - 1 }|]#
            "},
        ),
    )
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "Stage 4: tree-sitter Select extension requires the object-affinity migration"]
async fn select_mode_tree_sitter_prev_function_unselects_object() -> anyhow::Result<()> {
    test_with_config(
        AppBuilder::new().with_file("foo.rs", None),
        (
            indoc! {"\
                /// Increments
                #[fn inc(x: usize) -> usize { x + 1 }
                /// Decrements
                fn dec(x: usize) -> usize { x - 1 }|]#
            "},
            "v[f",
            indoc! {"\
                 /// Increments
                 #[|fn inc(x: usize) -> usize { x + 1 }
                 /// Decrements
                 fn dec(x: usize) -> usize { x - 1 }]#
            "},
        ),
    )
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "Stage 4: tree-sitter Select extension requires the object-affinity migration"]
async fn select_mode_tree_sitter_prev_function_goes_backwards_to_object() -> anyhow::Result<()> {
    // Note: the anchor stays put and the head moves back.
    test_with_config(
        AppBuilder::new().with_file("foo.rs", None),
        (
            indoc! {"\
                /// Increments
                fn inc(x: usize) -> usize { x + 1 }
                /// Decrements
                fn dec(x: usize) -> usize { x - 1 }
                /// Identity
                #[fn ident(x: usize) -> usize { x }|]#
            "},
            "v[f",
            indoc! {"\
                /// Increments
                fn inc(x: usize) -> usize { x + 1 }
                /// Decrements
                #[|fn dec(x: usize) -> usize { x - 1 }
                /// Identity
                fn ident(x: usize) -> usize { x }]#
            "},
        ),
    )
    .await?;

    test_with_config(
        AppBuilder::new().with_file("foo.rs", None),
        (
            indoc! {"\
                /// Increments
                fn inc(x: usize) -> usize { x + 1 }
                /// Decrements
                fn dec(x: usize) -> usize { x - 1 }
                /// Identity
                #[fn ident(x: usize) -> usize { x }|]#
            "},
            "v[f[f",
            indoc! {"\
                /// Increments
                #[|fn inc(x: usize) -> usize { x + 1 }
                /// Decrements
                fn dec(x: usize) -> usize { x - 1 }
                /// Identity
                fn ident(x: usize) -> usize { x }]#
            "},
        ),
    )
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn find_char() -> anyhow::Result<()> {
    test(("hel#[|l]#o\nhello", "fl", "hel#[l|]#o\nhello")).await?;
    test(("hel#[|l]#o\nhello", "2fl", "hel#[lo\nhel|]#lo")).await?;
    test(("hel#[|l]#o\nhello", "tl", "hel#[lo\nhe|]#llo")).await?;
    test(("hel#[l|]#o\nhello", "Fl", "hel#[|l]#o\nhello")).await?;
    test(("hello\nhel#[l|]#o", "Tl", "hello\nhel#[|l]#o")).await?;
    test(("hel#[|l]#o\nhello", "fx", "hel#[|l]#o\nhello")).await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn find_char_line_ending() -> anyhow::Result<()> {
    test(("on#[|e]#\ntwo\n", "f<ret>", "on#[e\n|]#two\n")).await?;
    test(("on#[|e]#\ntwo\n", "t<ret>", "on#[e|]#\ntwo\n")).await?;
    test(("one#[|\n]#two\n", "t<ret>", "one#[\ntwo|]#\n")).await?;
    test(("one\n#[|t]#wo\n", "F<ret>", "one#[|\n]#two\n")).await?;

    test(("on#[|e]#\r\ntwo\r\n", "f<ret>", "on#[e\r\n|]#two\r\n")).await?;
    test(("on#[|e]#\r\ntwo\r\n", "t<ret>", "on#[e|]#\r\ntwo\r\n")).await?;
    test(("one#[|\r\n]#two\r\n", "t<ret>", "one#[\r\ntwo|]#\r\n")).await?;
    test(("one\r\n#[|t]#wo\r\n", "F<ret>", "one#[|\r\n]#two\r\n")).await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn repeat_find_char() -> anyhow::Result<()> {
    test((
        indoc! {
            "\
            #[o|]#ne two
            one two"
        },
        "ft<A-.>",
        indoc! {
            "\
            one t#[wo
            one t|]#wo"
        },
    ))
    .await?;

    test((
        indoc! {
            "\
            #[o|]#ne two
            one two
            "
        },
        "f<ret><A-.>",
        indoc! {
            "\
            one two
            #[one two
            |]#"
        },
    ))
    .await?;

    test((
        indoc! {
            "\
            #[o|]#ne two
            one two
            "
        },
        "ftf<ret><A-.>",
        indoc! {
            "\
            one two
            #[one two
            |]#"
        },
    ))
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn copy_complete_line_preserves_half_open_range() -> anyhow::Result<()> {
    test(("#[a\n|]#b\nc\n", "C", "#(a\n|)##[b\n|]#c\n")).await?;
    test(("#[|a\n]#b\nc\n", "C", "#(|a\n)##[|b\n]#c\n")).await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_surround_replace() -> anyhow::Result<()> {
    test((
        indoc! {"\
            (#[|a]#)
            "},
        "mrm{",
        indoc! {"\
            {#[|a]#}
            "},
    ))
    .await?;

    test((
        indoc! {"\
            (#[a|]#)
            "},
        "mrm{",
        indoc! {"\
            {#[a|]#}
            "},
    ))
    .await?;

    test((
        indoc! {"\
            {{

            #(}|)#
            #[}|]#
            "},
        "mrm)",
        indoc! {"\
            ((

            #()|)#
            #[)|]#
            "},
    ))
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_surround_delete() -> anyhow::Result<()> {
    test((
        indoc! {"\
            (#[|a]#)
            "},
        "mdm",
        indoc! {"\
            #[|a]#
            "},
    ))
    .await?;

    test((
        indoc! {"\
            (#[a|]#)
            "},
        "mdm",
        indoc! {"\
            #[a|]#
            "},
    ))
    .await?;

    test((
        indoc! {"\
            {{

            #(}|)#
            #[}|]#
        "},
        "mdm",
        "\n\n#(|)#\n\n#[|]#\n\n",
        LineFeedHandling::AsIs,
    ))
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn tree_sitter_motions_work_across_injections() -> anyhow::Result<()> {
    test_with_config(
        AppBuilder::new().with_file("foo.html", None),
        (
            "<script>let #[|x]# = 1;</script>",
            "<A-o>",
            "<script>let #[|x = 1]#;</script>",
        ),
    )
    .await?;

    // When the full injected layer is selected, expand_selection jumps to
    // a more shallow layer.
    test_with_config(
        AppBuilder::new().with_file("foo.html", None),
        (
            "<script>#[|let x = 1;]#</script>",
            "<A-o>",
            "#[|<script>let x = 1;</script>]#",
        ),
    )
    .await?;

    test_with_config(
        AppBuilder::new().with_file("foo.html", None),
        (
            "<script>let #[|x = 1]#;</script>",
            "<A-i>",
            "<script>let #[|x]# = 1;</script>",
        ),
    )
    .await?;

    test_with_config(
        AppBuilder::new().with_file("foo.html", None),
        (
            "<script>let #[|x]# = 1;</script>",
            "<A-n>",
            "<script>let x #[=|]# 1;</script>",
        ),
    )
    .await?;

    test_with_config(
        AppBuilder::new().with_file("foo.html", None),
        (
            "<script>let #[|x]# = 1;</script>",
            "<A-p>",
            "<script>#[|let]# x = 1;</script>",
        ),
    )
    .await?;

    Ok(())
}
