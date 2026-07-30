use helix_term::application::Application;

use super::*;

mod insert;
mod movement;
mod reverse_selection_contents;
mod rotate_selection_contents;
mod write;

#[tokio::test(flavor = "multi_thread")]
async fn search_selection_detect_word_boundaries_at_eof() -> anyhow::Result<()> {
    // <https://github.com/helix-editor/helix/issues/12609>
    test((
        indoc! {"\
            #[o|]#ne
            two
            three"},
        "gej*h",
        indoc! {"\
            one
            two
            three#[|
            ]#"},
    ))
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_selection_duplication() -> anyhow::Result<()> {
    // Forward
    test((
        indoc! {"\
            #[lo|]#rem
            ipsum
            dolor
            "},
        "CC",
        indoc! {"\
            #(lo|)#rem
            #(ip|)#sum
            #[do|]#lor
            "},
    ))
    .await?;

    // Backward
    test((
        indoc! {"\
            #[|lo]#rem
            ipsum
            dolor
            "},
        "CC",
        indoc! {"\
            #(|lo)#rem
            #(|ip)#sum
            #[|do]#lor
            "},
    ))
    .await?;

    // Copy the selection to previous line, skipping the first line in the file
    test((
        indoc! {"\
            test
            #[testitem|]#
            "},
        "<A-C>",
        indoc! {"\
            test
            #[testitem|]#
            "},
    ))
    .await?;

    // Copy the selection to previous line, including the first line in the file
    test((
        indoc! {"\
            test
            #[test|]#
            "},
        "<A-C>",
        indoc! {"\
            #[test|]#
            #(test|)#
            "},
    ))
    .await?;

    // Copy the selection to next line, skipping the last line in the file
    test((
        indoc! {"\
            #[testitem|]#
            test
            "},
        "C",
        indoc! {"\
            #[testitem|]#
            test
            "},
    ))
    .await?;

    // Copy the selection to next line, including the last line in the file
    test((
        indoc! {"\
            #[test|]#
            test
            "},
        "C",
        indoc! {"\
            #(test|)#
            #[test|]#
            "},
    ))
    .await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "Stage 4: goto-file adjacent-path discovery requires the D8 object-affinity migration"]
async fn test_goto_file_impl() -> anyhow::Result<()> {
    let file = tempfile::NamedTempFile::new()?;

    fn match_paths(app: &Application, matches: Vec<&str>) -> usize {
        app.editor
            .documents()
            .filter_map(|d| d.path()?.file_name())
            .filter(|n| matches.iter().any(|m| *m == n.to_string_lossy()))
            .count()
    }

    // Single selection
    test_key_sequence(
        &mut AppBuilder::new().with_file(file.path(), None).build()?,
        Some("ione.js<esc>%gf"),
        Some(&|app| {
            assert_eq!(1, match_paths(app, vec!["one.js"]));
        }),
        false,
    )
    .await?;

    // Multiple selection
    test_key_sequence(
        &mut AppBuilder::new().with_file(file.path(), None).build()?,
        Some("ione.js<ret>two.js<esc>%<A-s>gf"),
        Some(&|app| {
            assert_eq!(2, match_paths(app, vec!["one.js", "two.js"]));
        }),
        false,
    )
    .await?;

    // Cursor on first quote
    test_key_sequence(
        &mut AppBuilder::new().with_file(file.path(), None).build()?,
        Some("iimport 'one.js'<esc>B;gf"),
        Some(&|app| {
            assert_eq!(1, match_paths(app, vec!["one.js"]));
        }),
        false,
    )
    .await?;

    // Cursor on last quote
    test_key_sequence(
        &mut AppBuilder::new().with_file(file.path(), None).build()?,
        Some("iimport 'one.js'<esc>bgf"),
        Some(&|app| {
            assert_eq!(1, match_paths(app, vec!["one.js"]));
        }),
        false,
    )
    .await?;

    // ';' is behind the path
    test_key_sequence(
        &mut AppBuilder::new().with_file(file.path(), None).build()?,
        Some("iimport 'one.js';<esc>B;gf"),
        Some(&|app| {
            assert_eq!(1, match_paths(app, vec!["one.js"]));
        }),
        false,
    )
    .await?;

    // allow numeric values in path
    test_key_sequence(
        &mut AppBuilder::new().with_file(file.path(), None).build()?,
        Some("iimport 'one123.js'<esc>B;gf"),
        Some(&|app| {
            assert_eq!(1, match_paths(app, vec!["one123.js"]));
        }),
        false,
    )
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_multi_selection_paste() -> anyhow::Result<()> {
    test((
        indoc! {"\
            #[|lorem]#
            #(|ipsum)#
            #(|dolor)#
            "},
        "yp",
        indoc! {"\
            #[|lorem]#lorem
            #(|ipsum)#ipsum
            #(|dolor)#dolor
            "},
    ))
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn point_character_operators_use_the_right_grapheme() -> anyhow::Result<()> {
    test(("#[|]#a\u{301}b\n", "d", "#[|]#b\n")).await?;
    test(("#[|]#ab\n", "rX", "#[|]#Xb\n")).await?;
    test(("#[|]#ab\n", "~", "#[|]#Ab\n")).await?;
    test(("#[|]#🦀b\n", "yp", "#[🦀|]#🦀b\n")).await?;
    test(("ab#[|]#", "d", "ab#[|]#", LineFeedHandling::AsIs)).await?;
    test(("#[|]#ab#(|)#", "d", "#[|]#b#(|)#", LineFeedHandling::AsIs)).await?;
    test(("#[|]#ab#(|)#", "cX<esc>", "X#[|]#b", LineFeedHandling::AsIs)).await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn sort_at_eof_is_a_noop() -> anyhow::Result<()> {
    test_key_sequence_with_input_text(
        None,
        TestCase::from(("abc#[|]#", ":sort<ret>", "abc#[|]#", LineFeedHandling::AsIs)),
        &|app| assert_status_not_error(&app.editor),
        false,
    )
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn point_character_operator_matrix() -> anyhow::Result<()> {
    let cases = [
        ("#[|]#ab", "yp", "#[a|]#ab"),
        ("#[|]#ab", "yP", "#[a|]#ab"),
        ("#[|]#ab", "y<space>?paste_before<ret>", "#[a|]#ab"),
        ("#[|]#1x", "<C-a>", "#[2|]#x"),
        ("#[|]#1x", "<C-x>", "#[0|]#x"),
        ("#[|]#ab", "<space>?switch_to_uppercase<ret>", "#[|]#Ab"),
        ("#[|]#Ab", "<space>?switch_to_lowercase<ret>", "#[|]#ab"),
        ("#[|]#ab", "|tr a-z A-Z<ret>", "#[A|]#b"),
        ("#[|]#ab", "!printf X<ret>", "#[X|]#ab"),
        ("#[|]#ab", "<A-!>printf X<ret>", "#[X|]#ab"),
        ("#[|]#ab", "yR", "#[|]#ab"),
        ("#[|]# ab", "_", "#[|]# ab"),
        ("#[|]#ab", "_", "#[a|]#b"),
        ("#[|]#ab", "s.<ret>", "#[a|]#b"),
        ("#[|]#ab", "Sx<ret>", "#[a|]#b"),
        ("#[|]#ab", ":reflow<ret>", "#[|]#ab"),
    ];

    for case in cases {
        test((case.0, case.1, case.2, LineFeedHandling::AsIs)).await?;
    }

    for keys in ["y"] {
        test(("#[|]#a", keys, "#[|]#a", LineFeedHandling::AsIs)).await?;
        test(("a#[|]#", keys, "a#[|]#", LineFeedHandling::AsIs)).await?;
    }
    test((
        "#[|]#a",
        "<space>?yank_joined<ret>p",
        "#[a|]#a",
        LineFeedHandling::AsIs,
    ))
    .await?;
    test((
        "#[|]#xa",
        "yA<esc><space>?yank_joined<ret>p",
        "xa#[x|]#",
        LineFeedHandling::AsIs,
    ))
    .await?;

    for keys in ["yp", "yP"] {
        test(("#[ab|]#", keys, "ab#[ab|]#", LineFeedHandling::AsIs)).await?;
        test(("#[|ab]#", keys, "#[|ab]#ab", LineFeedHandling::AsIs)).await?;
    }
    for keys in ["xyp", "xyP"] {
        test((
            "#[|]#one\ntwo",
            keys,
            "one\n#[one\n|]#two",
            LineFeedHandling::AsIs,
        ))
        .await?;
    }
    test(("#[|]#ab", "y2p", "#[aa|]#ab", LineFeedHandling::AsIs)).await?;
    test(("#[|]#ab", "\"ay\"ap", "#[a|]#ab", LineFeedHandling::AsIs)).await?;
    test((
        "#[|]#a\n#(|)#b\n#(|)#",
        "Ka<ret>",
        "#[a|]#\nb\n#(|)#",
        LineFeedHandling::AsIs,
    ))
    .await?;
    test((
        "#[|]#a\n#(|)#b\n#(|)#",
        "<A-K>a<ret>",
        "a\n#[b|]#\n#(|)#",
        LineFeedHandling::AsIs,
    ))
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn clipboard_paste_variants_share_the_head_boundary() -> anyhow::Result<()> {
    let editor: helix_view::editor::Config = toml::from_str(
        r#"
        [clipboard-provider.custom]
        yank = { command = "printf", args = ["z"] }
        paste = { command = "cat" }
        yank-primary = { command = "printf", args = ["p"] }
        paste-primary = { command = "cat" }
        "#,
    )?;
    let mut config = test_config();
    config.editor.clipboard_provider = editor.clipboard_provider;
    let static_primary_keys: helix_term::config::ConfigRaw = toml::from_str(
        r#"
        [keys.normal]
        "C-p" = "paste_primary_clipboard_after"
        "C-o" = "paste_primary_clipboard_before"
        "C-r" = "replace_selections_with_primary_clipboard"
        "#,
    )?;
    config.keys = static_primary_keys.keys.unwrap();
    for keys in [
        "<space>p",
        "<space>P",
        ":clipboard-paste-after<ret>",
        ":clipboard-paste-before<ret>",
    ] {
        test_with_config(
            AppBuilder::new().with_config(config.clone()),
            ("#[|]#ab", keys, "#[z|]#ab", LineFeedHandling::AsIs),
        )
        .await?;
    }
    for keys in [
        ":primary-clipboard-paste-after<ret>",
        ":primary-clipboard-paste-before<ret>",
        "<C-p>",
        "<C-o>",
    ] {
        test_with_config(
            AppBuilder::new().with_config(config.clone()),
            ("#[|]#ab", keys, "#[p|]#ab", LineFeedHandling::AsIs),
        )
        .await?;
    }
    for keys in ["<space>R", ":clipboard-paste-replace<ret>"] {
        test_with_config(
            AppBuilder::new().with_config(config.clone()),
            ("#[|]#ab", keys, "#[|]#zb", LineFeedHandling::AsIs),
        )
        .await?;
    }
    for keys in [":primary-clipboard-paste-replace<ret>", "<C-r>"] {
        test_with_config(
            AppBuilder::new().with_config(config.clone()),
            ("#[|]#ab", keys, "#[|]#pb", LineFeedHandling::AsIs),
        )
        .await?;
    }

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn bracketed_and_middle_click_paste_use_exact_event_boundaries() -> anyhow::Result<()> {
    use helix_view::document::Mode;

    for (mode, input, expected) in [
        (Mode::Normal, "#[|]#ab", "#[z|]#ab"),
        (Mode::Select, "#[|ab]#", "#[|z]#ab"),
        (Mode::Insert, "#[|]#ab", "z#[|]#ab"),
    ] {
        let mut app = AppBuilder::new().with_input_text(input).build()?;
        app.editor.mode = mode;
        let (expected_text, expected_selection) = helix_core::test::print(expected);
        let check = |app: &Application| {
            let doc = helix_view::doc!(app.editor);
            assert_eq!(expected_text, doc.text().to_string());
            assert_eq!(expected_selection, *doc.selection(app.editor.tree.focus));
            let expected_mode = if mode == Mode::Select {
                Mode::Normal
            } else {
                mode
            };
            assert_eq!(expected_mode, app.editor.mode());
        };
        test_event_sequences(
            &mut app,
            vec![(vec![paste_event("z")], Some(&check))],
            false,
        )
        .await?;
    }

    let editor: helix_view::editor::Config = toml::from_str(
        r#"
        [clipboard-provider.custom]
        yank = { command = "printf", args = ["z"] }
        paste = { command = "cat" }
        yank-primary = { command = "printf", args = ["p"] }
        paste-primary = { command = "cat" }
        "#,
    )?;
    let mut config = test_config();
    config.editor.clipboard_provider = editor.clipboard_provider;
    config.editor.mouse_yank_register = '*';
    let mut app = AppBuilder::new()
        .with_config(config)
        .with_input_text("#[|]#ab")
        .build()?;
    let (row, column) = {
        let view = app.editor.tree.get(app.editor.tree.focus);
        let doc = app.editor.documents.get(&view.doc).unwrap();
        let area = view.inner_area(doc);
        (area.y, area.x)
    };
    test_event_sequences(
        &mut app,
        vec![(
            vec![middle_click_event(row, column)],
            Some(&|app| {
                let doc = helix_view::doc!(app.editor);
                assert_eq!("pab", doc.text());
                assert_eq!(
                    Selection::new(smallvec::smallvec![Range::new(0, 1)], 0),
                    *doc.selection(app.editor.tree.focus)
                );
            }),
        )],
        false,
    )
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn test_multi_selection_shell_commands() -> anyhow::Result<()> {
    // pipe
    test((
        indoc! {"\
            #[|lorem]#
            #(|ipsum)#
            #(|dolor)#
            "},
        "|echo foo<ret>",
        indoc! {"\
            #[|foo]#
            #(|foo)#
            #(|foo)#"
        },
    ))
    .await?;

    // insert-output
    test((
        indoc! {"\
            #[|lorem]#
            #(|ipsum)#
            #(|dolor)#
            "},
        "!echo foo<ret>",
        indoc! {"\
            #[|foo]#lorem
            #(|foo)#ipsum
            #(|foo)#dolor
            "},
    ))
    .await?;

    // append-output
    test((
        indoc! {"\
            #[|lorem]#
            #(|ipsum)#
            #(|dolor)#
            "},
        "<A-!>echo foo<ret>",
        indoc! {"\
            lorem#[|foo]#
            ipsum#(|foo)#
            dolor#(|foo)#
            "},
    ))
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_undo_redo() -> anyhow::Result<()> {
    // A jumplist selection is created at a point which is undone.
    //
    // * 2[<space>   Add two newlines at line start. We're now on line 3.
    // * <C-s>       Save the selection on line 3 in the jumplist.
    // * u           Undo the two newlines. We're now on line 1.
    // * <C-o><C-i>  Jump forward an back again in the jumplist. This would panic
    //               if the jumplist were not being updated correctly.
    test((
        "#[|]#",
        "2[<space><C-s>u<C-o><C-i>",
        "#[|]#",
        LineFeedHandling::AsIs,
    ))
    .await?;

    // A jumplist selection is passed through an edit and then an undo and then a redo.
    //
    // * [<space>    Add a newline at line start. We're now on line 2.
    // * <C-s>       Save the selection on line 2 in the jumplist.
    // * kd          Delete line 1. The jumplist selection should be adjusted to the new line 1.
    // * uU          Undo and redo the `kd` edit.
    // * <C-o>       Jump back in the jumplist. This would panic if the jumplist were not being
    //               updated correctly.
    // * <C-i>       Jump forward to line 1.
    test((
        "#[|]#",
        "[<space><C-s>kduU<C-o><C-i>",
        "#[|]#",
        LineFeedHandling::AsIs,
    ))
    .await?;

    // In this case we 'redo' manually to ensure that the transactions are composing correctly.
    test((
        "#[|]#",
        "[<space>u[<space>u",
        "#[|]#",
        LineFeedHandling::AsIs,
    ))
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_extend_line() -> anyhow::Result<()> {
    // extend with line selected then count
    test((
        indoc! {"\
            #[l|]#orem
            ipsum
            dolor
            
            "},
        "x2x",
        indoc! {"\
            #[lorem
            ipsum
            dolor\n|]#
            
            "},
    ))
    .await?;

    // extend with count on partial selection
    test((
        indoc! {"\
            #[l|]#orem
            ipsum
            
            "},
        "2x",
        indoc! {"\
            #[lorem
            ipsum\n|]#
            
            "},
    ))
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_character_info() -> anyhow::Result<()> {
    // UTF-8, single byte
    test_key_sequence(
        &mut helpers::AppBuilder::new().build()?,
        Some("ih<esc>h:char<ret>"),
        Some(&|app| {
            assert_eq!(
                r#""h" (U+0068) Dec 104 Hex 68"#,
                app.editor.get_status().unwrap().0
            );
        }),
        false,
    )
    .await?;

    // UTF-8, multi-byte
    test_key_sequence(
        &mut helpers::AppBuilder::new().build()?,
        Some("ië<esc>h:char<ret>"),
        Some(&|app| {
            assert_eq!(
                r#""ë" (U+0065 U+0308) Hex 65 + cc 88"#,
                app.editor.get_status().unwrap().0
            );
        }),
        false,
    )
    .await?;

    // Multiple characters displayed as one, escaped characters
    test_key_sequence(
        &mut helpers::AppBuilder::new().build()?,
        Some(":line<minus>ending crlf<ret>:char<ret>"),
        Some(&|app| {
            assert_eq!(
                r#""\r\n" (U+000d U+000a) Hex 0d + 0a"#,
                app.editor.get_status().unwrap().0
            );
        }),
        false,
    )
    .await?;

    // Non-UTF-8
    test_key_sequence(
        &mut helpers::AppBuilder::new().build()?,
        Some(":encoding ascii<ret>ih<esc>h:char<ret>"),
        Some(&|app| {
            assert_eq!(r#""h" Dec 104 Hex 68"#, app.editor.get_status().unwrap().0);
        }),
        false,
    )
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_delete_char_backward() -> anyhow::Result<()> {
    // don't panic when deleting overlapping ranges
    test(("#(x|)# #[x|]#", "c<space><backspace><esc>", "#[|]#")).await?;
    test((
        "#( |)##( |)#a#( |)#axx#[x|]#a",
        "li<backspace><esc>",
        "#(|)# #(|)#xxx#[|]#",
    ))
    .await?;

    Ok(())
}

// Cursor behavior is different when the text is created in the buffer vs loaded from a file.
// This test will not work for reproducing the crash or verifying the result after the fix.
// // #[tokio::test(flavor = "multi_thread")]
// async fn test_try_restore_indent() -> anyhow::Result<()> {
//     test((" #[ |]#foo\na#( |)#bar\n", "o<C-u><esc>", " foo\n#[\n|]#a bar\n#(\n|)#")).await?;
//     Ok(())
// }

#[tokio::test(flavor = "multi_thread")]
async fn test_try_restore_indent() -> anyhow::Result<()> {
    // Bug: 15228 try_restore_indent uses primary cursor position for all selections,
    // causing invalid range errors when multiple cursors are on different lines
    let file = temp_file_with_contents("  foo\na bar\n")?;
    test_key_sequence(
        &mut AppBuilder::new().with_file(file.path(), None).build()?,
        Some("jl<A-C>o<C-u><esc>"),
        None,
        false,
    )
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_delete_word_backward() -> anyhow::Result<()> {
    // don't panic when deleting overlapping ranges
    test(("fo#[o|]#ba#(r|)#", "a<C-w><esc>", "#[|]#r#(|)#")).await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_delete_word_forward() -> anyhow::Result<()> {
    // don't panic when deleting overlapping ranges
    test(("fo#[o|]#b#(|ar)#", "i<A-d><esc>", "foo#[|]#")).await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_delete_char_forward() -> anyhow::Result<()> {
    test((
        indoc! {"\
                #[abc|]#def
                #(abc|)#ef
                #(abc|)#f
                #(abc|)#
            "},
        "a<del><esc>",
        "abc#[|]#ef\nabc#(|)#f\nabc#(|)#\nabc#(|)#",
        LineFeedHandling::AsIs,
    ))
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_insert_with_indent() -> anyhow::Result<()> {
    const INPUT: &str = indoc! { "
        #[f|]#n foo() {
            if let Some(_) = None {

            }
         
        }

        fn bar() {

        }
        "
    };

    // insert_at_line_start
    test((
        INPUT,
        ":lang rust<ret>%<A-s>I",
        indoc! { "
            #[|]#fn foo() {
                #(|)#if let Some(_) = None {
                    #(|)#
                #(|)#}
            #(|)# 
            #(|)#}
            #(|)#\n
            #(|)#fn bar() {
                #(|)#
            #(|)#}
            "
        },
    ))
    .await?;

    // insert_at_line_end
    test((
        INPUT,
        ":lang rust<ret>%<A-s>A",
        indoc! { "
            fn foo() {#[|]#
                if let Some(_) = None {#(|)#
                    #(|)#
                }#(|)#
             #(|)#
            }#(|)#
            #(|)#\n
            fn bar() {#(|)#
                #(|)#
            }#(|)#
            "
        },
    ))
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_join_selections() -> anyhow::Result<()> {
    // normal join
    test((
        indoc! {"\
            #[a|]#bc
            def
        "},
        "J",
        indoc! {"\
            #[a|]#bc def
        "},
    ))
    .await?;

    // join without inserting spaces
    test((
        indoc! {"\
            #[a|]#bc
            def
        "},
        "+",
        indoc! {"\
            #[a|]#bcdef
        "},
    ))
    .await?;

    // join with empty line
    test((
        indoc! {"\
            #[a|]#bc

            def
        "},
        "JJ",
        indoc! {"\
            #[a|]#bc def
        "},
    ))
    .await?;

    // join with additional space in non-empty line
    test((
        indoc! {"\
            #[a|]#bc

                def
        "},
        "JJ",
        indoc! {"\
            #[a|]#bc def
        "},
    ))
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_join_selections_space() -> anyhow::Result<()> {
    // join with empty lines panic
    test((
        indoc! {"\
            #[a

            b

            c

            d

            e|]#
        "},
        "<A-J>",
        indoc! {"\
            a#[|]# b#(|)# c#(|)# d#(|)# e
        "},
    ))
    .await?;

    // normal join
    test((
        indoc! {"\
            #[a|]#bc
            def
        "},
        "<A-J>",
        indoc! {"\
            abc#[|]# def
        "},
    ))
    .await?;

    // join with empty line
    test((
        indoc! {"\
            #[a|]#bc

            def
        "},
        "<A-J>",
        indoc! {"\
            #[a|]#bc
            def
        "},
    ))
    .await?;

    // join with additional space in non-empty line
    test((
        indoc! {"\
            #[a|]#bc

                def
        "},
        "<A-J><A-J>",
        indoc! {"\
            abc#[|]# def
        "},
    ))
    .await?;

    // join with retained trailing spaces
    test((
        indoc! {"\
            #[aaa   

            bb  

            c |]#
        "},
        "<A-J>",
        indoc! {"\
            aaa   #[|]# bb  #(|)# c 
        "},
    ))
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_join_selections_comment() -> anyhow::Result<()> {
    test((
        indoc! {"\
            /// #[a|]#bc
            /// def
        "},
        ":lang rust<ret>J",
        indoc! {"\
            /// #[a|]#bc def
        "},
    ))
    .await?;

    // Only join if the comment token matches the previous line.
    test((
        indoc! {"\
            #[| // a
            // b
            /// c
            /// d
            e
            /// f
            // g]#
        "},
        ":lang rust<ret>J",
        indoc! {"\
            #[| // a b /// c d e f // g]#
        "},
    ))
    .await?;

    test((
        "#[|\t// Join comments
\t// with indent]#",
        ":lang go<ret>J",
        "#[|\t// Join comments with indent]#",
    ))
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_toggle_comments_inside_comment_injection() -> anyhow::Result<()> {
    // A `//` line comment's text is injected as the `comment` language, which has no
    // comment-tokens of its own. With the cursor inside the comment, toggling must
    // resolve tokens from the enclosing language and un-comment the line,
    // not fall back to the hardcoded default `#`.
    test((
        indoc! {"\
            // #[a|]#bc
        "},
        ":lang rust<ret><C-c>",
        indoc! {"\
            #[a|]#bc
        "},
    ))
    .await?;

    // A `///` doc comment's text is injected as markdown (no line comment token of
    // its own). Toggling must strip the whole `///` marker via Rust's tokens rather
    // than insert a markdown `<!-- -->` inside or leave a stray `/`.
    test((
        indoc! {"\
            /// #[a|]#bc
        "},
        ":lang rust<ret><C-c>",
        indoc! {"\
            #[a|]#bc
        "},
    ))
    .await?;

    // Likewise for the `//!` inner doc comment marker.
    test((
        indoc! {"\
            //! #[a|]#bc
        "},
        ":lang rust<ret><C-c>",
        indoc! {"\
            #[a|]#bc
        "},
    ))
    .await?;

    // Commenting a normal code line still uses the top-level language's token
    // (no injection layer at the cursor), no regression for the common case.
    test((
        indoc! {"\
            #[l|]#et x = 5;
        "},
        ":lang rust<ret><C-c>",
        indoc! {"\
            // #[l|]#et x = 5;
        "},
    ))
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_read_file() -> anyhow::Result<()> {
    let mut file = tempfile::NamedTempFile::new()?;
    let contents_to_read = "some contents";
    let output_file = helpers::temp_file_with_contents(contents_to_read)?;

    test_key_sequence(
        &mut helpers::AppBuilder::new()
            .with_file(file.path(), None)
            .build()?,
        Some(&format!(":r {:?}<ret><esc>:w<ret>", output_file.path())),
        Some(&|app| {
            assert!(!app.editor.is_err(), "error: {:?}", app.editor.get_status());
        }),
        false,
    )
    .await?;

    let expected_contents = LineFeedHandling::Native.apply(contents_to_read);
    helpers::assert_file_has_content(&mut file, &expected_contents)?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn surround_delete() -> anyhow::Result<()> {
    // Test `surround_delete` when head < anchor
    test(("(#[|  ]#)", "mdm", "#[|  ]#")).await?;
    test(("(#[|  ]#)", "md(", "#[|  ]#")).await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn surround_replace_ts() -> anyhow::Result<()> {
    const INPUT: &str = r#"\
fn foo() {
    if let Some(_) = None {
        testing!("f#[|o]#o)");
    }
}
"#;
    test((
        INPUT,
        ":lang rust<ret>mrm'",
        r#"\
fn foo() {
    if let Some(_) = None {
        testing!('f#[|o]#o)');
    }
}
"#,
    ))
    .await?;

    test((
        INPUT,
        ":lang rust<ret>3mrm[",
        r#"\
fn foo() {
    if let Some(_) = None [
        testing!("f#[|o]#o)");
    ]
}
"#,
    ))
    .await?;

    test((
        INPUT,
        ":lang rust<ret>2mrm{",
        r#"\
fn foo() {
    if let Some(_) = None {
        testing!{"f#[|o]#o)"};
    }
}
"#,
    ))
    .await?;

    test((
        indoc! {"\
            #[a
            b
            c
            d
            e|]#
            f
            "},
        "s\\n<ret>r,",
        "a#[,|]#b#(,|)#c#(,|)#d#(,|)#e\nf\n",
    ))
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn macro_play_within_macro_record() -> anyhow::Result<()> {
    // <https://github.com/helix-editor/helix/issues/12697>
    //
    // * `"aQihello<esc>Q` record a macro to register 'a' which inserts "hello"
    // * `Q"aq<space>world<esc>Q` record a macro to the default macro register which plays the
    //   macro in register 'a' and then inserts " world"
    // * `%d` clear the buffer
    // * `q` replay the macro in the default macro register
    // * `i<ret>` add a newline at the end
    //
    // The inner macro in register 'a' should replay within the outer macro exactly once to insert
    // "hello world".
    test((
        indoc! {"\
            #[|]#
        "},
        r#""aQihello<esc>QQ"aqi<space>world<esc>Q%dqi<ret>"#,
        indoc! {"\
            hello world
            #[|]#"},
    ))
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn global_search_with_multibyte_chars() -> anyhow::Result<()> {
    // Assert that `helix_term::commands::global_search` handles multibyte characters correctly.
    test((
        indoc! {"\
            // Hello world!
            // #[|
            ]#
            "},
        // start global search
        " /«十分に長い マルチバイトキャラクター列» で検索<ret><esc>",
        indoc! {"\
            // Hello world!
            // #[|
            ]#
            "},
    ))
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn align_selections_with_varying_columns() -> anyhow::Result<()> {
    test((
        indoc! {r"
            #[|]#I    I  II I
            IIIIIIIII
            IIIII
            IIIIIIIII
        "},
        r"%sI<ret>&gg",
        indoc! {r"
            #[|I    I  II I
            I    I  II IIIII
            I    I  II I
            I    I  II IIIII]#
        "},
    ))
    .await?;

    Ok(())
}
