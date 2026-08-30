use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CorpusEntityKind {
    Statute,
    Date,
    EmailDomain,
    EmailAddress,
    UrlHost,
    Acronym,
    Organization,
    Money,
}

impl CorpusEntityKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Statute => "STATUTE",
            Self::Date => "DATE",
            Self::EmailDomain => "EMAIL_DOMAIN",
            Self::EmailAddress => "EMAIL_ADDRESS",
            Self::UrlHost => "URL_HOST",
            Self::Acronym => "ACRONYM",
            Self::Organization => "ORGANIZATION",
            Self::Money => "MONEY",
        }
    }
}

/// How hard a mention's extractor actually verified the string it matched.
///
/// A confidence number cannot carry this. 9,000 basis points on a pattern that merely looked
/// right and 9,000 on one whose check digit was recomputed are the same number and are not the
/// same claim, and an investigative finding built on the second survives a challenge the first
/// does not. Per `docs/investigative-workbench-architecture.md` section 4: a `PatternOnly` hit is
/// a lead, never a finding, and the reader must not present it as one.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EntityValidationGrade {
    /// A check digit or equivalent arithmetic constraint was recomputed and matched.
    CheckDigitPassed,
    /// The string was parsed by a real parser, or satisfies a structural rule that random text
    /// does not: a URL that `Url::parse` accepted, a calendar date whose month and day are in
    /// range, a currency amount with a well-formed minor unit.
    FormatValidated,
    /// The shape matched and nothing verified it. A lead.
    PatternOnly,
    /// The string was matched against a maintained word list rather than validated.
    GazetteerHit,
}

impl EntityValidationGrade {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CheckDigitPassed => "CHECK_DIGIT_PASSED",
            Self::FormatValidated => "FORMAT_VALIDATED",
            Self::PatternOnly => "PATTERN_ONLY",
            Self::GazetteerHit => "GAZETTEER_HIT",
        }
    }
}

/// Why two mentions were treated as the same entity.
///
/// Merging is inference and the merge basis has to survive with the merge, so an investigator can
/// see which nodes the app decided were one thing and on what grounds. Only
/// `ExactNormalizedMatch` is produced today: every merge is a byte-identical normalized value, and
/// nothing looser is applied silently. The other two exist because the column that records them
/// has to be in the schema before a proposal path can write one, and adding it later is another
/// corpus rebuild.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EntityResolutionBasis {
    ExactNormalizedMatch,
    HeuristicSimilarity,
    InvestigatorConfirmed,
}

impl EntityResolutionBasis {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ExactNormalizedMatch => "EXACT_NORMALIZED_MATCH",
            Self::HeuristicSimilarity => "HEURISTIC_SIMILARITY",
            Self::InvestigatorConfirmed => "INVESTIGATOR_CONFIRMED",
        }
    }
}

/// How an edge came to exist, per `docs/investigative-workbench-architecture.md` section 3.
///
/// `Observed` is read out of the bytes and is as strong as the document. `Computed` is a
/// derivation the app performed and can re-derive: co-occurrence is this, and calling it anything
/// else would be a claim about the world that the corpus never made. `Asserted` is drawn by an
/// investigator. A reader that renders the three alike is the same failure as a proxy date
/// presented as a sourced determination, which `analysis/statute.rs` already refuses.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RelationshipBasis {
    Observed,
    Computed,
    Asserted,
}

impl RelationshipBasis {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Observed => "OBSERVED",
            Self::Computed => "COMPUTED",
            Self::Asserted => "ASSERTED",
        }
    }
}

impl std::fmt::Display for CorpusEntityKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CorpusPatternKind {
    ExactDuplicate,
    NearDuplicate,
    SharedStatute,
    SharedEntity,
    SharedTerm,
    TemporalCluster,
    EntityRelationship,
}

impl CorpusPatternKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ExactDuplicate => "EXACT_DUPLICATE",
            Self::NearDuplicate => "NEAR_DUPLICATE",
            Self::SharedStatute => "SHARED_STATUTE",
            Self::SharedEntity => "SHARED_ENTITY",
            Self::SharedTerm => "SHARED_TERM",
            Self::TemporalCluster => "TEMPORAL_CLUSTER",
            Self::EntityRelationship => "ENTITY_RELATIONSHIP",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CorpusEvidenceReference {
    pub portal_key: String,
    pub request_id: u64,
    pub document_id: u64,
    pub segment_index: Option<usize>,
    pub character_start: Option<usize>,
    pub character_end: Option<usize>,
    pub evidence_label: String,
    pub content_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CorpusPattern {
    pub pattern_id: String,
    pub kind: CorpusPatternKind,
    pub title: String,
    pub explanation: String,
    pub confidence_basis_points: u16,
    pub evidence: Vec<CorpusEvidenceReference>,
    pub algorithm_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CorpusPatternQuery {
    pub portal_key: Option<String>,
    pub minimum_evidence_count: usize,
    #[serde(default)]
    pub minimum_confidence_basis_points: u16,
    #[serde(default)]
    pub indexed_from: Option<String>,
    #[serde(default)]
    pub indexed_to: Option<String>,
    #[serde(default)]
    pub pattern_kinds: Vec<CorpusPatternKind>,
    #[serde(default)]
    pub required_terms: Vec<String>,
    pub result_limit: usize,
}

impl Default for CorpusPatternQuery {
    fn default() -> Self {
        Self {
            portal_key: None,
            minimum_evidence_count: 2,
            minimum_confidence_basis_points: 0,
            indexed_from: None,
            indexed_to: None,
            pattern_kinds: Vec::new(),
            required_terms: Vec::new(),
            result_limit: 100,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CorpusIndexStatus {
    pub indexed_document_count: usize,
    pub indexed_segment_count: usize,
    pub indexed_term_count: usize,
    pub indexed_relationship_count: usize,
    /// Remaining bounded writer-queue slots. A high value means idle capacity; zero means new
    /// corpus writes will wait for the serialized writer.
    pub available_write_capacity: usize,
    pub algorithm_version: String,
}
