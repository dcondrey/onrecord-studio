//! Read-only audit of an existing corpus. No downloads, model calls, or source mutations.
use onrecord_studio_lib::parsers::{
    audit_document_path, recovery, unredactor, ForensicAuditOptions,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
use std::{path::PathBuf, time::Instant};

#[test]
#[ignore = "Set ONRECORD_FORENSIC_MANIFEST to a JSON document inventory; audits existing files without changing them"]
fn audit_existing_corpus_for_actual_recoverable_evidence() {
    let manifest = PathBuf::from(
        std::env::var("ONRECORD_FORENSIC_MANIFEST").expect("explicit corpus manifest"),
    );
    let output = PathBuf::from(
        std::env::var("ONRECORD_FORENSIC_OUTPUT").unwrap_or_else(|_| {
            std::env::temp_dir()
                .join("onrecord-forensic-validation")
                .to_string_lossy()
                .into()
        }),
    );
    std::fs::create_dir_all(&output).unwrap();
    let records: Vec<Value> = serde_json::from_slice(&std::fs::read(manifest).unwrap()).unwrap();
    let mut summaries = vec![];
    for record in records {
        let Some(path) = record["local_path"].as_str() else {
            continue;
        };
        let start = Instant::now();
        let source = std::fs::read(path).unwrap();
        let digest = sha256_hex(&source);
        if let Some(expected) = record["raw_file_sha256"].as_str() {
            assert_eq!(
                digest, expected,
                "Source integrity: {}",
                record["file_name"]
            );
        }
        let pdf = source.starts_with(b"%PDF-");
        drop(source);
        let mut result = json!({"document":record,"sourceSha256":digest,"pdf":pdf});
        if pdf {
            result["geometry"] = match unredactor::evaluate_vector_redaction_leaks(path) {
                Ok(report) => serde_json::to_value(report).unwrap(),
                Err(error) => json!({"error":error.to_string()}),
            };
            result["revisions"] = match recovery::carve_historical_pdf_versions(path) {
                Ok(report) => serde_json::to_value(report).unwrap(),
                Err(error) => json!({"error":error.to_string()}),
            };
        }
        result["residual"] = match audit_document_path(
            path,
            ForensicAuditOptions {
                include_hypotheses: true,
                preserve_raw_values: false,
            },
        ) {
            Ok(report) => serde_json::to_value(report).unwrap(),
            Err(error) => json!({"error":error.to_string()}),
        };
        assert_eq!(
            sha256_hex(&std::fs::read(path).unwrap()),
            digest,
            "Audit changed source bytes"
        );
        result["elapsedMs"] = json!(start.elapsed().as_millis());
        let id = record["id"].as_u64().unwrap();
        std::fs::write(
            output.join(format!("{id}.json")),
            serde_json::to_vec_pretty(&result).unwrap(),
        )
        .unwrap();
        let summary = json!({"id":id,"fileName":record["file_name"],"sourceUrl":record["download_url"],"sourceSha256":digest,"pdf":pdf,"pages":result["geometry"]["scannedPageCount"],"leakyFlag":result["geometry"]["isLeakyRedaction"],"geometryComplete":result["geometry"]["coverageComplete"],"clashes":result["geometry"]["boundingBoxClashesCount"],"hiddenCharacters":result["geometry"]["extractedHiddenText"].as_str().map(str::len),"historicalVersions":result["revisions"]["historicalVersionsFound"],"recoveredSegments":result["revisions"]["recoveredTextSegments"].as_array().map(Vec::len),"residualFindings":result["residual"]["findings"].as_array().map(Vec::len),"geometryError":result["geometry"]["error"],"revisionError":result["revisions"]["error"],"residualError":result["residual"]["error"],"elapsedMs":result["elapsedMs"]});
        println!("{}", summary);
        summaries.push(summary);
        std::fs::write(
            output.join("summary.json"),
            serde_json::to_vec_pretty(&summaries).unwrap(),
        )
        .unwrap();
    }
    assert!(!summaries.is_empty());
}

#[test]
#[ignore = "Compare recovered corpus revisions against the current PDF using the same native text extractor"]
fn recovered_versions_are_compared_with_current_document_text() {
    let root = PathBuf::from(
        std::env::var("ONRECORD_FORENSIC_OUTPUT").unwrap_or_else(|_| {
            std::env::temp_dir()
                .join("onrecord-forensic-validation")
                .to_string_lossy()
                .into()
        }),
    );
    let rows: Vec<Value> =
        serde_json::from_slice(&std::fs::read(root.join("summary.json")).unwrap()).unwrap();
    let mut comparisons = vec![];
    for row in rows
        .iter()
        .filter(|row| row["recoveredSegments"].as_u64().unwrap_or(0) > 0)
    {
        let result: Value = serde_json::from_slice(
            &std::fs::read(root.join(format!("{}.json", row["id"]))).unwrap(),
        )
        .unwrap();
        let doc =
            lopdf::Document::load(result["document"]["local_path"].as_str().unwrap()).unwrap();
        let mut pages = vec![];
        let mut errors = 0;
        for page in doc.get_pages().keys() {
            match doc.extract_text(&[*page]) {
                Ok(text) => pages.push(text),
                Err(_) => errors += 1,
            }
        }
        let current = pages
            .join("\n")
            .lines()
            .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        let hash = sha256_hex(current.as_bytes());
        for recovered in result["revisions"]["recoveredTextSegments"]
            .as_array()
            .unwrap()
        {
            let comparison = json!({"id":row["id"],"fileName":row["fileName"],"revision":recovered["revisionNumber"],"identicalToCurrentText":recovered["recoveredTextSha256"]==hash,"currentTextSha256":hash,"historicalTextSha256":recovered["recoveredTextSha256"],"currentCharacters":current.chars().count(),"historicalCharacters":recovered["recoveredCharacterCount"],"pageErrors":errors,"historicalTruncated":result["revisions"]["analysisTruncated"]});
            println!("{comparison}");
            comparisons.push(comparison);
        }
    }
    assert!(!comparisons.is_empty());
    std::fs::write(
        root.join("revision-comparisons.json"),
        serde_json::to_vec_pretty(&comparisons).unwrap(),
    )
    .unwrap();
}
