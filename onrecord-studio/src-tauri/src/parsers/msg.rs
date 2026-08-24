//! Outlook email extraction engine isolating raw text layers and communication maps.

use std::path::Path;
use msg_parser::OutlookMessage;
use crate::error::AppError;

pub struct ExtractedEmailPayload {
    pub body_text: String,
    pub metadata_summary: String,
}

/// Parses local binary .msg email artifacts into structured, indexable string layers.
pub fn extract_msg_layers<P: AsRef<Path>>(file_path: P) -> Result<ExtractedEmailPayload, AppError> {
    let file_bytes = std::fs::read(file_path)
        .map_err(|e| AppError::Internal(format!("Failed to read target email file stream: {}", e)))?;
        
    let message = OutlookMessage::from_bytes(&file_bytes)
        .map_err(|e| AppError::DataParsing(format!("MSG parser extraction breakdown: {:?}", e)))?;

    // 1. Structure clear communication header trails
    let sender_name = message.sender.name.unwrap_or_else(|| "Unknown Sender".to_string());
    let sender_email = message.sender.email.unwrap_or_else(|| "unknown@agency.gov".to_string());
    let subject = message.subject.unwrap_or_else(|| "No Subject".to_string());
    
    let mut metadata_summary = format!(
        "From: {} <{}>\nSubject: {}\nDate Logged: {:?}\n",
        sender_name, sender_email, subject, message.headers.date
    );

    // Append standard recipient paths to the query layer
    if let Some(recipients) = message.recipients {
        for rx in recipients {
            let rx_name = rx.name.unwrap_or_default();
            let rx_email = rx.email.unwrap_or_default();
            metadata_summary.push_str(&format!("To: {} <{}>\n", rx_name, rx_email));
        }
    }

    // 2. Isolate body text. Prioritize plain text layer, fallback to HTML layer processing if missing.
    let body_text = if let Some(text_body) = message.body.text {
        text_body
    } else if let Some(html_body) = message.body.html {
        // Strip primitive HTML structures inline using basic regex if a fully fledged DOM tree engine isn't present
        let html_strip_regex = regex::Regex::new(r"<[^>]*>").unwrap();
        html_strip_regex.replace_all(&html_body, "").to_string()
    } else {
        String::new()
    };

    Ok(ExtractedEmailPayload {
        body_text: body_text.trim().to_string(),
        metadata_summary,
    })
}
