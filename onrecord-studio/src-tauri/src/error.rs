use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum OcrRequirementReason {
    EmptyCharacterLayer,
    LowCharacterDensity,
}

impl std::fmt::Display for OcrRequirementReason {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyCharacterLayer => formatter.write_str("empty character layer"),
            Self::LowCharacterDensity => formatter.write_str("low character density"),
        }
    }
}

// NOTE(ipc): Variant names and the tagged shape are consumed by the webview. Treat changes here
// as protocol changes and update the TypeScript error handling and contract tests together.
//
// INVARIANT(ipc): The serialized `errorType` is the VARIANT NAME ("Storage"), not `category()`
// ("STORAGE"). `category()` never crosses the bridge, so a frontend comparison against the
// SCREAMING_SNAKE form silently never matches.
//
// COMPAT(ipc): Every variant's `content` is a bare string except `OcrRequired`, whose content is
// an object. That asymmetry is why `formatNativeError` in src/ipc.ts stringifies a non-string
// message rather than assuming `typeof message === 'string'`. See docs/ipc-contract.md.
#[derive(Debug, Error, Serialize)]
#[serde(tag = "errorType", content = "message")]
pub enum AppError {
    #[error("Network operation failed: {0}")]
    Network(String),
    #[error("Portal discovery failed: {0}")]
    PortalDiscovery(String),
    #[error("Portal security policy rejected the operation: {0}")]
    PortalSecurity(String),
    #[error("Portal capacity limit reached: {0}")]
    PortalCapacity(String),
    #[error("Rate limiter rejected the operation: {0}")]
    RateLimiter(String),
    #[error("Storage operation failed: {0}")]
    Storage(String),
    #[error("Analytical operation failed: {0}")]
    Analysis(String),
    #[error("Compliance evaluation failed: {0}")]
    Compliance(String),
    #[error("Provenance verification failed: {0}")]
    Provenance(String),
    #[error("Report export failed: {0}")]
    Export(String),
    #[error("Data transformation failed: {0}")]
    DataParsing(String),
    #[error(
        "OCR required for '{path}': {reason}; density {density:.6}, minimum {minimum_density:.2}"
    )]
    OcrRequired {
        path: String,
        reason: OcrRequirementReason,
        density: f64,
        minimum_density: f64,
    },
    #[error("Internal execution boundary failed: {0}")]
    Internal(String),
}

impl AppError {
    pub const fn category(&self) -> &'static str {
        match self {
            Self::Network(_) => "NETWORK",
            Self::PortalDiscovery(_) => "PORTAL_DISCOVERY",
            Self::PortalSecurity(_) => "PORTAL_SECURITY",
            Self::PortalCapacity(_) => "PORTAL_CAPACITY",
            Self::RateLimiter(_) => "RATE_LIMITER",
            Self::Storage(_) => "STORAGE",
            Self::Analysis(_) => "ANALYSIS",
            Self::Compliance(_) => "COMPLIANCE",
            Self::Provenance(_) => "PROVENANCE",
            Self::Export(_) => "EXPORT",
            Self::DataParsing(_) => "DATA_PARSING",
            Self::OcrRequired { .. } => "OCR_REQUIRED",
            Self::Internal(_) => "INTERNAL",
        }
    }

    pub const fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::Network(_) | Self::RateLimiter(_) | Self::PortalCapacity(_)
        )
    }

    pub const fn is_source_repairable(&self) -> bool {
        matches!(
            self,
            Self::PortalDiscovery(_) | Self::DataParsing(_) | Self::OcrRequired { .. }
        )
    }
}

impl From<rusqlite::Error> for AppError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Storage(format!("SQLite failure: {error}"))
    }
}

impl From<duckdb::Error> for AppError {
    fn from(error: duckdb::Error) -> Self {
        Self::Analysis(format!("DuckDB failure: {error}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn serializes_a_stable_structured_ipc_error() {
        let value = serde_json::to_value(AppError::Storage("database unavailable".to_string()))
            .expect("AppError should serialize for the Tauri IPC boundary");

        assert_eq!(
            value,
            json!({
                "errorType": "Storage",
                "message": "database unavailable",
            })
        );
    }

    // INVARIANT(ipc): the wire tag is the variant name, never `category()`. A frontend matching
    // on the SCREAMING_SNAKE form would silently never match.
    #[test]
    fn the_wire_tag_is_the_variant_name_not_the_category() {
        let error = AppError::Storage("unavailable".to_string());
        let value = serde_json::to_value(&error).expect("serialize");

        assert_eq!(value["errorType"], "Storage");
        assert_eq!(error.category(), "STORAGE");
        assert_ne!(value["errorType"], error.category());
        // `category`, `is_retryable` and `is_source_repairable` are native-side policy and must
        // not appear on the wire; the frontend can only branch on `errorType`.
        assert!(value.get("category").is_none());
        assert!(value.get("isRetryable").is_none());
    }

    // COMPAT(ipc): OcrRequired is the one variant whose `content` is an object rather than a
    // string. This pins the asymmetry that src/ipc.ts must keep handling.
    #[test]
    fn ocr_required_serializes_a_structured_message_unlike_every_other_variant() {
        let value = serde_json::to_value(AppError::OcrRequired {
            path: "/tmp/scan.pdf".to_string(),
            reason: OcrRequirementReason::LowCharacterDensity,
            density: 0.004,
            minimum_density: 0.02,
        })
        .expect("serialize");

        assert_eq!(value["errorType"], "OcrRequired");
        assert!(
            value["message"].is_object(),
            "OcrRequired carries a structured message: {value}",
        );
        assert_eq!(value["message"]["reason"], "LOW_CHARACTER_DENSITY");
        assert_eq!(value["message"]["path"], "/tmp/scan.pdf");
        // COMPAT(ipc): these fields are snake_case on the wire. `AppError` carries `tag` and
        // `content` but NOT `rename_all_fields`, so this is the one payload on the bridge that
        // breaks the camelCase convention every other struct follows. Adding the attribute would
        // be a breaking protocol change, so the inconsistency is pinned rather than fixed.
        assert_eq!(value["message"]["minimum_density"], 0.02);
        assert!(value["message"].get("minimumDensity").is_none());

        // Every other variant keeps a bare string, which is the shape consumers assume.
        let string_message =
            serde_json::to_value(AppError::Network("timeout".to_string())).expect("serialize");
        assert!(string_message["message"].is_string());
    }
}
