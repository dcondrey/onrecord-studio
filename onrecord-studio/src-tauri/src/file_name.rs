use std::path::Path;

pub(crate) fn sanitized_component(value: &str, maximum_bytes: usize) -> String {
    let basename = Path::new(value)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    let mut sanitized = String::with_capacity(basename.len().min(maximum_bytes));
    for character in basename.chars().filter_map(|character| {
        if character.is_control() {
            None
        } else if matches!(character, '/' | '\\' | ':' | '\0') {
            Some('_')
        } else {
            Some(character)
        }
    }) {
        if sanitized.len() + character.len_utf8() > maximum_bytes {
            break;
        }
        sanitized.push(character);
    }
    sanitized.trim().trim_matches('.').to_string()
}

#[cfg(test)]
mod tests {
    use super::sanitized_component;

    #[test]
    fn strips_path_escape_characters_and_honors_utf8_byte_limits() {
        assert_eq!(
            sanitized_component("../../evidence:final.pdf", 180),
            "evidence_final.pdf"
        );
        assert_eq!(sanitized_component("...", 180), "");
        assert!(sanitized_component(&"é".repeat(200), 180).len() <= 180);
    }
}
