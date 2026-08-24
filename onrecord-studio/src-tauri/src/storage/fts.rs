//! Search statement formatting helpers for virtual full-text indexing nodes.

/// Cleans and formats raw user search text into an FTS5 token stream.
pub fn format_fts5_query(input: &str) -> String {
    let sanitized: String = input
        .chars()
        .map(|c| if c.is_alphanumeric() || c == ' ' { c } else { ' ' })
        .collect();
    
    sanitized
        .split_whitespace()
        .map(|word| format!("\"{}\"", word))
        .collect::<Vec<String>>()
        .join(" AND ")
}
