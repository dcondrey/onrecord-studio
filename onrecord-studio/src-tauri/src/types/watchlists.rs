use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WatchKind {
    Agency,
    Entity,
    TopicPack,
    Query,
    Source,
}

impl WatchKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Agency => "AGENCY",
            Self::Entity => "ENTITY",
            Self::TopicPack => "TOPIC_PACK",
            Self::Query => "QUERY",
            Self::Source => "SOURCE",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "AGENCY" => Self::Agency,
            "ENTITY" => Self::Entity,
            "TOPIC_PACK" => Self::TopicPack,
            "QUERY" => Self::Query,
            "SOURCE" => Self::Source,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Watch {
    /// Empty on creation; the repository assigns one.
    pub id: String,
    pub kind: WatchKind,
    /// Agency name, entity phrase, topic pack id, query terms, or portal key, by `kind`.
    pub target: String,
    pub label: String,
    pub created_at: String,
    pub enabled: bool,
}

/// What a check did for one dimension. `NotMeasured` and `Unchanged` are different facts: the
/// first says nothing was observed this run, the second says it was observed and matched.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DimensionOutcome {
    BaselineRecorded,
    Unchanged,
    Changed,
    NotMeasured,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WatchDimension {
    Documents,
    Findings,
    Figures,
    Withheld,
    SourceHealth,
}

impl WatchDimension {
    pub const ALL: [WatchDimension; 5] = [
        Self::Documents,
        Self::Findings,
        Self::Figures,
        Self::Withheld,
        Self::SourceHealth,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Documents => "DOCUMENTS",
            Self::Findings => "FINDINGS",
            Self::Figures => "FIGURES",
            Self::Withheld => "WITHHELD",
            Self::SourceHealth => "SOURCE_HEALTH",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|d| d.as_str() == value)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DimensionResult {
    pub dimension: WatchDimension,
    pub outcome: DimensionOutcome,
    pub change_count: u32,
    /// When the comparison baseline was observed. `None` when there was no baseline.
    pub baseline_as_of: Option<String>,
    /// When this run observed the dimension. `None` when `outcome` is `NotMeasured`.
    pub observed_as_of: Option<String>,
    /// Why nothing was observed. Set only for `NotMeasured`.
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WatchCheckStatus {
    /// Every observed dimension only recorded its first baseline.
    Baseline,
    Compared,
    /// Nothing could be observed this run.
    NoData,
}

/// One change. `old_value` is `None` when the baseline did not contain the subject, and
/// `new_value` is `None` when the current observation does not; `basis` says which.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WatchChange {
    pub id: String,
    pub watch_id: String,
    pub dimension: WatchDimension,
    pub change_kind: String,
    pub subject: String,
    pub old_value: Option<String>,
    pub new_value: Option<String>,
    pub old_as_of: Option<String>,
    pub new_as_of: String,
    pub basis: String,
    pub detected_at: String,
    /// The source the change was read from, so a source purge can remove it.
    pub portal_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WatchCheck {
    pub watch_id: String,
    pub checked_at: String,
    pub status: WatchCheckStatus,
    pub dimensions: Vec<DimensionResult>,
    pub new_changes: Vec<WatchChange>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WatchSummary {
    pub watch: Watch,
    pub last_checked_at: Option<String>,
    /// Empty when the watch has never been checked.
    pub last_dimensions: Vec<DimensionResult>,
}
