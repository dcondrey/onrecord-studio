use super::super::extraction::{
    archive_entry_id, existing_container_manifest, validate_container_tree_budget_with_limits,
};
use super::*;

#[test]
fn derives_stable_javascript_safe_archive_entry_identifiers() {
    let digest = "a".repeat(64);
    let first = archive_entry_id(42, &digest, 1, Path::new("folder/evidence.txt"));
    let repeated = archive_entry_id(42, &digest, 1, Path::new("folder/evidence.txt"));
    let different = archive_entry_id(42, &digest, 1, Path::new("folder/other.txt"));

    assert_eq!(first, repeated);
    assert_ne!(first, different);
    assert!(first >= SYNTHETIC_DOCUMENT_NAMESPACE);
    assert!(first < (1_u64 << 53));
}

#[test]
fn reuses_only_regular_files_from_committed_container_directories() {
    let temporary = tempdir().unwrap();
    let nested = temporary.path().join("nested");
    std::fs::create_dir(&nested).unwrap();
    let first = temporary.path().join("one.txt");
    let second = nested.join("two.txt");
    std::fs::write(&first, b"one").unwrap();
    std::fs::write(&second, b"two").unwrap();

    assert_eq!(
        existing_container_manifest(temporary.path()).unwrap(),
        vec![second, first]
    );
}

#[test]
fn recursive_container_budget_measures_the_persisted_tree() {
    let fixture = tempdir().expect("fixture directory must open");
    let nested = fixture.path().join("nested");
    std::fs::create_dir(&nested).expect("nested fixture directory must be created");
    std::fs::write(fixture.path().join("first.bin"), b"abc")
        .expect("first fixture must be written");
    std::fs::write(nested.join("second.bin"), b"de").expect("second fixture must be written");

    assert_eq!(
        validate_container_tree_budget_with_limits(fixture.path(), 3, 5)
            .expect("tree at both limits must be accepted"),
        5
    );
    assert!(
        validate_container_tree_budget_with_limits(fixture.path(), 3, 4)
            .unwrap_err()
            .to_string()
            .contains("CONTAINER_TREE_SIZE_EXCEEDED")
    );
    assert!(
        validate_container_tree_budget_with_limits(fixture.path(), 2, 5)
            .unwrap_err()
            .to_string()
            .contains("CONTAINER_TREE_ENTRY_COUNT_EXCEEDED")
    );
}
