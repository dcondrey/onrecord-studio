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
    /// A human actor. Distinct from `EmailAddress` because one person may hold several addresses
    /// or none; policy-vs-practice reasoning has to name the person, not the mailbox.
    Person,
    /// A named function ("Chief Executive", "Records Officer") independent of the person filling
    /// it. Needed so an org chart claim can be checked against operational signers.
    Role,
    /// A named seat that ties a `Role` to a specific unit ("Records Officer for Department X").
    Position,
    /// A document's role in a policy-vs-practice comparison — policy vs. record vs. response vs.
    /// minutes vs. contract vs. denial vs. budget vs. other. Authoritative from observational.
    DocumentClass,
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
            Self::Person => "PERSON",
            Self::Role => "ROLE",
            Self::Position => "POSITION",
            Self::DocumentClass => "DOCUMENT_CLASS",
        }
    }
}

/// The role a `Money` mention plays in the document it appears in.
///
/// `Money` entities are shared across documents (a shared amount is a graph edge), but the same
/// amount can be an invoice line in one document, a budget appropriation in another, and a
/// payment record in a third — and the point of the reasoning is often "the same number, three
/// different ledgers". The context lives on the mention, not on the entity.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MoneyContext {
    Invoice,
    Approval,
    Budget,
    Payment,
    Other,
}

impl MoneyContext {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Invoice => "INVOICE",
            Self::Approval => "APPROVAL",
            Self::Budget => "BUDGET",
            Self::Payment => "PAYMENT",
            Self::Other => "OTHER",
        }
    }
}

/// What an edge is claiming, not merely that it exists.
///
/// `CoOccurrence` is the current default: two entities appeared near each other in one segment,
/// which the app derived and can re-derive. The other variants encode edges the text actually
/// carried — a signature line for `SignedBy`, an approval sentence for `Approved`, a statute
/// citation for `Cites`. `Promises` and `Delivered` are the anchor of policy-vs-practice
/// comparison: a document that says an obligation was undertaken vs. a document that says it was
/// fulfilled. `Supersedes` and `Amends` carry temporal replacement between two documents.
///
/// A reader that draws a `CoOccurrence` edge alike to a `SignedBy` edge makes a claim about the
/// world the app did not — the same failure `RelationshipBasis` guards against, one dimension
/// down. Distance in the graph does not equal semantic strength.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RelationshipKind {
    CoOccurrence,
    SignedBy,
    Approved,
    Cites,
    Promises,
    Delivered,
    Supersedes,
    Amends,
}

impl RelationshipKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CoOccurrence => "CO_OCCURRENCE",
            Self::SignedBy => "SIGNED_BY",
            Self::Approved => "APPROVED",
            Self::Cites => "CITES",
            Self::Promises => "PROMISES",
            Self::Delivered => "DELIVERED",
            Self::Supersedes => "SUPERSEDES",
            Self::Amends => "AMENDS",
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

/// One node, as the graph draws it.
///
/// `document_count` is the number of distinct documents the entity was mentioned in, which is what
/// makes a node worth looking at; `validation_grade` is the weakest grade among those mentions, so
/// a node the extractors never verified cannot render as one they did.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CorpusEntityNode {
    pub entity_id: String,
    pub kind: CorpusEntityKind,
    pub display_value: String,
    pub document_count: u64,
    pub validation_grade: EntityValidationGrade,
}

/// One arc, and what it is allowed to claim.
///
/// `shared_document_count` is the count of distinct documents in which the two entities appeared
/// in the same segment. `basis` is `COMPUTED` for every edge the app writes today, and it travels
/// with the edge rather than being assumed by the reader, because an observed edge and a derived
/// one must not be drawn alike.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CorpusEntityEdge {
    pub neighbor: CorpusEntityNode,
    pub basis: RelationshipBasis,
    /// What the edge is claiming. `CoOccurrence` is the default: the app derived it. Typed
    /// variants (`SignedBy`, `Cites`, `Promises`, `Delivered`, `Supersedes`, `Amends`,
    /// `Approved`) mean the text actually carried that relationship. A reader that renders
    /// them alike drops the difference between "were near each other in one paragraph" and
    /// "one signed the other into effect".
    pub kind: RelationshipKind,
    pub shared_document_count: u64,
    pub confidence_basis_points: u16,
    /// RFC 3339 UTC. Inclusive lower bound on the edge's temporal validity, when the segment
    /// carried a date; `None` when it did not. Never populated to a default value — an unknown
    /// validity and one that starts at the epoch are not the same claim.
    #[serde(default)]
    pub valid_from: Option<String>,
    /// RFC 3339 UTC. Inclusive upper bound on the edge's temporal validity, or `None` for open-
    /// ended / unknown. Distinguishing absent from "still valid" belongs to the reader, not the
    /// schema.
    #[serde(default)]
    pub valid_until: Option<String>,
}

/// One step along a path returned by [`crate::storage::corpus::CorpusIndex::find_entity_paths`].
///
/// The `neighbor` is the node this step arrived at; the incoming `edge_kind`, `edge_basis`, and
/// `shared_document_count` describe the arc traversed. The step at index 0 in a path is the
/// starting entity itself with `edge_kind = CoOccurrence` and `shared_document_count = 0` as a
/// sentinel — a callsite that would treat the origin's edge as real should reject step 0
/// explicitly rather than relying on the sentinel to be safe.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EntityPathStep {
    pub neighbor: CorpusEntityNode,
    pub edge_kind: RelationshipKind,
    pub edge_basis: RelationshipBasis,
    pub shared_document_count: u64,
}

/// A path of entities linking `from` to `to`, ordered by traversal.
///
/// `hops` is `steps.len() - 1` for a caller that just wants length. Confidence is the weakest
/// edge in the path — a path is only as strong as its weakest arc, and reporting the strongest
/// would flatter chains held together by one thin edge.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EntityPath {
    pub steps: Vec<EntityPathStep>,
    pub hops: usize,
    /// Basis points. The minimum `edge_confidence` across steps > 0. `None` if the path is
    /// trivial (start == end); a zero-hop path is a real answer but has no edge to be as strong
    /// as, so absence is honest.
    pub weakest_confidence_basis_points: Option<u16>,
}

/// An entity ranked by how much information flows through it in the current graph.
///
/// `brokerage_score` is a degree-based approximation of betweenness centrality: real betweenness
/// on a graph this size is O(V*E) per source and unaffordable in a bounded query. The score
/// counts weighted incident-edge strength — high-degree, high-shared-document nodes score
/// highest. `is_approximation = true` on every row so a reader never mistakes this for a
/// computed centrality measure it would defend in court.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EntityBroker {
    pub entity: CorpusEntityNode,
    pub brokerage_score: u64,
    pub distinct_neighbors: u64,
    pub is_approximation: bool,
}

/// The subgraph around one entity.
///
/// `truncated` says the neighbour list hit its limit, so a reader can tell a sparse entity from a
/// busy one whose arcs were cut. An absent flag would make those two look the same, which is the
/// same failure as a missing truncation row on an entity kind.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CorpusEntityNeighborhood {
    pub center: CorpusEntityNode,
    pub edges: Vec<CorpusEntityEdge>,
    pub truncated: bool,
}
