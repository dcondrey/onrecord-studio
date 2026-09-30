//! The verifiable "show your work" report: a single self-contained HTML file whose signed
//! payload the reader's browser (and `scripts/verify-report.mjs`) re-verifies without this
//! application. Signing reuses `integrity::sign_package` so there is one signing scheme.

pub mod builder;
pub mod output;
pub mod packet;
pub mod source;

use crate::error::AppError;
use crate::integrity::{sha256_hex, sign_package, verify_package};
use crate::types::report::VerifiableReportManifest;
use ed25519_dalek::SigningKey;
use std::collections::HashMap;

pub const TEMPLATE: &str = include_str!("../../templates/verifiable-report.html");
pub const PAYLOAD_PLACEHOLDER: &str = "__ONRECORD_REPORT_PAYLOAD__";
pub const MAXIMUM_REPORT_BYTES: usize = 8 * 1024 * 1024;

/// What the stored document contributes to a citation. `text` is `None` when the document is not
/// public, has no stored text, or the text exceeded the loading budget.
#[derive(Debug, Clone, Default)]
pub struct SourceDocument {
    pub file_name: Option<String>,
    pub visibility: String,
    pub raw_file_sha256: Option<String>,
    pub extracted_text_projection_sha256: Option<String>,
    pub text: Option<String>,
    pub download_url: Option<String>,
}

impl SourceDocument {
    pub fn is_public(&self) -> bool {
        self.visibility.trim().eq_ignore_ascii_case("public")
    }
}

pub type SourceDocuments = HashMap<(String, i64), SourceDocument>;

pub fn template_sha256() -> String {
    sha256_hex(TEMPLATE.as_bytes())
}

pub struct RenderedReport {
    pub html: String,
    pub payload_sha256: String,
    pub signer_key_id: String,
    pub identity_assurance: String,
}

pub fn sign_and_render(
    manifest: VerifiableReportManifest,
    signing_key: &SigningKey,
) -> Result<RenderedReport, AppError> {
    if TEMPLATE.matches(PAYLOAD_PLACEHOLDER).count() != 1 {
        return Err(AppError::Internal(
            "Report template must contain the payload placeholder exactly once".to_string(),
        ));
    }
    let package = sign_package(manifest, signing_key).map_err(AppError::Export)?;
    // A report this installation cannot itself re-verify must never reach a reader.
    verify_package(&package).map_err(|error| {
        AppError::Export(format!(
            "Freshly signed report failed self-verification: {error}"
        ))
    })?;
    let payload = serde_json::to_string(&package)
        .map_err(|error| AppError::Export(format!("Failed to encode report payload: {error}")))?
        // `<` never occurs outside a JSON string, so this keeps `</script>` and `<!--` inside a
        // quote from ending or reshaping the embedding tag without changing the parsed value.
        .replace('<', "\\u003c");
    let html = TEMPLATE.replacen(PAYLOAD_PLACEHOLDER, &payload, 1);
    if html.len() > MAXIMUM_REPORT_BYTES {
        return Err(AppError::Export(format!(
            "Report exceeds the {MAXIMUM_REPORT_BYTES}-byte limit; select fewer findings"
        )));
    }
    Ok(RenderedReport {
        html,
        payload_sha256: package.manifest_sha256,
        signer_key_id: package.signature.signer_key_id,
        identity_assurance: package.signature.identity_assurance,
    })
}

#[cfg(test)]
mod tests;
