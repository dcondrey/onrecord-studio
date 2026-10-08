//! Strong identifiers read from document text: US EIN, California contractor license (CSLB) and
//! assessor's parcel number (APN).
//!
//! None of these carries a checksum, so the strongest claim any of them earns is a structural one
//! (`FormatValidated`). A bare digit run is never an identifier: the EIN needs its dash and a
//! clean boundary, the license and the parcel need an explicit label. A candidate that
//! `pii::mask_pii` would mask in its surrounding text is dropped, so the index never stores a value
//! the app elsewhere removes.

use crate::storage::pii::mask_pii;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct IdentifierMatch {
    pub normalized: String,
    pub display: String,
    pub byte_start: usize,
    pub byte_end: usize,
}

impl IdentifierMatch {
    fn new(text: &str, byte_start: usize, byte_end: usize) -> Self {
        let value = text[byte_start..byte_end].to_string();
        Self {
            normalized: value.clone(),
            display: value,
            byte_start,
            byte_end,
        }
    }
}

/// Bytes of context handed to the PII detectors before a candidate; labels and separators that
/// decide whether a value is masked sit directly in front of it.
const PII_CONTEXT_BYTES: usize = 64;
/// How far from a license label the word "contractor" may sit, within the same line.
const CONTRACTOR_CONTEXT_BYTES: usize = 100;
/// Filler words allowed between a label and its value ("Contractor License No. 123456").
const MAXIMUM_LABEL_FILLERS: usize = 4;

/// True when PII masking would remove any part of `start..end`, or cannot run.
fn masked_by_pii(text: &str, start: usize, end: usize) -> bool {
    masked_by_pii_reaching(text, start, end, 8)
}

fn masked_by_pii_reaching(text: &str, start: usize, end: usize, trailing: usize) -> bool {
    let window_start = text.floor_char_boundary(start.saturating_sub(PII_CONTEXT_BYTES));
    let window_end = text.ceil_char_boundary((end + trailing).min(text.len()));
    mask_pii(&text[window_start..window_end]).map_or(true, |result| {
        result.findings.iter().any(|finding| {
            window_start + finding.byte_start < end && window_start + finding.byte_end > start
        })
    })
}

fn boundary_before(bytes: &[u8], start: usize) -> bool {
    start == 0 || !(bytes[start - 1].is_ascii_alphanumeric() || b"-/._".contains(&bytes[start - 1]))
}

fn boundary_after(bytes: &[u8], end: usize) -> bool {
    match bytes.get(end) {
        None => true,
        Some(next) if next.is_ascii_alphanumeric() || b"-/_".contains(next) => false,
        Some(b'.') => !bytes.get(end + 1).is_some_and(u8::is_ascii_digit),
        Some(_) => true,
    }
}

fn digits_at(bytes: &[u8], from: usize, count: usize) -> bool {
    bytes
        .get(from..from + count)
        .is_some_and(|slice| slice.iter().all(u8::is_ascii_digit))
}

/// IRS prefixes that are not assigned to any campus. Everything else passes structurally.
fn valid_ein_prefix(prefix: u8) -> bool {
    !matches!(
        prefix,
        0 | 7 | 8 | 9 | 17 | 18 | 19 | 28 | 29 | 49 | 69 | 70 | 78 | 79 | 89 | 96 | 97
    )
}

/// `NN-NNNNNNN` with an assigned prefix, on clean boundaries. The dash form is required; nine
/// digits with no dash are indistinguishable from an SSN, a phone number or an account number.
pub(super) fn find_eins(text: &str, limit: usize) -> Vec<IdentifierMatch> {
    let bytes = text.as_bytes();
    let mut found = Vec::new();
    let mut index = 0;
    while index + 10 <= bytes.len() && found.len() < limit {
        if digits_at(bytes, index, 2)
            && bytes[index + 2] == b'-'
            && digits_at(bytes, index + 3, 7)
            && boundary_before(bytes, index)
            && boundary_after(bytes, index + 10)
            && valid_ein_prefix((bytes[index] - b'0') * 10 + (bytes[index + 1] - b'0'))
            && !masked_by_pii(text, index, index + 10)
        {
            found.push(IdentifierMatch::new(text, index, index + 10));
            index += 10;
        } else {
            index += 1;
        }
    }
    found
}

fn is_word_boundary(bytes: &[u8], start: usize, end: usize) -> bool {
    (start == 0 || !bytes[start - 1].is_ascii_alphanumeric())
        && bytes
            .get(end)
            .is_none_or(|next| !next.is_ascii_alphanumeric())
}

/// Positions just past each whole-word occurrence of any label in `lower`, in text order.
fn label_ends(lower: &str, labels: &[&str]) -> Vec<usize> {
    let bytes = lower.as_bytes();
    let mut ends = Vec::new();
    for label in labels {
        let mut from = 0;
        while let Some(found) = lower[from..].find(label) {
            let start = from + found;
            let end = start + label.len();
            if is_word_boundary(bytes, start, end) {
                ends.push(end);
            }
            from = end;
        }
    }
    ends.sort_unstable();
    ends.dedup();
    ends
}

/// Skips separators and filler words after a label, returning where the value starts. A line
/// break ends the label: the value must sit on the label's own line.
fn skip_label_tail(lower: &str, mut index: usize, fillers: &[&str]) -> usize {
    let bytes = lower.as_bytes();
    let mut skipped = 0;
    loop {
        while bytes.get(index).is_some_and(|byte| {
            (byte.is_ascii_whitespace() && *byte != b'\n') || b":#.()".contains(byte)
        }) {
            index += 1;
        }
        let word_end = bytes[index..]
            .iter()
            .position(|byte| !(byte.is_ascii_alphabetic() || *byte == b'\''))
            .map_or(bytes.len(), |length| index + length);
        if word_end > index
            && skipped < MAXIMUM_LABEL_FILLERS
            && fillers.contains(&&lower[index..word_end])
        {
            index = word_end;
            skipped += 1;
        } else {
            return index;
        }
    }
}

fn digit_run_end(bytes: &[u8], start: usize) -> usize {
    bytes[start..]
        .iter()
        .position(|byte| !byte.is_ascii_digit())
        .map_or(bytes.len(), |length| start + length)
}

/// The line containing `around`, clipped to `reach` bytes either side.
fn same_line_window(lower: &str, around: usize, reach: usize) -> &str {
    let start = lower.floor_char_boundary(around.saturating_sub(reach));
    let end = lower.ceil_char_boundary((around + reach).min(lower.len()));
    let window = &lower[start..end];
    let anchor = around - start;
    let line_start = window[..anchor].rfind('\n').map_or(0, |at| at + 1);
    let line_end = window[anchor..]
        .find('\n')
        .map_or(window.len(), |at| anchor + at);
    &window[line_start..line_end]
}

/// A 6 to 8 digit license number introduced by an explicit label. `CSLB` and the contractor
/// phrasings qualify alone; the generic `Lic. No.` and `License No.` qualify only when the word
/// "contractor" sits on the same line within [`CONTRACTOR_CONTEXT_BYTES`]. A driver's license is
/// not a contractor's, and PII masking owns that label.
pub(super) fn find_contractor_licenses(text: &str, limit: usize) -> Vec<IdentifierMatch> {
    const EXPLICIT: &[&str] = &[
        "cslb",
        "contractor license",
        "contractor's license",
        "contractors license",
        "contractor lic",
        "contractor's lic",
    ];
    const NEEDS_CONTRACTOR: &[&str] = &["lic", "license", "licence"];
    const FILLERS: &[&str] = &[
        "license", "lic", "licence", "no", "number", "num", "id", "is",
    ];
    let lower = text.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let mut labels: Vec<(usize, bool)> = label_ends(&lower, EXPLICIT)
        .into_iter()
        .map(|end| (end, true))
        .chain(
            label_ends(&lower, NEEDS_CONTRACTOR)
                .into_iter()
                .map(|end| (end, false)),
        )
        .collect();
    labels.sort_unstable();
    let mut found: Vec<IdentifierMatch> = Vec::new();
    for (label_end, explicit) in labels {
        if found.len() >= limit {
            break;
        }
        if !explicit
            && !same_line_window(&lower, label_end, CONTRACTOR_CONTEXT_BYTES).contains("contractor")
        {
            continue;
        }
        let start = skip_label_tail(&lower, label_end, FILLERS);
        let end = digit_run_end(bytes, start);
        if !(6..=8).contains(&(end - start))
            || !boundary_after(bytes, end)
            || found.iter().any(|existing| existing.byte_start == start)
            || masked_by_pii(text, start, end)
        {
            continue;
        }
        found.push(IdentifierMatch::new(text, start, end));
    }
    found.sort_by_key(|found| found.byte_start);
    found
}

/// `NNN-NNN-NN` or `NNN-NNN-NN-NN` introduced by `APN` or an "Assessor's Parcel" phrase. A parcel
/// identifies a property; see `resolution::identifiers` for what it may and may not anchor.
pub(super) fn find_parcel_numbers(text: &str, limit: usize) -> Vec<IdentifierMatch> {
    const LABELS: &[&str] = &[
        "apn",
        "assessor's parcel",
        "assessors parcel",
        "assessor parcel",
    ];
    const FILLERS: &[&str] = &["apn", "no", "number", "num", "id", "of"];
    let lower = text.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let mut found: Vec<IdentifierMatch> = Vec::new();
    for label_end in label_ends(&lower, LABELS) {
        if found.len() >= limit {
            break;
        }
        let start = skip_label_tail(&lower, label_end, FILLERS);
        let shaped = digits_at(bytes, start, 3)
            && bytes.get(start + 3) == Some(&b'-')
            && digits_at(bytes, start + 4, 3)
            && bytes.get(start + 7) == Some(&b'-')
            && digits_at(bytes, start + 8, 2);
        if !shaped {
            continue;
        }
        let mut end = start + 10;
        if bytes.get(end) == Some(&b'-') && digits_at(bytes, end + 1, 2) {
            end += 3;
        }
        if !boundary_after(bytes, end)
            || found.iter().any(|existing| existing.byte_start == start)
            || masked_by_pii(text, start, end)
        {
            continue;
        }
        found.push(IdentifierMatch::new(text, start, end));
    }
    found.sort_by_key(|found| found.byte_start);
    found
}

/// Labels that introduce a numbered document series. "Contract" never matches "contractor": a
/// label is a whole word.
const SERIES_LABELS: &[&str] = &[
    "resolution",
    "ordinance",
    "contract",
    "agreement",
    "permit",
    "purchase order",
    "invoice",
    "bid",
    "rfp",
    "work order",
    "claim",
    "citation",
    "incident",
    "docket",
];
const SERIES_VALUE_BYTES: std::ops::RangeInclusive<usize> = 2..=24;
/// A series value can be just the area code of "(619) 555-0198", so the PII detectors must see
/// the whole phone number after it to recognize it.
const SERIES_PII_TRAILING_BYTES: usize = 16;

/// Where the required `no` / `number` / `#` marker after a series label ends. Without one,
/// "the resolution 2024" is prose, not a citation of a numbered document.
fn series_marker_end(lower: &str, mut index: usize) -> Option<usize> {
    let bytes = lower.as_bytes();
    while bytes
        .get(index)
        .is_some_and(|byte| matches!(byte, b' ' | b'\t' | b':'))
    {
        index += 1;
    }
    if bytes.get(index) == Some(&b'#') {
        return Some(index + 1);
    }
    ["number", "no"].into_iter().find_map(|marker| {
        let end = index + marker.len();
        (lower[index..].starts_with(marker)
            && bytes
                .get(end)
                .is_none_or(|next| !next.is_ascii_alphanumeric()))
        .then_some(end)
    })
}

/// `d/d/d` or `dddd-dd-dd`: a date written where a number was expected.
fn is_date_shaped(value: &str) -> bool {
    let slash_parts: Vec<&str> = value.split('/').collect();
    let all_digits =
        |part: &&str| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());
    (slash_parts.len() == 3 && slash_parts.iter().all(all_digits))
        || (value.len() == 10
            && digits_at(value.as_bytes(), 0, 4)
            && value.as_bytes()[4] == b'-'
            && digits_at(value.as_bytes(), 5, 2)
            && value.as_bytes()[7] == b'-'
            && digits_at(value.as_bytes(), 8, 2))
}

/// A labelled series number ("Resolution No. 2024-012"), normalized to `label:VALUE` so the gap
/// scan can group a series by label and prefix. The value must end in a digit and hold at least
/// two; a longer run is refused whole rather than truncated, and phone numbers are refused by the
/// PII check.
pub(super) fn find_series_numbers(text: &str, limit: usize) -> Vec<IdentifierMatch> {
    const FILLERS: &[&str] = &["no", "number"];
    let lower = text.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let mut labels: Vec<(usize, &str)> = SERIES_LABELS
        .iter()
        .flat_map(|label| {
            label_ends(&lower, &[*label])
                .into_iter()
                .map(move |end| (end, *label))
        })
        .collect();
    labels.sort_unstable();
    let mut found: Vec<IdentifierMatch> = Vec::new();
    for (label_end, label) in labels {
        if found.len() >= limit {
            break;
        }
        let Some(marker_end) = series_marker_end(&lower, label_end) else {
            continue;
        };
        let start = skip_label_tail(&lower, marker_end, FILLERS);
        let run_end = bytes[start..]
            .iter()
            .position(|byte| !(byte.is_ascii_alphanumeric() || b"-/.".contains(byte)))
            .map_or(bytes.len(), |length| start + length);
        let Some(end) = bytes[start..run_end]
            .iter()
            .rposition(u8::is_ascii_digit)
            .map(|last| start + last + 1)
        else {
            continue;
        };
        let value = &text[start..end];
        if !bytes[start].is_ascii_alphanumeric()
            || !SERIES_VALUE_BYTES.contains(&value.len())
            || value.bytes().filter(u8::is_ascii_digit).count() < 2
            || is_date_shaped(value)
            || !boundary_after(bytes, end)
            || found.iter().any(|existing| existing.byte_start == start)
            || masked_by_pii_reaching(text, start, end, SERIES_PII_TRAILING_BYTES)
        {
            continue;
        }
        let label_start = label_end - label.len();
        found.push(IdentifierMatch {
            normalized: format!("{label}:{}", value.to_ascii_uppercase()),
            display: format!("{} {value}", &text[label_start..label_end]),
            byte_start: start,
            byte_end: end,
        });
    }
    found.sort_by_key(|found| found.byte_start);
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values(matches: Vec<IdentifierMatch>) -> Vec<String> {
        matches.into_iter().map(|found| found.normalized).collect()
    }

    #[test]
    fn an_ein_needs_its_dash_an_assigned_prefix_and_clean_boundaries() {
        let cases: [(&str, Vec<&str>); 15] = [
            ("EIN 12-3456789 on file", vec!["12-3456789"]),
            ("(12-3456789)", vec!["12-3456789"]),
            ("12-3456789.", vec!["12-3456789"]),
            (
                "a 12-3456789 and 98-7654321",
                vec!["12-3456789", "98-7654321"],
            ),
            ("123456789", vec![]),
            ("112-3456789", vec![]),
            ("12-34567890", vec![]),
            ("12-345678", vec![]),
            ("12-3456789-1", vec![]),
            ("555-12-3456789", vec![]),
            ("00-1234567 and 07-1234567", vec![]),
            ("call 619-555-0198", vec![]),
            ("SSN 123-45-6789", vec![]),
            ("on 2024-03-1234567", vec![]),
            ("v12-3456789x", vec![]),
        ];
        for (text, expected) in cases {
            assert_eq!(values(find_eins(text, 100)), expected, "{text}");
        }
    }

    #[test]
    fn an_ein_the_pii_masker_removes_is_not_indexed() {
        let text = "Driver's license 12-3456789";
        assert!(!mask_pii(text).unwrap().findings.is_empty());
        assert!(find_eins(text, 100).is_empty());
    }

    #[test]
    fn extraction_stops_at_the_limit() {
        let text = "12-3456789 12-3456780 12-3456781";
        assert_eq!(find_eins(text, 2).len(), 2);
        assert_eq!(find_eins(text, 3).len(), 3);
    }

    #[test]
    fn a_contractor_license_needs_a_label_and_contractor_context() {
        let cases: [(&str, Vec<&str>); 14] = [
            ("CSLB 123456", vec!["123456"]),
            ("CSLB License No. 1234567", vec!["1234567"]),
            ("Contractor License #12345678.", vec!["12345678"]),
            ("Contractor's License Number: 123456", vec!["123456"]),
            ("The contractor, Lic. No. 123456, signed", vec!["123456"]),
            ("123456", vec![]),
            ("License No. 123456", vec![]),
            ("Lic. No. 123456 was issued to the driver", vec![]),
            ("The contractor signed.\nLic. No. 123456", vec![]),
            ("CSLB 12345", vec![]),
            ("CSLB 123456789", vec![]),
            ("CSLB 123456-78", vec![]),
            ("CSLB 619-555-0198", vec![]),
            ("CSLB 03/14/1990", vec![]),
        ];
        for (text, expected) in cases {
            assert_eq!(
                values(find_contractor_licenses(text, 100)),
                expected,
                "{text}"
            );
        }
    }

    #[test]
    fn a_drivers_license_is_never_a_contractor_license() {
        assert!(find_contractor_licenses("Driver's License 12345678", 100).is_empty());
    }

    #[test]
    fn a_parcel_number_needs_an_apn_label_and_a_known_shape() {
        let cases: [(&str, Vec<&str>); 12] = [
            ("APN 123-456-78", vec!["123-456-78"]),
            ("APN: 123-456-78-90.", vec!["123-456-78-90"]),
            (
                "Assessor's Parcel Number (APN): 123-456-78",
                vec!["123-456-78"],
            ),
            ("assessors parcel no. 123-456-78", vec!["123-456-78"]),
            ("123-456-78", vec![]),
            ("APN 123-45-6789", vec![]),
            ("APN 123-456-7", vec![]),
            ("APN 123-456-789", vec![]),
            ("APN 123-456-78-9", vec![]),
            ("APN 1234-456-78", vec![]),
            ("APN 619-555-0198", vec![]),
            ("Parcel 123-456-78", vec![]),
        ];
        for (text, expected) in cases {
            assert_eq!(values(find_parcel_numbers(text, 100)), expected, "{text}");
        }
    }

    #[test]
    fn offsets_address_the_value_alone() {
        let text = "é APN 123-456-78 and CSLB 123456";
        let parcel = &find_parcel_numbers(text, 10)[0];
        assert_eq!(&text[parcel.byte_start..parcel.byte_end], "123-456-78");
        let license = &find_contractor_licenses(text, 10)[0];
        assert_eq!(&text[license.byte_start..license.byte_end], "123456");
    }
    #[test]
    fn a_series_number_needs_a_label_a_marker_and_a_numeric_non_date_value() {
        let cases: [(&str, Vec<&str>); 23] = [
            ("Resolution No. 2024-012", vec!["resolution:2024-012"]),
            ("per RESOLUTION NO. 2024-012.", vec!["resolution:2024-012"]),
            ("Ordinance Number: 1234", vec!["ordinance:1234"]),
            (
                "Purchase Order #po-24/0071",
                vec!["purchase order:PO-24/0071"],
            ),
            (
                "Permit No.BP-2024-0012, issued",
                vec!["permit:BP-2024-0012"],
            ),
            ("Work Order # 88", vec!["work order:88"]),
            (
                "RFP No. 23-045 and Bid No. 23-046",
                vec!["rfp:23-045", "bid:23-046"],
            ),
            ("Docket No. 24-CV-1234", vec!["docket:24-CV-1234"]),
            ("Contract Agreement No. 7.12", vec!["agreement:7.12"]),
            ("Resolution 2024-012", vec![]),
            ("the resolution, number twelve", vec![]),
            ("Contractor No. 1234", vec![]),
            ("Claims No. 1234", vec![]),
            ("Permit No. 5", vec![]),
            ("Bid No. A", vec![]),
            ("Invoice No. 2024-012A", vec![]),
            ("Invoice No. 1234567890123456789012345", vec![]),
            ("Resolution No. 12/15/2024", vec![]),
            ("Incident # 2024-03-15", vec![]),
            ("Citation No. 619-555-0198", vec![]),
            ("Citation No. (619) 555-0198", vec![]),
            ("Resolution No.\n2024-012", vec![]),
            ("Claim No. -12", vec![]),
        ];
        for (text, expected) in cases {
            assert_eq!(values(find_series_numbers(text, 100)), expected, "{text}");
        }
        let found = &find_series_numbers("é Purchase Order No. 2024-9.", 10)[0];
        assert_eq!(found.display, "Purchase Order 2024-9");
        assert_eq!(
            &"é Purchase Order No. 2024-9."[found.byte_start..found.byte_end],
            "2024-9"
        );
    }
}
