use super::matching::{
    find_candidates, normalize_name, resolve_confirmed_subjects, AgencyTarget, ConfirmedSubjects,
    PortalTarget, ENTITY_SEARCH_LIMIT,
};
use super::store::{write_atomic_with, ProfileStore, PROFILE_FILE_NAME};
use super::*;
use crate::analysis::finding::{Finding, FindingSubject};
use crate::storage::corpus::{CorpusDocumentInput, CorpusIndex};
use crate::storage::sqlite::DuckDbRepository;
use crate::types::{CorpusEntityKind, CorpusEntityNode, EntityValidationGrade};
use std::io::Write;

fn input(name: &str, aliases: &[&str]) -> AffiliationInput {
    AffiliationInput {
        id: None,
        name: name.to_string(),
        aliases: aliases.iter().map(|alias| alias.to_string()).collect(),
    }
}

fn profile_input(affiliations: Vec<AffiliationInput>) -> ProfileInput {
    ProfileInput {
        display_name: Some("Reporter".to_string()),
        affiliations,
    }
}

fn rejected(result: Result<UserProfile, AppError>) -> bool {
    matches!(result, Err(AppError::DataParsing(_)))
}

/// `length` characters in short words: the resolver drops a single token longer than its token
/// limit, which would leave a name with nothing to compare.
fn word(length: usize) -> String {
    (0..length)
        .map(|index| {
            if index % 10 == 9 && index + 1 != length {
                ' '
            } else {
                'a'
            }
        })
        .collect()
}

fn declared(name: &str) -> UserProfile {
    apply_input(None, profile_input(vec![input(name, &["City Police"])])).unwrap()
}

fn entity(id: &str, display: &str, grade: EntityValidationGrade) -> CorpusEntityNode {
    CorpusEntityNode {
        entity_id: id.to_string(),
        kind: CorpusEntityKind::Organization,
        display_value: display.to_string(),
        document_count: 7,
        validation_grade: grade,
    }
}

fn answer_kind(profile: UserProfile, name: &str, kind: AffiliationMatchKind) -> UserProfile {
    let affiliation_id = profile.affiliations[0].id.clone();
    apply_decision(
        profile,
        AffiliationMatchDecisionInput {
            affiliation_id,
            kind,
            normalized_name: name.to_string(),
        },
        MatchDecisionKind::Confirmed,
    )
    .unwrap()
}

fn answer(profile: UserProfile, candidate_name: &str, kind: MatchDecisionKind) -> UserProfile {
    let affiliation_id = profile.affiliations[0].id.clone();
    apply_decision(
        profile,
        AffiliationMatchDecisionInput {
            affiliation_id,
            kind: AffiliationMatchKind::Organization,
            normalized_name: candidate_name.to_string(),
        },
        kind,
    )
    .unwrap()
}

#[test]
fn every_limit_accepts_the_limit_and_rejects_one_past_it() {
    let with_display = |length: usize| {
        apply_input(
            None,
            ProfileInput {
                display_name: Some(word(length)),
                affiliations: vec![],
            },
        )
    };
    assert!(with_display(MAXIMUM_DISPLAY_NAME_CHARACTERS - 1).is_ok());
    assert!(with_display(MAXIMUM_DISPLAY_NAME_CHARACTERS).is_ok());
    assert!(rejected(with_display(MAXIMUM_DISPLAY_NAME_CHARACTERS + 1)));

    let with_count = |count: usize| {
        apply_input(
            None,
            profile_input(
                (0..count)
                    .map(|index| input(&format!("Agency {index}"), &[]))
                    .collect(),
            ),
        )
    };
    assert!(with_count(MAXIMUM_AFFILIATIONS - 1).is_ok());
    assert!(with_count(MAXIMUM_AFFILIATIONS).is_ok());
    assert!(rejected(with_count(MAXIMUM_AFFILIATIONS + 1)));

    let with_name =
        |length: usize| apply_input(None, profile_input(vec![input(&word(length), &[])]));
    assert!(with_name(MAXIMUM_AFFILIATION_NAME_CHARACTERS - 1).is_ok());
    assert!(with_name(MAXIMUM_AFFILIATION_NAME_CHARACTERS).is_ok());
    assert!(rejected(with_name(MAXIMUM_AFFILIATION_NAME_CHARACTERS + 1)));

    let with_aliases = |count: usize| {
        let aliases: Vec<String> = (0..count).map(|index| format!("alias {index}")).collect();
        let refs: Vec<&str> = aliases.iter().map(String::as_str).collect();
        apply_input(None, profile_input(vec![input("Records Office", &refs)]))
    };
    assert!(with_aliases(MAXIMUM_ALIASES_PER_AFFILIATION - 1).is_ok());
    assert!(with_aliases(MAXIMUM_ALIASES_PER_AFFILIATION).is_ok());
    assert!(rejected(with_aliases(MAXIMUM_ALIASES_PER_AFFILIATION + 1)));

    let with_alias = |length: usize| {
        apply_input(
            None,
            profile_input(vec![input("Records Office", &[&word(length)])]),
        )
    };
    assert!(with_alias(MAXIMUM_ALIAS_CHARACTERS - 1).is_ok());
    assert!(with_alias(MAXIMUM_ALIAS_CHARACTERS).is_ok());
    assert!(rejected(with_alias(MAXIMUM_ALIAS_CHARACTERS + 1)));
}

#[test]
fn text_is_trimmed_and_control_characters_and_normalized_duplicates_are_rejected() {
    let saved = apply_input(
        None,
        ProfileInput {
            display_name: Some("   ".to_string()),
            affiliations: vec![input("  Records Office  ", &["  RO  "])],
        },
    )
    .unwrap();
    assert_eq!(
        saved.display_name, None,
        "a blank name is absent, not empty"
    );
    assert_eq!(saved.affiliations[0].name, "Records Office");
    assert_eq!(saved.affiliations[0].aliases, ["RO"]);
    assert_eq!(saved.affiliations[0].basis, AffiliationBasis::SelfDeclared);

    assert!(rejected(apply_input(
        None,
        profile_input(vec![input("Records\nOffice", &[])])
    )));
    assert!(rejected(apply_input(
        None,
        profile_input(vec![input("Records Office", &["R\u{0}O"])])
    )));
    assert!(rejected(apply_input(
        None,
        profile_input(vec![
            input("Records Office", &[]),
            input("records  OFFICE.", &[])
        ])
    )));
    assert!(rejected(apply_input(
        None,
        profile_input(vec![input("Records Office", &["RO", "ro"])])
    )));
    assert!(rejected(apply_input(
        None,
        profile_input(vec![input("Records Office", &["records office"])])
    )));
    assert!(rejected(apply_input(
        None,
        profile_input(vec![input("!!!", &[])])
    )));
}

#[test]
fn renaming_an_affiliation_drops_its_decisions_but_editing_aliases_keeps_them() {
    let profile = answer(
        declared("Police Department"),
        "police department",
        MatchDecisionKind::Confirmed,
    );
    let id = profile.affiliations[0].id.clone();
    let keep = apply_input(
        Some(profile.clone()),
        profile_input(vec![AffiliationInput {
            id: Some(id.clone()),
            name: "Police Department".into(),
            aliases: vec!["PD".into()],
        }]),
    )
    .unwrap();
    assert_eq!(keep.match_decisions.len(), 1);
    assert_eq!(keep.affiliations[0].id, id);
    assert_eq!(
        keep.affiliations[0].created_at,
        profile.affiliations[0].created_at
    );

    let renamed = apply_input(
        Some(keep),
        profile_input(vec![AffiliationInput {
            id: Some(id),
            name: "Fire Department".into(),
            aliases: vec![],
        }]),
    )
    .unwrap();
    assert!(renamed.match_decisions.is_empty());
}

#[test]
fn the_profile_file_never_holds_an_entity_id_and_delete_returns_to_unset() {
    let directory = tempfile::tempdir().unwrap();
    let store = ProfileStore::new(directory.path());
    assert_eq!(store.load().unwrap(), None, "no file is the unset state");

    let entities = vec![entity(
        "ENTITY-ID-9f3a",
        "City Police Department",
        EntityValidationGrade::GazetteerHit,
    )];
    let profile = declared("City Police Department");
    let candidates = find_candidates(&profile, &[], &[], &entities);
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].entity_id.as_deref(), Some("ENTITY-ID-9f3a"));

    store
        .update(|_| {
            Ok(answer(
                profile,
                &candidates[0].normalized_name,
                MatchDecisionKind::Confirmed,
            ))
        })
        .unwrap();
    let text = std::fs::read_to_string(directory.path().join(PROFILE_FILE_NAME)).unwrap();
    assert!(!text.contains("ENTITY-ID-9f3a"));
    assert!(!text.to_lowercase().contains("entityid") && !text.contains("entity_id"));
    assert!(text.contains("city police department"));
    assert_eq!(store.load().unwrap().unwrap().match_decisions.len(), 1);

    assert_eq!(store.delete().unwrap(), ProfileDeletionOutcome::Removed);
    assert!(!directory.path().join(PROFILE_FILE_NAME).exists());
    assert_eq!(store.load().unwrap(), None);
    assert_eq!(
        store.delete().unwrap(),
        ProfileDeletionOutcome::NothingToRemove
    );
}

#[test]
fn a_failed_write_leaves_the_previous_file_intact_and_no_temp_file_behind() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join(PROFILE_FILE_NAME);
    store::write_atomic(&path, b"old contents").unwrap();

    let failure = write_atomic_with(&path, |file| {
        file.write_all(b"half of the new")?;
        Err(std::io::Error::other("disk full"))
    });
    assert!(failure.is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"old contents");
    let names: Vec<_> = std::fs::read_dir(directory.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(names, [std::ffi::OsString::from(PROFILE_FILE_NAME)]);
}

#[test]
fn a_corrupt_file_is_an_error_not_unset_and_delete_recovers_from_it() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join(PROFILE_FILE_NAME);
    std::fs::write(&path, b"{ not json").unwrap();
    let store = ProfileStore::new(directory.path());
    assert!(store.load().is_err());
    assert!(store.update(|_| Ok(declared("Records Office"))).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"{ not json");
    assert_eq!(store.delete().unwrap(), ProfileDeletionOutcome::Removed);
    assert_eq!(store.load().unwrap(), None);
}

#[test]
fn matching_reports_how_each_candidate_matched_and_never_applies_one() {
    let profile = declared("City Police Department");
    let entities = vec![
        entity(
            "a",
            "City Police Department",
            EntityValidationGrade::GazetteerHit,
        ),
        entity("b", "City Police", EntityValidationGrade::GazetteerHit),
        entity(
            "c",
            "City Police Departmnt",
            EntityValidationGrade::PatternOnly,
        ),
        entity("d", "Water Board", EntityValidationGrade::GazetteerHit),
    ];
    let portals = vec![PortalTarget {
        portal_key: "cityportal".into(),
        display_name: "City Police Department".into(),
    }];
    let candidates = find_candidates(&profile, &portals, &[], &entities);
    let method_of = |id: &str| {
        candidates
            .iter()
            .find(|candidate| candidate.entity_id.as_deref() == Some(id))
            .map(|candidate| candidate.method)
    };
    assert_eq!(method_of("a"), Some(AffiliationMatchMethod::Exact));
    assert_eq!(method_of("b"), Some(AffiliationMatchMethod::Alias));
    assert_eq!(method_of("c"), Some(AffiliationMatchMethod::Fuzzy));
    assert_eq!(method_of("d"), None);
    let portal = candidates
        .iter()
        .find(|candidate| candidate.portal_key.as_deref() == Some("cityportal"))
        .unwrap();
    assert_eq!(
        portal.validation_grade, None,
        "a registered portal was not extracted"
    );
    assert!(candidates
        .iter()
        .all(|candidate| candidate.decision == CandidateDecision::Pending));

    let none = find_candidates(&profile, &[], &[], &[entities[3].clone()]);
    assert!(none.is_empty());
}

#[test]
fn decisions_survive_a_corpus_rebuild_because_they_are_keyed_by_name() {
    let profile = answer(
        declared("City Police Department"),
        "city police department",
        MatchDecisionKind::Confirmed,
    );
    let before = vec![entity(
        "old-id",
        "City Police Department",
        EntityValidationGrade::GazetteerHit,
    )];
    let after = vec![entity(
        "new-id",
        "City Police Department",
        EntityValidationGrade::GazetteerHit,
    )];
    let ids = resolve_confirmed_subjects(&profile, &[], &[], &before).entity_ids;
    assert_eq!(ids.into_iter().collect::<Vec<_>>(), ["old-id"]);
    let ids = resolve_confirmed_subjects(&profile, &[], &[], &after).entity_ids;
    assert_eq!(ids.into_iter().collect::<Vec<_>>(), ["new-id"]);
    let candidates = find_candidates(&profile, &[], &[], &after);
    assert_eq!(candidates[0].decision, CandidateDecision::Confirmed);

    let gone = find_candidates(&profile, &[], &[], &[]);
    assert_eq!(gone[0].method, AffiliationMatchMethod::PreviouslyDecided);
    assert!(!gone[0].currently_present);
}

#[test]
fn the_name_normalization_agrees_with_what_the_corpus_resolver_stores() {
    assert_eq!(
        normalize_name("  San Diego  Police-Department. "),
        "san diego police department"
    );
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let index = CorpusIndex::open(directory.path().join("corpus.db")).unwrap();
        let text = "The San Diego Police Department released the log. The San Diego Police Department \
                    also released the report.";
        index
            .index_document(CorpusDocumentInput {
                portal_key: "records".into(),
                request_id: 1,
                document_id: 1,
                raw_file_sha256: format!("{:064x}", 1),
                extracted_text_sha256: format!("{:064x}", 2),
                text: text.into(),
            })
            .await
            .unwrap();
        let entities = index.top_entities(ENTITY_SEARCH_LIMIT).await.unwrap();
        let profile = declared("san diego police department");
        let candidates = find_candidates(&profile, &[], &[], &entities);
        assert!(
            candidates
                .iter()
                .any(|candidate| candidate.method == AffiliationMatchMethod::Exact
                    && candidate.validation_grade.is_some()),
            "an organization the resolver extracted must match its own normalized form: {entities:?}"
        );
    });
}

#[test]
fn a_confirmed_organization_resolves_however_rarely_it_is_mentioned() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let index = CorpusIndex::open(directory.path().join("corpus.db")).unwrap();
        let documents = [
            "The San Diego Police Department released the log.",
            "The Los Angeles Police Department released the log.",
            "The Los Angeles Police Department released the report.",
            "The Los Angeles Police Department released the memo.",
        ];
        for (id, text) in (1_u64..).zip(documents) {
            index
                .index_document(CorpusDocumentInput {
                    portal_key: "records".into(),
                    request_id: id,
                    document_id: id,
                    raw_file_sha256: format!("{:064x}", id),
                    extracted_text_sha256: format!("{:064x}", id + 100),
                    text: text.into(),
                })
                .await
                .unwrap();
        }
        let top = index.top_entities(1).await.unwrap();
        let named = index
            .organization_entity_ids_named(["the san diego police department".to_string()].into())
            .await
            .unwrap();
        assert_eq!(named.len(), 1, "{top:?}");
        assert!(
            !top.iter().any(|entity| named.contains(&entity.entity_id)),
            "the lookup must reach past the ranking: {top:?}"
        );
    });
}

#[test]
fn the_disclosure_is_the_standard_sentence_and_absent_when_unset() {
    assert_eq!(disclosure_sentence(None), None);
    let empty = apply_input(None, profile_input(vec![])).unwrap();
    assert_eq!(disclosure_sentence(Some(&empty)), None);
    assert_eq!(
        build_disclosure(None, ConfirmedSubjects::default()).status,
        DisclosureStatus::Unset
    );

    let one = apply_input(None, profile_input(vec![input("City Clerk", &[])])).unwrap();
    assert_eq!(
        disclosure_sentence(Some(&one)).as_deref(),
        Some("Prepared by a user who has declared an affiliation with City Clerk (self-declared, not verified).")
    );
    let two = apply_input(
        None,
        profile_input(vec![input("City Clerk", &[]), input("Water Board", &[])]),
    )
    .unwrap();
    assert!(disclosure_sentence(Some(&two))
        .unwrap()
        .contains("City Clerk and Water Board"));
}

trait Rendered {
    fn rendered(&self, subject: &FindingSubject) -> String;
}

impl Rendered for DisclosureContext {
    fn rendered(&self, subject: &FindingSubject) -> String {
        self.text(subject).unwrap_or_default()
    }
}

fn subject(portal: Option<&str>, agency: Option<i64>, entity: Option<&str>) -> FindingSubject {
    FindingSubject {
        portal_key: portal.map(str::to_string),
        agency_id: agency,
        request_id: None,
        document_id: None,
        entity_id: entity.map(str::to_string),
    }
}

#[test]
fn a_footer_needs_a_declaration_and_names_an_own_subject_only_when_confirmed() {
    assert_eq!(
        DisclosureContext::unset().rendered(&subject(Some("p"), None, None)),
        ""
    );
    let own = ConfirmedSubjects {
        portal_keys: ["clerk-portal".to_string()].into(),
        agencies: [("shared-portal".to_string(), 4)].into(),
        entity_ids: ["ent-1".to_string()].into(),
    };
    let mut disclosure = build_disclosure(Some(&declared("City Clerk")), own);
    let context = DisclosureContext::from_disclosure(&disclosure);
    let other = context.rendered(&subject(Some("other"), None, None));
    assert!(other.contains("declared an affiliation with City Clerk"));
    assert!(!other.contains("subject of this document"));
    for own_subject in [
        subject(Some("clerk-portal"), None, None),
        subject(None, None, Some("ent-1")),
        subject(Some("shared-portal"), Some(4), None),
    ] {
        assert!(context
            .rendered(&own_subject)
            .contains("subject of this document"));
    }
    // An agency id is unique only within its portal.
    assert!(!context
        .rendered(&subject(Some("shared-portal"), Some(5), None))
        .contains("subject of this document"));
    assert!(!context
        .rendered(&subject(Some("other"), Some(4), None))
        .contains("subject of this document"));

    disclosure.sentence = None;
    let undeclared = DisclosureContext::from_disclosure(&disclosure);
    assert_eq!(
        undeclared.rendered(&subject(Some("clerk-portal"), None, None)),
        ""
    );
}

#[test]
fn an_agency_matches_by_name_and_resolves_to_its_portal_scoped_id() {
    let profile = answer_kind(
        declared("Police Department"),
        "police department",
        AffiliationMatchKind::Agency,
    );
    let agencies = [
        AgencyTarget {
            portal_key: "a".into(),
            agency_id: 1,
            name: "Police Department".into(),
        },
        AgencyTarget {
            portal_key: "b".into(),
            agency_id: 1,
            name: "Parks".into(),
        },
    ];
    let candidates = find_candidates(&profile, &[], &agencies, &[]);
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].kind, AffiliationMatchKind::Agency);
    assert_eq!(candidates[0].agency_id, Some(1));
    assert_eq!(candidates[0].decision, CandidateDecision::Confirmed);
    let own = resolve_confirmed_subjects(&profile, &[], &agencies, &[]);
    assert_eq!(
        own.agencies.into_iter().collect::<Vec<_>>(),
        [("a".to_string(), 1)]
    );
}

pub(crate) async fn seeded_repository() -> DuckDbRepository {
    let repository = DuckDbRepository::new(":memory:").unwrap();
    repository
        .run_migrations(include_str!("../../migrations/0001_initial_schema.sql"))
        .await
        .unwrap();
    // Twenty extended requests for one agency, all on one ground, each with a public document:
    // enough for the extension-abuse detector to fire.
    repository
        .query_connection("profile fixture", |connection| {
            let mut sql = String::from(
                "INSERT INTO portals (subdomain, display_name, base_url) VALUES ('clerk-portal', 'Clerk Portal', 'https://clerk.example');
                 INSERT INTO agencies (id, portal_subdomain, name) VALUES (3, 'clerk-portal', 'City Clerk');",
            );
            for id in 1..=20 {
                sql.push_str(&format!(
                    "INSERT INTO requests (id, portal_subdomain, title, status, created_at, updated_at, agency_id)
                       VALUES ({id}, 'clerk-portal', 'Request {id}', 'open', '2026-01-01T00:00:00Z', '2026-01-02T00:00:00Z', 3);
                     INSERT INTO statutory_extensions (request_id, portal_subdomain, notice_at, expected_determination_at, reason, notice_source)
                       VALUES ({id}, 'clerk-portal', '2026-01-{day:02}T00:00:00Z', '2026-02-01T00:00:00Z', 'voluminous', 'portal');
                     INSERT INTO documents (id, request_id, portal_subdomain, visibility, created_at, updated_at)
                       VALUES ({id}, {id}, 'clerk-portal', 'public', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');",
                    day = id,
                ));
            }
            connection
                .execute_batch(&sql)
                .map_err(|error| AppError::Storage(format!("fixture: {error}")))
        })
        .await
        .unwrap();
    repository
}

pub(crate) async fn snapshot(repository: &DuckDbRepository) -> Vec<Finding> {
    let mut findings = repository
        .list_findings(
            crate::storage::findings::FindingsFilter {
                include_dismissed: Some(true),
                ..Default::default()
            },
            500,
        )
        .await
        .unwrap();
    findings.sort_by(|left, right| left.id.cmp(&right.id));
    findings
}

#[test]
fn findings_are_identical_with_and_without_a_profile() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let repository = seeded_repository().await;
        let first = crate::analysis::detectors::refresh_all_findings(&repository)
            .await
            .unwrap();
        assert!(
            first.findings_recorded >= 1,
            "the fixture must make a detector fire"
        );
        let before = snapshot(&repository).await;
        assert!(!before.is_empty());

        // A profile whose confirmed match is exactly this finding's agency.
        let directory = tempfile::tempdir().unwrap();
        let store = ProfileStore::new(directory.path());
        store
            .update(|_| {
                Ok(answer_kind(
                    declared("City Clerk"),
                    "city clerk",
                    AffiliationMatchKind::Agency,
                ))
            })
            .unwrap();
        let profile = store.load().unwrap().unwrap();
        let agencies = [AgencyTarget {
            portal_key: "clerk-portal".into(),
            agency_id: 3,
            name: "City Clerk".into(),
        }];
        let own = resolve_confirmed_subjects(&profile, &[], &agencies, &[]);
        assert_eq!(
            own.agencies.len(),
            1,
            "the profile does match this finding's subject"
        );
        assert!(before
            .iter()
            .any(|finding| finding.subject.agency_id == Some(3)));

        crate::analysis::detectors::refresh_all_findings(&repository)
            .await
            .unwrap();
        let after = snapshot(&repository).await;
        let key = |finding: &Finding| {
            (
                finding.id.clone(),
                finding.kind,
                finding.severity,
                finding.confidence,
                finding.headline.clone(),
                finding.explanation.clone(),
                finding.subject.clone(),
                finding.citations.clone(),
            )
        };
        assert_eq!(
            before.iter().map(key).collect::<Vec<_>>(),
            after.iter().map(key).collect::<Vec<_>>()
        );
    });
}

#[test]
fn nothing_that_produces_findings_reads_the_profile() {
    fn scan(directory: &std::path::Path, hits: &mut Vec<String>) {
        for entry in std::fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                scan(&path, hits);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                let source = std::fs::read_to_string(&path).unwrap();
                let production = source.split("#[cfg(test)]").next().unwrap_or("");
                if production.contains("crate::profile") {
                    hits.push(path.display().to_string());
                }
            }
        }
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut hits = Vec::new();
    for directory in ["analysis", "storage"] {
        scan(&root.join(directory), &mut hits);
    }
    hits.retain(|path| !path.contains("analysis/artifacts"));
    assert!(hits.is_empty(), "these read the profile: {hits:?}");
}
