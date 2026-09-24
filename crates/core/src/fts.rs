//! FTS5 query construction (pure).

/// Build a safe FTS5 MATCH query from free-text user input.
///
/// Rules:
/// - Returns `None` when there are no usable terms (empty / punctuation only).
/// - Non-alphanumeric separators split terms; double quotes and FTS operators
///   (`*`, `-`, `(`, `)`, `:`, `^`) can never reach SQLite.
/// - Every term becomes a prefix search (`term*`) so "search as you type" works.
/// - Terms are joined with a space (implicit AND).
pub fn build_fts_query(input: &str) -> Option<String> {
    let terms: Vec<String> = input
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .filter(|t| !t.is_empty())
        .map(|t| format!("{}*", t.to_lowercase()))
        .collect();

    if terms.is_empty() {
        None
    } else {
        Some(terms.join(" "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_and_blank_are_none() {
        assert_eq!(build_fts_query(""), None);
        assert_eq!(build_fts_query("   "), None);
    }

    #[test]
    fn single_word_becomes_prefix() {
        assert_eq!(build_fts_query("hello"), Some("hello*".into()));
    }

    #[test]
    fn multiple_words_implicit_and() {
        assert_eq!(build_fts_query("pink floyd"), Some("pink* floyd*".into()));
    }

    #[test]
    fn quotes_and_fts_operators_are_stripped() {
        assert_eq!(build_fts_query(r#"say "you""#), Some("say* you*".into()));
        assert_eq!(build_fts_query("c++"), Some("c*".into()));
        assert_eq!(
            build_fts_query("pre-*mix -(live)"),
            Some("pre* mix* live*".into())
        );
    }

    #[test]
    fn keeps_unicode_letters_and_digits() {
        assert_eq!(build_fts_query("CAFÉ"), Some("café*".into()));
        assert_eq!(build_fts_query("2023"), Some("2023*".into()));
        assert_eq!(build_fts_query("日本語"), Some("日本語*".into()));
    }
}
