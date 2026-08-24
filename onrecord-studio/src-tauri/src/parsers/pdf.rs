//! Native PDF extraction engine isolating textual layers and auditing layout profiles.

use std::path::Path;
use std::collections::BTreeMap;
use lopdf::{Document, Object};
use crate::error::AppError;

/// Processes disk binary streams to pull text tokens from readable PDF layouts.
pub fn extract_text_layers<P: AsRef<Path>>(file_path: P) -> Result<String, AppError> {
    let doc = Document::load(file_path)
        .map_err(|e| AppError::DataParsing(format!("Failed to parse structural PDF file layout: {}", e)))?;
    
    let mut extracted_text = String::new();
    
    // Sort page references to ensure strict chronological flow of reading arrays
    let pages: BTreeMap<u32, (lopdf::ObjectId, Object)> = doc.get_pages();

    for &page_num in pages.keys() {
        if let Ok(text) = doc.extract_text(&[page_num]) {
            extracted_text.push_str(&text);
            extracted_text.push('\n');
        }
    }

    Ok(extracted_text)
}

/// Evaluates whether an extracted document string is empty or masked vector content.
/// Used to determine if the file should be routed to a Local OCR queue.
pub fn requires_ocr_reprocessing(text: &str, min_character_density: usize) -> bool {
    if text.trim().is_empty() {
        return true;
    }
    
    // Strip trailing margins and line feeds to compute real alpha characters density bounds
    let clean_count = text.chars().filter(|c| c.is_alphanumeric()).count();
    clean_count < min_character_density
}
