pub(crate) fn collapse_whitespace(value: &str) -> String {
    collapse_separators(value, char::is_whitespace)
}

pub(crate) fn normalize_delimited_field(value: &str) -> String {
    // TODO(normalization): Add typed row/entity inference for heterogeneous CSV, JSON, and API
    // payloads; this helper currently normalizes boundaries without understanding field meaning.
    collapse_separators(value, |character| {
        character.is_whitespace() || character.is_control() || character == ','
    })
}

pub(crate) fn normalize_index_text(value: &str) -> String {
    collapse_separators(value, |character| {
        character.is_whitespace() || character.is_control()
    })
}

fn collapse_separators(value: &str, mut is_separator: impl FnMut(char) -> bool) -> String {
    let mut normalized = String::with_capacity(value.len());
    let mut separator_pending = false;
    for character in value.chars() {
        if is_separator(character) {
            separator_pending = !normalized.is_empty();
            continue;
        }
        if separator_pending {
            normalized.push(' ');
            separator_pending = false;
        }
        normalized.push(character);
    }
    normalized
}

#[cfg(test)]
mod tests {
    use super::{collapse_whitespace, normalize_delimited_field, normalize_index_text};

    #[test]
    fn normalization_preserves_only_the_requested_punctuation() {
        assert_eq!(collapse_whitespace("  alpha,\tbeta  "), "alpha, beta");
        assert_eq!(normalize_delimited_field(" alpha,\tbeta\n"), "alpha beta");
        assert_eq!(normalize_index_text(" alpha,\0beta\n"), "alpha, beta");
    }
}
