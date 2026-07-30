use super::*;

const A: &str = indoc! {"
    #(a|)#
    #(b|)#
    #(c|)#
    #[d|]#
    #(e|)#"
};
const A_REV: &str = indoc! {"
    #(e|)#
    #[d|]#
    #(c|)#
    #(b|)#
    #(a|)#"
};
const B: &str = indoc! {"
    #(a|)#
    #(b|)#
    #[c|]#
    #(d|)#
    #(e|)#"
};
const B_REV: &str = indoc! {"
    #(e|)#
    #(d|)#
    #[c|]#
    #(b|)#
    #(a|)#"
};

const CMD: &str = "<space>?reverse_selection_contents<ret>";

#[tokio::test(flavor = "multi_thread")]
async fn reverse_selection_contents() -> anyhow::Result<()> {
    test((A, CMD, A_REV)).await?;
    test((B, CMD, B_REV)).await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn reverse_selection_contents_with_count() -> anyhow::Result<()> {
    test((B, format!("2{CMD}"), B)).await?;
    test((B, format!("3{CMD}"), B_REV)).await?;
    test((B, format!("4{CMD}"), B)).await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn reverse_selection_contents_preserves_primary_with_eof_point() -> anyhow::Result<()> {
    test((
        "#[a|]#\n#(b|)#\n#(c|)#\n#(|)#",
        CMD,
        "#(c|)#\n#(b|)#\n#[a|]#\n#(|)#",
        LineFeedHandling::AsIs,
    ))
    .await?;
    test((
        "#(a|)#\n#(b|)#\n#(c|)#\n#[|]#",
        CMD,
        "#(c|)#\n#(b|)#\n#(a|)#\n#[|]#",
        LineFeedHandling::AsIs,
    ))
    .await?;

    Ok(())
}
