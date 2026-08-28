//! Shared failure cleanup and startup recognition for atomic container-extraction staging.

use std::fs;
use std::path::Path;

use uuid::Uuid;

use crate::error::AppError;

pub(crate) fn cleanup_failed_extraction(
    staging: &Path,
    label: &str,
    primary_error: AppError,
) -> AppError {
    match fs::remove_dir_all(staging) {
        Ok(()) => primary_error,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => primary_error,
        Err(cleanup_error) => AppError::Internal(format!(
            "{label}_STAGING_CLEANUP_FAILED: failed to remove partial extraction '{}': {cleanup_error}. Original failure: {primary_error}",
            staging.display()
        )),
    }
}

/// Recognizes only the generated `.<destination>.<uuid>.extracting` directory convention.
/// Startup reconciliation uses this predicate so ordinary hidden evidence directories are kept.
pub(crate) fn is_managed_extraction_staging_name(name: &str) -> bool {
    let Some(body) = name
        .strip_prefix('.')
        .and_then(|value| value.strip_suffix(".extracting"))
    else {
        return false;
    };
    let Some((destination, identifier)) = body.rsplit_once('.') else {
        return false;
    };
    !destination.is_empty() && Uuid::parse_str(identifier).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_only_generated_extraction_staging_names() {
        assert!(is_managed_extraction_staging_name(
            ".report.contents.550e8400-e29b-41d4-a716-446655440000.extracting"
        ));
        assert!(!is_managed_extraction_staging_name(".report.extracting"));
        assert!(!is_managed_extraction_staging_name(
            "report.550e8400-e29b-41d4-a716-446655440000.extracting"
        ));
        assert!(!is_managed_extraction_staging_name(
            ".report.not-a-uuid.extracting"
        ));
    }

    #[test]
    fn cleanup_failure_preserves_the_original_error_in_its_diagnostic() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let not_a_directory = directory.path().join("partial");
        fs::write(&not_a_directory, b"partial").expect("fixture file");
        let error = cleanup_failed_extraction(
            &not_a_directory,
            "ZIP",
            AppError::DataParsing("invalid entry".to_string()),
        );
        let message = error.to_string();
        assert!(message.contains("ZIP_STAGING_CLEANUP_FAILED"));
        assert!(message.contains("invalid entry"));
    }
}
