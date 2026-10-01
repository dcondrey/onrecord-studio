//! Which identifier kinds can raise a candidate, and for whom.
//!
//! An identifier shared by two entities is a reason to look, never an identity. Each kind below
//! names the entity kinds it may anchor: this is where an identifier is stopped from linking
//! entities it says nothing about. A parcel number identifies a property, so it anchors no
//! name-resolved kind: two people or organizations that both sit near one APN are neighbours of a
//! parcel, not each other.

/// How an identifier kind may link entities.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IdentifierRule {
    pub kind: &'static str,
    /// Entity kinds this identifier may anchor. Empty means it anchors none of the kinds the
    /// resolution scan compares (people and organizations).
    pub anchors: &'static [&'static str],
    /// The identifier and the entity must be this many characters apart or closer (the span of the
    /// co-occurrence edge covering both) in a document for it to count. `None` is the whole
    /// segment. The edge spans the first mention of each in a segment, so a nearer later mention is
    /// not seen: the rule can miss a link, never invent one.
    pub maximum_span_characters: Option<u64>,
    /// An identifier linking more entities than this is a shared mailbox or a template value, and
    /// is skipped and counted rather than paired.
    pub maximum_fanout: usize,
}

pub const IDENTIFIER_RULES: [IdentifierRule; 4] = [
    IdentifierRule {
        kind: "EMAIL_ADDRESS",
        anchors: &["PERSON", "ORGANIZATION"],
        maximum_span_characters: None,
        maximum_fanout: 12,
    },
    IdentifierRule {
        kind: "EIN",
        anchors: &["ORGANIZATION"],
        maximum_span_characters: Some(200),
        maximum_fanout: 8,
    },
    IdentifierRule {
        kind: "CONTRACTOR_LICENSE",
        anchors: &["PERSON", "ORGANIZATION"],
        maximum_span_characters: Some(200),
        maximum_fanout: 8,
    },
    IdentifierRule {
        kind: "PARCEL_NUMBER",
        anchors: &[],
        maximum_span_characters: Some(200),
        maximum_fanout: 8,
    },
];

pub fn identifier_rule(kind: &str) -> Option<&'static IdentifierRule> {
    IDENTIFIER_RULES.iter().find(|rule| rule.kind == kind)
}

/// Whether `identifier_kind` may link entities of `entity_kind`.
pub fn may_anchor(identifier_kind: &str, entity_kind: &str) -> bool {
    identifier_rule(identifier_kind).is_some_and(|rule| rule.anchors.contains(&entity_kind))
}

/// The SQL condition tying each identifier kind to the entity kinds it may anchor and the span it
/// may cover. `identifier` and `named` are table aliases; `span` is the edge span expression.
pub(crate) fn link_condition(identifier: &str, named: &str, span: &str) -> String {
    let clauses: Vec<String> = IDENTIFIER_RULES
        .iter()
        .filter(|rule| !rule.anchors.is_empty())
        .map(|rule| {
            let anchors = rule
                .anchors
                .iter()
                .map(|kind| format!("'{kind}'"))
                .collect::<Vec<_>>()
                .join(", ");
            let limit = rule
                .maximum_span_characters
                .map(|limit| format!(" AND {span} <= {limit}"))
                .unwrap_or_default();
            format!(
                "({identifier}.entity_kind = '{}' AND {named}.entity_kind IN ({anchors}){limit})",
                rule.kind
            )
        })
        .collect();
    format!("({})", clauses.join(" OR "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_parcel_anchors_no_person_or_organization_and_an_ein_only_organizations() {
        for entity in ["PERSON", "ORGANIZATION"] {
            assert!(!may_anchor("PARCEL_NUMBER", entity));
        }
        assert!(may_anchor("EIN", "ORGANIZATION"));
        assert!(!may_anchor("EIN", "PERSON"));
        assert!(!may_anchor("DATE", "PERSON"));
    }
}
