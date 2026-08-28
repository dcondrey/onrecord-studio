pub(crate) fn collapse_whitespace(value: &str) -> String {
    collapse_separators(value, char::is_whitespace)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScalarKind {
    Empty,
    Boolean,
    Integer,
    Decimal,
    CalendarDate,
    EmailAddress,
    WebUrl,
    Text,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NormalizedScalar {
    pub kind: ScalarKind,
    pub canonical: String,
}

/// Infers only scalar types with deterministic, locale-independent wire forms. Ambiguous numeric
/// dates and lossy floating-point conversions remain text so normalization never invents meaning.
pub(crate) fn infer_normalized_scalar(value: &str) -> NormalizedScalar {
    let collapsed = collapse_whitespace(value);
    if collapsed.is_empty() {
        return NormalizedScalar {
            kind: ScalarKind::Empty,
            canonical: String::new(),
        };
    }
    let folded = collapsed.to_ascii_lowercase();
    if matches!(folded.as_str(), "true" | "false") {
        return NormalizedScalar {
            kind: ScalarKind::Boolean,
            canonical: folded,
        };
    }
    if is_canonical_integer(&collapsed) {
        return NormalizedScalar {
            kind: ScalarKind::Integer,
            canonical: collapsed,
        };
    }
    if is_canonical_decimal(&collapsed) {
        return NormalizedScalar {
            kind: ScalarKind::Decimal,
            canonical: collapsed,
        };
    }
    if chrono::NaiveDate::parse_from_str(&collapsed, "%Y-%m-%d").is_ok() {
        return NormalizedScalar {
            kind: ScalarKind::CalendarDate,
            canonical: collapsed,
        };
    }
    if is_email_address(&collapsed) {
        return NormalizedScalar {
            kind: ScalarKind::EmailAddress,
            canonical: folded,
        };
    }
    if let Ok(mut url) = reqwest::Url::parse(&collapsed) {
        if matches!(url.scheme(), "http" | "https") && url.host_str().is_some() {
            url.set_fragment(None);
            return NormalizedScalar {
                kind: ScalarKind::WebUrl,
                canonical: url.to_string(),
            };
        }
    }
    NormalizedScalar {
        kind: ScalarKind::Text,
        canonical: collapsed,
    }
}

pub(crate) fn normalize_delimited_field(value: &str) -> String {
    let separated = collapse_separators(value, |character| {
        character.is_whitespace() || character.is_control() || character == ','
    });
    infer_normalized_scalar(&separated).canonical
}

fn is_canonical_integer(value: &str) -> bool {
    let digits = value
        .strip_prefix('-')
        .or_else(|| value.strip_prefix('+'))
        .unwrap_or(value);
    !digits.is_empty()
        && digits.bytes().all(|byte| byte.is_ascii_digit())
        && (digits == "0" || !digits.starts_with('0'))
}

fn is_canonical_decimal(value: &str) -> bool {
    let unsigned = value
        .strip_prefix('-')
        .or_else(|| value.strip_prefix('+'))
        .unwrap_or(value);
    let Some((whole, fraction)) = unsigned.split_once('.') else {
        return false;
    };
    !whole.is_empty()
        && !fraction.is_empty()
        && whole.bytes().all(|byte| byte.is_ascii_digit())
        && fraction.bytes().all(|byte| byte.is_ascii_digit())
        && (whole == "0" || !whole.starts_with('0'))
}

fn is_email_address(value: &str) -> bool {
    if value.len() > 320 || value.bytes().any(|byte| byte.is_ascii_whitespace()) {
        return false;
    }
    let Some((local, domain)) = value.rsplit_once('@') else {
        return false;
    };
    !local.is_empty()
        && local.len() <= 64
        && domain.contains('.')
        && domain.split('.').all(|label| {
            !label.is_empty()
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
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
    use super::{
        collapse_whitespace, infer_normalized_scalar, normalize_delimited_field,
        normalize_index_text, ScalarKind,
    };

    #[test]
    fn normalization_preserves_only_the_requested_punctuation() {
        assert_eq!(collapse_whitespace("  alpha,\tbeta  "), "alpha, beta");
        assert_eq!(normalize_delimited_field(" alpha,\tbeta\n"), "alpha beta");
        assert_eq!(normalize_index_text(" alpha,\0beta\n"), "alpha, beta");
    }

    #[test]
    fn scalar_inference_is_typed_and_refuses_ambiguous_values() {
        assert_eq!(infer_normalized_scalar(" TRUE ").kind, ScalarKind::Boolean);
        assert_eq!(
            infer_normalized_scalar("2026-08-28").kind,
            ScalarKind::CalendarDate
        );
        assert_eq!(
            infer_normalized_scalar("Analyst@Example.Gov").canonical,
            "analyst@example.gov"
        );
        assert_eq!(infer_normalized_scalar("08/09/10").kind, ScalarKind::Text);
        assert_eq!(infer_normalized_scalar("00123").kind, ScalarKind::Text);
    }
}
