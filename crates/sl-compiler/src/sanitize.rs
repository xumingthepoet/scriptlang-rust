fn sanitize_rhai_source(source: &str) -> String {
    let without_line_comments = line_comment_regex().replace_all(source, " ");
    let without_block_comments = block_comment_regex().replace_all(&without_line_comments, " ");
    let without_double_quotes = double_quote_regex().replace_all(&without_block_comments, " ");
    single_quote_regex()
        .replace_all(&without_double_quotes, " ")
        .into_owned()
}

fn contains_root_identifier(source: &str, symbol: &str) -> bool {
    if symbol.is_empty() {
        return false;
    }

    let symbol_len = symbol.len();
    let mut offset = 0usize;
    while let Some(found) = source[offset..].find(symbol) {
        let start = offset + found;
        let end = start + symbol_len;
        let left = source[..start].chars().next_back();
        let right = source[end..].chars().next();
        if is_left_boundary(left) && is_right_boundary(right) {
            return true;
        }
        offset = end;
    }
    false
}

fn line_comment_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"//[^\n]*").expect("line comment regex"))
}

fn block_comment_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"(?s)/\*.*?\*/").expect("block comment regex"))
}

fn double_quote_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r#""(?:\\.|[^"\\])*""#).expect("double quote regex"))
}

fn single_quote_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r#"'(?:\\.|[^'\\])*'"#).expect("single quote regex"))
}

fn is_identifier_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '$' || ch == '_'
}

fn is_left_boundary(left: Option<char>) -> bool {
    match left {
        None => true,
        Some(ch) => !is_identifier_char(ch) && ch != '.',
    }
}

fn is_right_boundary(right: Option<char>) -> bool {
    match right {
        None => true,
        Some(ch) => !is_identifier_char(ch) && ch != ':',
    }
}

#[cfg(test)]
mod sanitize_tests {
    use super::*;

    #[test]
    fn sanitize_and_root_identifier_helpers_cover_boundary_cases() {
        let sanitized = sanitize_rhai_source(
            r#"
        let a = "x"; // comment
        let b = 'y';
        /* block */
        game.value
        "#,
        );
        assert!(sanitized.contains("game.value"));
        assert!(!sanitized.contains("comment"));
        assert!(!sanitized.contains("\"x\""));
        assert!(!sanitized.contains("'y'"));

        assert!(!contains_root_identifier("game", ""));
        assert!(contains_root_identifier("game", "game"));
        assert!(!contains_root_identifier("xgame", "game"));
        assert!(!contains_root_identifier("gamex", "game"));
        assert!(!contains_root_identifier("obj.game", "game"));
        assert!(!contains_root_identifier("game:value", "game"));
        assert!(contains_root_identifier(" game ", "game"));
    }
}
