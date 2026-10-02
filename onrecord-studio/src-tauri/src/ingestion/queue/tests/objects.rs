use super::super::objects::{
    persist_download, sanitize_file_name, validate_cached_payload,
    validate_internal_output_directory,
};
use super::*;

#[test]
fn sanitizes_names_to_a_portable_byte_bound() {
    assert_eq!(
        sanitize_file_name("../../evidence:final.pdf", 7),
        "evidence_final.pdf"
    );
    assert_eq!(sanitize_file_name("...", 7), "document_7.bin");

    let unicode_name = format!("{}.txt", "🔎".repeat(100));
    let sanitized = sanitize_file_name(&unicode_name, 9);
    assert!(sanitized.len() <= 180);
    assert!(sanitized.is_char_boundary(sanitized.len()));
}

#[test]
fn rejects_output_paths_outside_the_managed_root() {
    let root = Path::new("/managed/documents");
    assert!(validate_internal_output_directory(root, &root.join("portal")).is_ok());
    assert!(validate_internal_output_directory(root, Path::new("/managed/escape")).is_err());
    assert!(validate_internal_output_directory(root, &root.join("../escape")).is_err());
}

#[tokio::test]
async fn commits_downloads_idempotently_and_detects_collisions() {
    let temporary = tempdir().unwrap();
    let output = temporary.path().join("portal");
    let first = persist_download(&output, 42, "evidence.txt", b"first")
        .await
        .unwrap();
    let repeated = persist_download(&output, 42, "evidence.txt", b"first")
        .await
        .unwrap();
    assert_eq!(first, repeated);
    assert_eq!(tokio::fs::read(&first).await.unwrap(), b"first");

    let collision = persist_download(&output, 42, "evidence.txt", b"different").await;
    assert!(collision.is_err());
    assert_eq!(tokio::fs::read(&first).await.unwrap(), b"first");
}

#[tokio::test]
async fn cached_payloads_must_be_regular_managed_files() {
    let managed = tempdir().unwrap();
    let outside = tempdir().unwrap();
    let managed_file = managed.path().join("cached.txt");
    let outside_file = outside.path().join("outside.txt");
    tokio::fs::write(&managed_file, b"cached").await.unwrap();
    tokio::fs::write(&outside_file, b"outside").await.unwrap();

    assert_eq!(
        validate_cached_payload(managed.path(), &managed_file, Some(6))
            .await
            .unwrap()
            .1,
        6
    );
    assert!(validate_cached_payload(managed.path(), &outside_file, None)
        .await
        .is_err());
    assert!(
        validate_cached_payload(managed.path(), &managed_file, Some(99))
            .await
            .is_err()
    );
}

#[cfg(unix)]
#[tokio::test]
async fn cached_payloads_reject_symbolic_links() {
    let managed = tempdir().unwrap();
    let target = managed.path().join("target.txt");
    let link = managed.path().join("link.txt");
    tokio::fs::write(&target, b"target").await.unwrap();
    std::os::unix::fs::symlink(&target, &link).unwrap();

    assert!(validate_cached_payload(managed.path(), &link, None)
        .await
        .is_err());
}
