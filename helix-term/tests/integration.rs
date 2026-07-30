#[cfg(feature = "integration")]
mod test {
    mod helpers;

    use helix_core::{
        indent::IndentStyle,
        snippets::{ActiveSnippet, Snippet, SnippetRenderCtx},
        syntax::config::AutoPairConfig,
        Range, Selection,
    };
    use helix_term::config::Config;

    use indoc::indoc;

    use self::helpers::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn hello_world() -> anyhow::Result<()> {
        test(("#[|]#", "ihello world<esc>", "hello world#[|]#")).await?;
        Ok(())
    }

    mod auto_pairs;
    mod command_line;
    mod commands;
    mod movement;
    mod splits;
}
