use super::super::extraction::{
    archive_entry_id, container_tree_limits, existing_container_manifest,
    validate_container_tree_budget_with_limits,
};
use super::super::objects::local_import_limit;
use super::*;

#[test]
fn derives_stable_javascript_safe_archive_entry_identifiers() {
    let digest = "a".repeat(64);
    let first = archive_entry_id(42, &digest, 1, Path::new("folder/evidence.txt"));
    let repeated = archive_entry_id(42, &digest, 1, Path::new("folder/evidence.txt"));
    let different = archive_entry_id(42, &digest, 1, Path::new("folder/other.txt"));

    assert_eq!(first, repeated);
    assert_ne!(first, different);
    // The same member gets the same identifier on every platform, and the value never drifts.
    assert_eq!(
        first,
        archive_entry_id(42, &digest, 1, &Path::new("folder").join("evidence.txt"))
    );
    assert_eq!(first, 6_017_228_482_553_958);
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
        existing_container_manifest(temporary.path(), MAXIMUM_CONTAINER_ENTRIES).unwrap(),
        vec![second, first]
    );
}

/// A mailbox the first run accepted must also be accepted from its cache and by its tree budget,
/// while an archive tree stays at the generic caps.
#[test]
fn a_mailbox_expansion_is_held_to_the_pst_caps_and_an_archive_to_the_generic_ones() {
    let temporary = tempdir().unwrap();
    let mailbox = temporary.path().join("7_0123456789abcdef_mailbox");
    let archive = temporary.path().join("7_0123456789abcdef_contents");
    for root in [&mailbox, &archive] {
        std::fs::create_dir(root).unwrap();
        for index in 0..=MAXIMUM_CONTAINER_ENTRIES {
            std::fs::write(root.join(format!("{index}.txt")), b"x").unwrap();
        }
    }
    assert_eq!(
        existing_container_manifest(&mailbox, MAXIMUM_PST_CONTAINER_ENTRIES)
            .unwrap()
            .len(),
        MAXIMUM_CONTAINER_ENTRIES + 1
    );
    assert!(existing_container_manifest(&archive, MAXIMUM_CONTAINER_ENTRIES).is_err());

    let (entries, bytes) = container_tree_limits(&mailbox);
    assert_eq!(
        (entries, bytes),
        (
            MAXIMUM_PST_CONTAINER_TREE_ENTRIES,
            MAXIMUM_PST_CONTAINER_TREE_BYTES
        )
    );
    assert!(validate_container_tree_budget_with_limits(&mailbox, entries, bytes).is_ok());
    for generic in [
        archive.as_path(),
        Path::new("/x/7_attachments"),
        Path::new("/x/mailbox"),
        Path::new("/x/7_mailbox_contents"),
    ] {
        assert_eq!(
            container_tree_limits(generic),
            (MAXIMUM_CONTAINER_TREE_ENTRIES, MAXIMUM_CONTAINER_TREE_BYTES),
            "{}",
            generic.display()
        );
    }
}

#[tokio::test]
async fn only_an_outlook_store_is_imported_past_the_generic_limit() {
    let temporary = tempdir().unwrap();
    let write = |name: &str, bytes: &[u8]| {
        let path = temporary.path().join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    };
    assert_eq!(
        local_import_limit(&write("mail.bin", b"!BDN\0\0\0\0SM\x17\0")).await,
        MAXIMUM_PST_LOCAL_IMPORT_BYTES
    );
    for (name, bytes) in [
        ("named.pst", &b"PK\x03\x04"[..]),
        ("short.pst", &b"!BD"[..]),
        ("empty.pst", &b""[..]),
    ] {
        assert_eq!(
            local_import_limit(&write(name, bytes)).await,
            MAXIMUM_LOCAL_IMPORT_BYTES,
            "{name}"
        );
    }
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
