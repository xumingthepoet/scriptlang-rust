fn sanitize_rhai_source(source: &str) -> String {
    let line_comment_re = Regex::new(r"//[^\n]*").expect("line comment regex");
    let block_comment_re = Regex::new(r"(?s)/\*.*?\*/").expect("block comment regex");
    let double_quote_re = Regex::new(r#""(?:\\.|[^"\\])*""#).expect("double quote regex");
    let single_quote_re = Regex::new(r#"'(?:\\.|[^'\\])*'"#).expect("single quote regex");

    let without_line_comments = line_comment_re.replace_all(source, " ");
    let without_block_comments = block_comment_re.replace_all(&without_line_comments, " ");
    let without_double_quotes = double_quote_re.replace_all(&without_block_comments, " ");
    single_quote_re
        .replace_all(&without_double_quotes, " ")
        .into_owned()
}

fn contains_root_identifier(source: &str, symbol: &str) -> bool {
    let pattern = format!(
        r"(?m)(^|[^.$0-9A-Za-z_]){}([^:$0-9A-Za-z_]|$)",
        regex::escape(symbol)
    );
    Regex::new(&pattern)
        .expect("root identifier regex must compile")
        .is_match(source)
}

