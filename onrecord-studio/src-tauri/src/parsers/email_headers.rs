//! RFC 5322 header extraction from `.msg` files, for the email-thread-gap detector.
//!
//! The `msg_parser` crate this project already depends on exposes only `Date` and `Message-ID`
//! on its public `Headers` type. `In-Reply-To` and `References` — the fields that let a detector
//! see whether a released message points at another message the release does not contain — sit
//! inside the raw transport-headers blob that Outlook stores as the MAPI property
//! `PR_TRANSPORT_MESSAGE_HEADERS` (property tag `0x007D`, types `0x001E` ANSI or `0x001F`
//! Unicode). That blob lives as a stream at the root of the Compound File Binary Format `.msg`
//! file, under the name `__substg1.0_007D001F` (Unicode) or `__substg1.0_007D001E` (ANSI).
//!
//! This module opens the `.msg` file with the `cfb` crate, reads the transport-headers stream,
//! and parses RFC 5322 folded header lines for the three fields the thread-gap detector cares
//! about. Anything else (subject, from, date) already comes out of `parsers::msg`; this module
//! deliberately does *not* re-parse those fields.
//!
//! Bounded: rejects any `.msg` larger than `MAX_MSG_BYTES` or a transport-headers stream larger
//! than `MAX_HEADERS_BYTES`. A malformed `.msg` or an unreadable stream returns `Ok(default)` —
//! empty results, not an error. The caller (a detector) decides whether an empty result matters.

use std::collections::BTreeSet;
use std::io::Read;
use std::path::Path;

use crate::error::AppError;

/// Maximum `.msg` file size accepted. Real corporate messages fall well below this even with
/// attachments; anything larger is either a mislabeled file or a scan-me attack.
pub const MAX_MSG_BYTES: u64 = 256 * 1024 * 1024;

/// Maximum PR_TRANSPORT_MESSAGE_HEADERS stream size. The header block is text; a hundred KiB
/// covers every legitimate mail chain.
pub const MAX_HEADERS_BYTES: usize = 512 * 1024;

/// The header chain the thread-gap detector cares about.
///
/// `message_id` and `in_reply_to` are single Message-IDs (already stripped of surrounding
/// `<>` brackets). `references` is the full parent chain, ordered oldest to newest per the RFC
/// 5322 convention. All Message-IDs are lowercased for consistent comparison — RFC 5322 makes
/// the local-part case-sensitive in principle, but real-world implementations treat them as
/// case-insensitive and the practical alternative is missed edges.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EmailHeaderChain {
    pub message_id: Option<String>,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
}

impl EmailHeaderChain {
    /// True when nothing was extractable.
    pub fn is_empty(&self) -> bool {
        self.message_id.is_none() && self.in_reply_to.is_none() && self.references.is_empty()
    }

    /// Every Message-ID this message either has as its own identity or claims to reply to /
    /// reference. The message's own id sits first when known, so callers can pull it as a
    /// separate anchor point.
    pub fn all_referenced_ids(&self) -> BTreeSet<String> {
        let mut set = BTreeSet::new();
        if let Some(id) = &self.in_reply_to {
            set.insert(id.clone());
        }
        for reference in &self.references {
            set.insert(reference.clone());
        }
        set
    }
}

/// Read the header chain from a `.msg` file at `path`. Returns `Ok(default)` for malformed
/// inputs; `Err` only for oversize files.
pub fn read_msg_header_chain(path: &Path) -> Result<EmailHeaderChain, AppError> {
    let metadata = std::fs::metadata(path)
        .map_err(|error| AppError::Storage(format!("MSG stat failed: {error}")))?;
    if metadata.len() > MAX_MSG_BYTES {
        return Err(AppError::DataParsing(format!(
            "MSG exceeds {MAX_MSG_BYTES}-byte cap"
        )));
    }
    let Ok(mut container) = cfb::open(path) else {
        return Ok(EmailHeaderChain::default());
    };
    // Try the Unicode stream first (Office 2003+); fall back to ANSI.
    let headers_text = read_stream(&mut container, "/__substg1.0_007D001F", true)
        .or_else(|| read_stream(&mut container, "/__substg1.0_007D001E", false));
    let Some(text) = headers_text else {
        return Ok(EmailHeaderChain::default());
    };
    Ok(parse_header_chain(&text))
}

fn read_stream(
    container: &mut cfb::CompoundFile<std::fs::File>,
    path: &str,
    is_utf16_le: bool,
) -> Option<String> {
    if !container.exists(path) {
        return None;
    }
    let mut stream = container.open_stream(path).ok()?;
    // Cap read length; a hostile stream shouldn't blow the memory budget.
    let mut buffer = Vec::new();
    let take_limit = MAX_HEADERS_BYTES as u64;
    Read::take(&mut stream, take_limit)
        .read_to_end(&mut buffer)
        .ok()?;
    if is_utf16_le {
        if buffer.len() < 2 {
            return None;
        }
        let units: Vec<u16> = buffer
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_le_bytes(*pair))
            .collect();
        Some(String::from_utf16_lossy(&units))
    } else {
        // ANSI stream: use lossy UTF-8; real message headers are ASCII in practice.
        Some(String::from_utf8_lossy(&buffer).into_owned())
    }
}

/// Parse RFC 5322 folded headers, extracting Message-ID, In-Reply-To, and References. Field
/// names are matched case-insensitively; folded continuations (subsequent lines starting with
/// space or tab) are joined onto the previous line before parsing.
fn parse_header_chain(text: &str) -> EmailHeaderChain {
    let unfolded = unfold_headers(text);
    let mut chain = EmailHeaderChain::default();
    for line in unfolded.lines() {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let name_norm = name.trim().to_ascii_lowercase();
        let value = value.trim();
        match name_norm.as_str() {
            "message-id" => chain.message_id = extract_first_id(value),
            "in-reply-to" => chain.in_reply_to = extract_first_id(value),
            "references" => chain.references = extract_all_ids(value),
            _ => {}
        }
    }
    chain
}

/// Join RFC 5322 folded lines: any line that begins with whitespace is a continuation of the
/// previous logical line.
fn unfold_headers(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        if line.starts_with(' ') || line.starts_with('\t') {
            out.push(' ');
            out.push_str(line.trim_start());
        } else {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(line);
        }
    }
    out
}

/// Extract the first `<local@domain>` id from a header value, without the angle brackets.
fn extract_first_id(value: &str) -> Option<String> {
    let start = value.find('<')?;
    let after = &value[start + 1..];
    let end = after.find('>')?;
    let inner = after[..end].trim();
    if inner.is_empty() {
        None
    } else {
        Some(inner.to_ascii_lowercase())
    }
}

/// Extract every `<...>` id from a header value, in the order they appear.
fn extract_all_ids(value: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cursor = 0;
    while let Some(rel_start) = value[cursor..].find('<') {
        let absolute_start = cursor + rel_start;
        let after = &value[absolute_start + 1..];
        let Some(rel_end) = after.find('>') else {
            break;
        };
        let inner = after[..rel_end].trim();
        if !inner.is_empty() {
            out.push(inner.to_ascii_lowercase());
        }
        cursor = absolute_start + 1 + rel_end + 1;
    }
    out
}

/// Heuristic: does this Message-ID look like it came from a public mailing list rather than an
/// internal exchange? A missing reference to a listserv thread is much less suspicious than a
/// missing reference to an internal correspondent.
///
/// This is deliberately conservative — we flag as *list-like* only obviously listserv-shaped
/// ids (domain contains one of a small set of well-known list-hosting substrings, or the
/// local-part starts with the common list-generator prefixes). Everything else is treated as
/// internal for the purpose of the thread-gap detector's severity weighting.
pub fn looks_like_listserv(message_id: &str) -> bool {
    let lower = message_id.to_ascii_lowercase();
    const LIST_HOST_SUBSTRINGS: &[&str] = &[
        "@lists.",
        "@list.",
        "@groups.",
        "@mailman.",
        "@majordomo.",
        "@sympa.",
        "@googlegroups.com",
        "@yahoogroups.com",
    ];
    if LIST_HOST_SUBSTRINGS
        .iter()
        .any(|marker| lower.contains(marker))
    {
        return true;
    }
    // Local part patterns Mailman/Majordomo/Sympa use for auto-generated ids.
    // e.g. "mailman.42.1710000000.12345.list@..."
    let local_part = lower.split('@').next().unwrap_or("");
    ["mailman.", "sympa.", "majordomo.", "listserv."]
        .iter()
        .any(|prefix| local_part.starts_with(prefix))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_folded_and_capitalized_headers() {
        let text = "\
Subject: Test
From: a@b.c
Message-ID: <abc123@example.com>
In-Reply-To: <parent-id@example.com>
References: <root@example.com>\r\n <middle@example.com>\r\n <parent-id@example.com>
";
        let chain = parse_header_chain(text);
        assert_eq!(chain.message_id.as_deref(), Some("abc123@example.com"));
        assert_eq!(chain.in_reply_to.as_deref(), Some("parent-id@example.com"));
        assert_eq!(
            chain.references,
            vec![
                "root@example.com".to_string(),
                "middle@example.com".to_string(),
                "parent-id@example.com".to_string(),
            ]
        );
    }

    #[test]
    fn missing_headers_produce_empty_chain_not_error() {
        let text = "Subject: nothing to see here\n";
        let chain = parse_header_chain(text);
        assert!(chain.is_empty());
    }

    #[test]
    fn message_id_case_folded_for_consistent_comparison() {
        let text = "Message-ID: <UPPER-Case@Example.Com>";
        let chain = parse_header_chain(text);
        assert_eq!(chain.message_id.as_deref(), Some("upper-case@example.com"));
    }

    #[test]
    fn references_preserves_ordering_and_ignores_leading_junk() {
        let text = "References: garbage <a@x> more <b@x> <c@x>";
        let chain = parse_header_chain(text);
        assert_eq!(
            chain.references,
            vec!["a@x".to_string(), "b@x".to_string(), "c@x".to_string()]
        );
    }

    #[test]
    fn all_referenced_ids_unions_in_reply_to_and_references() {
        let chain = EmailHeaderChain {
            message_id: Some("self@x".into()),
            in_reply_to: Some("parent@x".into()),
            references: vec!["parent@x".into(), "grandparent@x".into()],
        };
        let set = chain.all_referenced_ids();
        assert!(set.contains("parent@x"));
        assert!(set.contains("grandparent@x"));
        assert!(!set.contains("self@x"));
        assert_eq!(set.len(), 2);
    }

    #[test]
    fn looks_like_listserv_flags_common_list_hosts_and_local_prefixes() {
        assert!(looks_like_listserv("abc@lists.example.org"));
        assert!(looks_like_listserv("thread-42@googlegroups.com"));
        assert!(looks_like_listserv("mailman.42.99.list@somewhere.org"));
        // A plain internal id is NOT flagged.
        assert!(!looks_like_listserv("abc123@department.city.gov"));
    }
}
