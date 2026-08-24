//! Native spreadsheet extraction module parsing .xlsx layouts into indexable data rows.

use std::path::Path;
use calamine::{open_workbook, Reader, Xlsx, DataType};
use crate::error::AppError;

/// Parses all sheets inside an Excel workbook, flattening grids into a clean CSV-like text block.
pub fn extract_spreadsheet_layers<P: AsRef<Path>>(file_path: P) -> Result<String, AppError> {
    let mut workbook: Xlsx<_> = open_workbook(file_path)
        .map_err(|e| AppError::DataParsing(format!("Failed to open Excel engine workbook: {}", e)))?;

    let mut flattened_text = String::new();
    let sheet_names = workbook.sheet_names().to_vec();

    for sheet_name in sheet_names {
        if let Some(Ok(range)) = workbook.worksheet_range(&sheet_name) {
            flattened_text.push_str(&format!("--- Sheet: {} ---\n", sheet_name));
            
            for row in range.rows() {
                let row_text: Vec<String> = row.iter()
                    .map(|cell| match cell {
                        DataType::Empty => String::new(),
                        DataType::String(s) => s.replace(',', " ").replace('\n', " ").trim().to_string(),
                        DataType::Float(f) => f.to_string(),
                        DataType::Int(i) => i.to_string(),
                        DataType::Bool(b) => b.to_string(),
                        DataType::Error(e) => format!("[ERR: {:?}]", e),
                        DataType::DateTime(d) => d.to_string(),
                    })
                    .collect();
                
                // Only join and append if the row isn't entirely blank space
                let joined_row = row_text.join(", ");
                if !joined_row.trim().replace(',', "").is_empty() {
                    flattened_text.push_str(&joined_row);
                    flattened_text.push('\n');
                }
            }
        }
    }

    Ok(flattened_text)
}
