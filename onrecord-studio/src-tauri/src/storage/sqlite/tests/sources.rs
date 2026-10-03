use super::*;

/// The page window and the connector recipe share a row and have separate writers, which is
/// the whole reason this can go wrong: an upsert that names every column would silently
/// replace whichever of the two it does not know about. The NextRequest path saves only a
/// window and the crawl path saves only a recipe, so each must leave the other's evidence of
/// what the operator chose exactly as it found it, in both orders.
#[tokio::test]
async fn a_page_window_and_a_connector_recipe_do_not_overwrite_each_other() {
    let guidance = SourceGuidance {
        allowed_hostnames: vec!["assets.example.test".to_string()],
        ..SourceGuidance::default()
    };
    let window = MetadataPageWindow {
        start_page: 400,
        page_size: 25,
    };

    for window_first in [true, false] {
        let (repository, registration) = repository_with_registered_portal().await;
        let save_window = |repository: DuckDbRepository, registration: PortalRegistration| async move {
            repository
                .save_source_metadata_page_window(
                    &registration,
                    &registration.canonical_url.clone(),
                    window,
                )
                .await
                .expect("the page window must save")
        };
        let save_recipe = |repository: DuckDbRepository,
                           registration: PortalRegistration,
                           guidance: SourceGuidance| async move {
            repository
                .save_source_connector_recipe(
                    &registration,
                    &registration.canonical_url.clone(),
                    &CrawlPolicy::default(),
                    &guidance,
                )
                .await
                .expect("the connector recipe must save")
        };
        if window_first {
            save_window(repository.clone(), registration.clone()).await;
            save_recipe(repository.clone(), registration.clone(), guidance.clone()).await;
        } else {
            save_recipe(repository.clone(), registration.clone(), guidance.clone()).await;
            save_window(repository.clone(), registration.clone()).await;
        }

        let loaded = repository
            .load_source_connector_recipe(RECIPE_ROOT_URL)
            .await
            .expect("the recipe must load")
            .expect("a row was written");
        assert_eq!(
            loaded.metadata_page_window,
            Some(window),
            "the saved page window must survive a recipe write (window first: {window_first})"
        );
        assert_eq!(
            loaded.guidance, guidance,
            "the saved guidance must survive a page window write (window first: {window_first})"
        );
        assert!(loaded.carries_guidance());
    }
}

/// A row that exists only to hold a page window must not be reported as saved guidance.
/// `SourceIngestionStart::used_saved_guidance` is a statement to the operator about how the
/// source was ingested, and deriving it from the row existing would make it a false one.
#[tokio::test]
async fn a_window_only_row_does_not_claim_saved_guidance() {
    let (repository, _registration) = repository_with_registered_portal().await;
    repository
        .save_source_metadata_page_window(
            &test_registration(PORTAL_SUBDOMAIN, RECIPE_ROOT_URL),
            RECIPE_ROOT_URL,
            MetadataPageWindow {
                start_page: 7,
                page_size: 10,
            },
        )
        .await
        .expect("the page window must save");

    let loaded = repository
        .load_source_connector_recipe(RECIPE_ROOT_URL)
        .await
        .expect("the recipe must load")
        .expect("a row was written");
    assert!(
        !loaded.carries_guidance(),
        "a window-only row carries no guidance, got {loaded:?}"
    );
}

/// A page window is looked up by the registration's canonical URL on every resume, and
/// `canonical_url` is `Url::to_string()`, which supplies the path a pasted bare host omits. A
/// window keyed on the pasted string is therefore a window no resume can ever find, which
/// silently reinstates the exact revert-to-1/50 this feature exists to prevent. The writer
/// takes the registration so the pasted string is not in reach; this pins the effect.
#[tokio::test]
async fn a_page_window_is_keyed_on_the_url_the_resume_paths_look_it_up_by() {
    let (repository, registration) = repository_with_registered_portal().await;
    let pasted = RECIPE_ROOT_URL
        .strip_suffix('/')
        .expect("the canonical form carries the path a paste omits");
    repository
        .save_source_metadata_page_window(
            &registration,
            &registration.canonical_url,
            MetadataPageWindow {
                start_page: 400,
                page_size: 25,
            },
        )
        .await
        .expect("the page window must save");

    let by_canonical = repository
        .load_source_connector_recipe(&registration.canonical_url)
        .await
        .expect("the recipe must load")
        .expect("the row the resume paths read must exist");
    assert_eq!(
        by_canonical.metadata_page_window,
        Some(MetadataPageWindow {
            start_page: 400,
            page_size: 25
        }),
        "a resume looks the recipe up by canonical_url and must find the saved window"
    );
    assert_ne!(
        pasted, registration.canonical_url,
        "the pasted form and the canonical form must actually differ, or this proves nothing"
    );
}

/// The guided-repair recipe carried the same key mismatch the page window did, and carried it
/// longer: the writer keyed the row on the operator's pasted string while both resume paths
/// queried `canonical_url`. Guidance therefore applied when the source was re-added and
/// stopped applying on every resume and every launch, with `used_saved_guidance` reporting it
/// accurately in the first case and not the others.
///
/// The rows already written that way cannot be found by any URL, so the resume paths match on
/// `source_key` instead: the one identifier that has always been written and read the same
/// way. This builds the legacy row directly, because a row written through the current writer
/// is not the shape an upgraded installation holds.
#[tokio::test]
async fn a_recipe_stored_under_a_pasted_url_still_applies_on_resume() {
    let (repository, registration) = repository_with_registered_portal().await;
    let pasted = RECIPE_ROOT_URL
        .strip_suffix('/')
        .expect("the canonical form carries the path a paste omits");
    assert_ne!(
        pasted, registration.canonical_url,
        "the pasted form and the canonical form must actually differ, or this proves nothing"
    );
    let guidance = SourceGuidance {
        allowed_hostnames: vec!["assets.example.test".to_string()],
        ..SourceGuidance::default()
    };
    {
        let conn = repository.conn.lock().await;
        conn.execute(
            "INSERT INTO source_connector_recipes (
                source_key, root_url, recipe_version, publisher_pattern, reuse_approved,
                crawl_policy_json, guidance_json, updated_at
             ) VALUES (?1, ?2, 2, ?3, 0, ?4, ?5, '2026-08-01T00:00:00.000Z')",
            params![
                PORTAL_SUBDOMAIN,
                pasted,
                "records.example.test",
                serde_json::to_string(&CrawlPolicy::default()).expect("policy encodes"),
                serde_json::to_string(&guidance).expect("guidance encodes"),
            ],
        )
        .expect("the legacy row must insert");
    }

    let readded = repository
        .load_source_connector_recipe(pasted)
        .await
        .expect("the recipe must load")
        .expect("a re-add must still find the row it found before the lookup normalized");
    assert_eq!(
        readded.guidance, guidance,
        "normalizing the lookup must not un-match the raw form a legacy row is stored under"
    );

    let resumed = repository
        .load_source_connector_recipe_for_source(&registration)
        .await
        .expect("the recipe must load")
        .expect("a resume must find the row the operator's guidance was written to");
    assert_eq!(
        resumed.guidance, guidance,
        "guidance saved before this fix must still apply on resume and at launch"
    );
}

/// `start_data_source` reads the recipe before discovery has run, so it has nothing but the
/// string the operator typed. That string is not what the row is keyed on, and the difference
/// is not exotic: a pasted bare host and the canonical `Url::to_string` differ by the path.
/// The lookup normalizes the way a submitted URL is normalized on the way in.
#[tokio::test]
async fn a_recipe_is_found_from_the_url_form_an_operator_actually_pastes() {
    let (repository, registration) = repository_with_registered_portal().await;
    let guidance = SourceGuidance {
        allowed_hostnames: vec!["assets.example.test".to_string()],
        ..SourceGuidance::default()
    };
    repository
        .save_source_connector_recipe(
            &registration,
            &registration.canonical_url,
            &CrawlPolicy::default(),
            &guidance,
        )
        .await
        .expect("the connector recipe must save");

    for pasted in ["records.example.test", "https://records.example.test"] {
        assert_ne!(
            pasted, registration.canonical_url,
            "each pasted form must differ from the stored one, or this proves nothing"
        );
        let loaded = repository
            .load_source_connector_recipe(pasted)
            .await
            .expect("the recipe must load")
            .expect("a re-add must find the guidance the operator supplied");
        assert_eq!(
            loaded.guidance, guidance,
            "guidance must apply when the source is re-added as '{pasted}'"
        );
    }
}

/// The one URL difference normalization cannot close. `start_data_source` reads the recipe
/// before discovery, so on a source that redirects it holds the string the operator typed
/// while the row is keyed on the URL the fetch resolved to. No rule applied to the submission
/// can produce the destination, which is why the submission is stored beside it rather than
/// derived at read time.
///
/// The two URLs deliberately differ by host as well as path, so the publisher-reuse arm
/// cannot be what finds the row. And the window is asserted, not just the guidance: a row
/// matched on `submitted_url` carries a `root_url` matching neither query form, so a lookup
/// that recognized its own row by the returned value would return the recipe and drop the
/// window.
#[tokio::test]
async fn a_recipe_is_found_from_the_url_that_redirected_to_the_one_it_is_keyed_on() {
    let (repository, registration) = repository_with_registered_portal().await;
    let submitted = "https://legacy.example.test/records";
    assert_ne!(
        normalized_submitted_url(submitted),
        registration.canonical_url,
        "the submitted form must not normalize to the canonical one, or this proves nothing"
    );
    assert_ne!(
        publisher_pattern_for_url(submitted),
        publisher_pattern_for_url(&registration.canonical_url),
        "the two URLs must not share a publisher, or the reuse arm could be what matches"
    );
    let guidance = SourceGuidance {
        allowed_hostnames: vec!["assets.example.test".to_string()],
        ..SourceGuidance::default()
    };
    let window = MetadataPageWindow {
        start_page: 400,
        page_size: 25,
    };
    repository
        .save_source_connector_recipe(&registration, submitted, &CrawlPolicy::default(), &guidance)
        .await
        .expect("the connector recipe must save");
    repository
        .save_source_metadata_page_window(&registration, submitted, window)
        .await
        .expect("the page window must save");

    let loaded = repository
        .load_source_connector_recipe(submitted)
        .await
        .expect("the recipe must load")
        .expect("a re-add of the URL the operator typed must find the row that URL wrote");
    assert_eq!(
        loaded.guidance, guidance,
        "guidance must apply when the source is re-added at the URL that redirects"
    );
    assert_eq!(
        loaded.metadata_page_window,
        Some(window),
        "a row matched on its submitted URL is still the caller's own row, and the window it \
         holds must come back with it"
    );
}

/// `submitted_url` carries no UNIQUE, and that is deliberate: a portal that moves resolves one
/// pasted string to a new canonical URL, which is a new `source_key` and so a second row
/// legitimately holding the same submission. The reader owes the operator the source they most
/// recently started at that URL, which is what `updated_at DESC` decides inside the own-row
/// rank. The superseded row's timestamp is written directly so the ordering is decided by the
/// data and not by how fast two writes happened to run.
#[tokio::test]
async fn two_sources_sharing_a_submitted_url_resolve_to_the_most_recent_one() {
    let (repository, superseded) = repository_with_registered_portal().await;
    let submitted = "https://legacy.example.test/records";
    let moved = test_registration("records-moved", "https://records-moved.example.test/");
    repository
        .insert_or_update_portal_registration(&moved)
        .await
        .expect("the moved registration must persist");
    let stale = SourceGuidance {
        allowed_hostnames: vec!["stale.example.test".to_string()],
        ..SourceGuidance::default()
    };
    let current = SourceGuidance {
        allowed_hostnames: vec!["current.example.test".to_string()],
        ..SourceGuidance::default()
    };
    repository
        .save_source_connector_recipe(&superseded, submitted, &CrawlPolicy::default(), &stale)
        .await
        .expect("the superseded recipe must save");
    {
        let conn = repository.conn.lock().await;
        conn.execute(
            "UPDATE source_connector_recipes
             SET updated_at = '2026-01-01T00:00:00.000000Z'
             WHERE source_key = ?1",
            params![superseded.portal_key],
        )
        .expect("the superseded row must be backdated");
    }
    repository
        .save_source_connector_recipe(&moved, submitted, &CrawlPolicy::default(), &current)
        .await
        .expect("the current recipe must save");

    let loaded = repository
        .load_source_connector_recipe(submitted)
        .await
        .expect("the recipe must load")
        .expect("one of the two rows holding the submission must answer");
    assert_eq!(
        loaded.guidance, current,
        "the most recently started source at a shared submitted URL is the one that answers"
    );
}

/// `root_url` is UNIQUE while both writers upsert on `source_key`, so a row holding a URL
/// under a superseded key fails the insert outright and takes the source start down with it.
/// The key is a digest over the source kind and the final URL, so a source that reclassifies
/// at an unchanged URL produces exactly that pair.
#[tokio::test]
async fn a_reclassified_source_replaces_the_recipe_row_holding_its_url() {
    let (repository, registration) = repository_with_registered_portal().await;
    repository
        .save_source_connector_recipe(
            &registration,
            &registration.canonical_url,
            &CrawlPolicy::default(),
            &SourceGuidance::default(),
        )
        .await
        .expect("the first recipe must save");
    let reclassified = PortalRegistration {
        portal_key: "source-reclassified".to_string(),
        ..test_registration("source-reclassified", RECIPE_ROOT_URL)
    };
    repository
        .insert_or_update_portal_registration(&reclassified)
        .await
        .expect("the reclassified registration must persist");
    let guidance = SourceGuidance {
        allowed_hostnames: vec!["assets.example.test".to_string()],
        ..SourceGuidance::default()
    };

    repository
        .save_source_connector_recipe(
            &reclassified,
            &reclassified.canonical_url,
            &CrawlPolicy::default(),
            &guidance,
        )
        .await
        .expect("a recipe for the same URL under a new key must not fail the start");

    let loaded = repository
        .load_source_connector_recipe_for_source(&reclassified)
        .await
        .expect("the recipe must load")
        .expect("the surviving row belongs to the current key");
    assert_eq!(loaded.guidance, guidance);
    assert!(
        repository
            .load_source_connector_recipe_for_source(&registration)
            .await
            .expect("the superseded lookup must not error")
            .is_none_or(|recipe| !recipe.carries_guidance()),
        "the superseded row must not survive holding the URL"
    );
}

/// The clearing delete and the upsert that replaces the row it removed share a transaction, so
/// a failed write cannot leave the source with no recipe at all. An unregistered key violates
/// the foreign key on `source_key`, which is a write that fails after the delete has run.
#[tokio::test]
async fn a_failed_recipe_write_does_not_destroy_the_recipe_it_would_have_replaced() {
    let (repository, registration) = repository_with_registered_portal().await;
    let guidance = SourceGuidance {
        allowed_hostnames: vec!["assets.example.test".to_string()],
        ..SourceGuidance::default()
    };
    repository
        .save_source_connector_recipe(
            &registration,
            &registration.canonical_url,
            &CrawlPolicy::default(),
            &guidance,
        )
        .await
        .expect("the standing recipe must save");
    let unregistered = test_registration("source-unregistered", RECIPE_ROOT_URL);

    repository
        .save_source_connector_recipe(
            &unregistered,
            &unregistered.canonical_url,
            &CrawlPolicy::default(),
            &SourceGuidance::default(),
        )
        .await
        .expect_err("a recipe for an unregistered source must not be accepted");

    let standing = repository
        .load_source_connector_recipe_for_source(&registration)
        .await
        .expect("the recipe must load")
        .expect("the standing row must survive a write that failed");
    assert_eq!(
        standing.guidance, guidance,
        "a failed write must not leave the source running under no guidance at all"
    );
}

/// A source running under a publisher recipe its operator approved for reuse has no row of its
/// own. Setting a page window gives it one, carrying default policy and guidance because a
/// NextRequest source has neither. That row must not be read as the operator withdrawing the
/// approved recipe, and its window must not travel back out to the siblings that share the
/// publisher: a window says where in *this* portal's index to start.
#[tokio::test]
async fn a_page_window_neither_withdraws_an_approved_publisher_recipe_nor_leaks_to_a_sibling() {
    let (repository, registration) = repository_with_registered_portal().await;
    let sibling = test_registration("records-archive", "https://records.example.test/archive");
    repository
        .insert_or_update_portal_registration(&sibling)
        .await
        .expect("the sibling registration must persist");
    let approved = SourceGuidance {
        reuse_for_publisher_pattern: true,
        allowed_hostnames: vec!["assets.example.test".to_string()],
        ..SourceGuidance::default()
    };
    repository
        .save_source_connector_recipe(
            &sibling,
            &sibling.canonical_url,
            &CrawlPolicy::default(),
            &approved,
        )
        .await
        .expect("the approved publisher recipe must save");
    let window = MetadataPageWindow {
        start_page: 400,
        page_size: 25,
    };
    repository
        .save_source_metadata_page_window(&registration, &registration.canonical_url, window)
        .await
        .expect("the page window must save");

    let loaded = repository
        .load_source_connector_recipe(&registration.canonical_url)
        .await
        .expect("the recipe must load")
        .expect("a row exists for this source");
    assert_eq!(
        loaded.guidance, approved,
        "setting a page window must not withdraw the approved publisher recipe"
    );
    assert_eq!(loaded.metadata_page_window, Some(window));

    let inherited = repository
        .load_source_connector_recipe(&sibling.canonical_url)
        .await
        .expect("the sibling recipe must load")
        .expect("the sibling has its own row");
    assert_eq!(
        inherited.metadata_page_window, None,
        "a page window belongs to one portal's request index, not to a shared publisher"
    );
}

/// Builds the pre-column table shape directly rather than stripping the column from a current
/// database, because a stripped column is not what an upgraded installation actually holds.
/// The recorded absence must stay an absence: reading it as page 1 at size 50 would report a
/// window the operator never chose, and 1 and 50 are legal windows an operator could choose.
#[tokio::test]
async fn a_recipe_written_before_the_page_window_column_reads_as_no_window() {
    let repository = DuckDbRepository::new(Path::new(":memory:")).expect("DuckDB core must open");
    {
        let connection = repository.conn.lock().await;
        connection
            .execute_batch(
                "CREATE TABLE portals (
                    subdomain TEXT PRIMARY KEY,
                    display_name TEXT NOT NULL,
                    base_url TEXT NOT NULL
                 );
                 CREATE TABLE source_connector_recipes (
                    source_key TEXT PRIMARY KEY,
                    root_url TEXT NOT NULL UNIQUE,
                    recipe_version BIGINT NOT NULL DEFAULT 2,
                    crawl_policy_json TEXT NOT NULL,
                    guidance_json TEXT NOT NULL,
                    updated_at TEXT NOT NULL DEFAULT '2026-01-01T00:00:00.000000Z',
                    FOREIGN KEY (source_key) REFERENCES portals(subdomain)
                 );",
            )
            .expect("the pre-column shape must be reproducible");
        connection
            .execute(
                "INSERT INTO portals (subdomain, display_name, base_url)
                 VALUES (?1, 'Records', ?2)",
                params![PORTAL_SUBDOMAIN, RECIPE_ROOT_URL],
            )
            .expect("the legacy portal row must insert");
        connection
            .execute(
                "INSERT INTO source_connector_recipes
                    (source_key, root_url, crawl_policy_json, guidance_json)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    PORTAL_SUBDOMAIN,
                    RECIPE_ROOT_URL,
                    serde_json::to_string(&CrawlPolicy::default()).expect("policy encodes"),
                    serde_json::to_string(&SourceGuidance::default()).expect("guidance encodes")
                ],
            )
            .expect("the legacy recipe row must insert");
    }

    repository
        .run_migrations(MIGRATION)
        .await
        .expect("an upgraded core must migrate");

    let loaded = repository
        .load_source_connector_recipe(RECIPE_ROOT_URL)
        .await
        .expect("the upgraded recipe must load")
        .expect("the legacy row survives the upgrade");
    assert_eq!(
        loaded.metadata_page_window, None,
        "a recipe that predates the column recorded no window, got {loaded:?}"
    );
}

#[test]
fn publisher_recipe_patterns_are_exact_normalized_hostnames() {
    assert_eq!(
        publisher_pattern_for_url("https://Data.Example.Gov./records/a"),
        Some("data.example.gov".to_string())
    );
    assert_eq!(
        publisher_pattern_for_url("https://other.example.gov/records/a"),
        Some("other.example.gov".to_string())
    );
    assert_eq!(publisher_pattern_for_url("not a URL"), None);
}

/// A cancelled run was stopped by the operator or by a failed start, so it is no evidence about
/// the source: a paused healthy source must not turn degraded, and a degraded one must not be
/// cleared, by cancelling.
#[tokio::test]
async fn a_cancelled_sync_run_leaves_source_health_as_it_was() {
    let repository = repository_with_document().await;
    let mut observed = Vec::new();
    for (run_id, status) in [
        ("run-completed", "COMPLETED"),
        ("run-paused", "CANCELLED"),
        ("run-with-errors", "COMPLETED_WITH_ERRORS"),
        ("run-paused-again", "CANCELLED"),
    ] {
        repository
            .begin_portal_sync_run(run_id, PORTAL_SUBDOMAIN)
            .await
            .expect("sync run should begin");
        if status == "COMPLETED_WITH_ERRORS" {
            repository
                .record_source_task_failure(
                    run_id,
                    PORTAL_SUBDOMAIN,
                    "listing",
                    &AppError::PortalSecurity("host answered HTTP 403".to_string()),
                )
                .await
                .expect("a source-level failure should record");
        }
        repository
            .finish_portal_sync_run(PortalSyncRunFinalization {
                run_id,
                portal_key: PORTAL_SUBDOMAIN,
                status,
                pages_processed: 0,
                requests_processed: 0,
                documents_processed: 0,
                failures: 0,
            })
            .await
            .expect("sync run should finalize");
        let connection = repository.conn.lock().await;
        let health: String = connection
            .query_row(
                "SELECT health_status FROM portals WHERE subdomain = ?1",
                params![PORTAL_SUBDOMAIN],
                |row| row.get(0),
            )
            .expect("portal health should be readable");
        observed.push(health);
    }
    assert_eq!(observed, ["verified", "verified", "degraded", "degraded"]);
}

/// One unreadable document or dead link is that item's failure. Flagging the whole source for
/// attention over it showed a badge whose only reason was a single file, on every source with
/// any such file. The failure is still recorded for the run.
#[tokio::test]
async fn document_failures_alone_do_not_flag_the_source_for_attention() {
    let repository = repository_with_document().await;
    repository
        .begin_portal_sync_run("run-items", PORTAL_SUBDOMAIN)
        .await
        .unwrap();
    for (target, error) in [
        ("scan.pdf", AppError::DataParsing("OCR_OUTPUT_EMPTY".to_string())),
        ("gone", AppError::PortalDiscovery("HTTP 404".to_string())),
    ] {
        repository
            .record_source_task_failure("run-items", PORTAL_SUBDOMAIN, target, &error)
            .await
            .unwrap();
    }
    repository
        .finish_portal_sync_run(PortalSyncRunFinalization {
            run_id: "run-items",
            portal_key: PORTAL_SUBDOMAIN,
            status: "COMPLETED_WITH_ERRORS",
            pages_processed: 1,
            requests_processed: 1,
            documents_processed: 3,
            failures: 2,
        })
        .await
        .unwrap();
    let connection = repository.conn.lock().await;
    let (health, last_error, recorded): (String, Option<String>, i64) = connection
        .query_row(
            "SELECT health_status, last_error,
                    (SELECT COUNT(*) FROM source_ingestion_failures WHERE run_id = 'run-items')
             FROM portals WHERE subdomain = ?1",
            params![PORTAL_SUBDOMAIN],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(health, "verified");
    assert_eq!(last_error, None);
    assert_eq!(recorded, 2, "item failures are still recorded for the run");
    drop(connection);

    // The same failures with nothing collected are not a healthy source, and the reason shows.
    repository
        .begin_portal_sync_run("run-nothing", PORTAL_SUBDOMAIN)
        .await
        .unwrap();
    repository
        .record_source_task_failure(
            "run-nothing",
            PORTAL_SUBDOMAIN,
            "scan.pdf",
            &AppError::DataParsing("OCR_OUTPUT_EMPTY".to_string()),
        )
        .await
        .unwrap();
    repository
        .finish_portal_sync_run(PortalSyncRunFinalization {
            run_id: "run-nothing",
            portal_key: PORTAL_SUBDOMAIN,
            status: "COMPLETED_WITH_ERRORS",
            pages_processed: 1,
            requests_processed: 1,
            documents_processed: 0,
            failures: 1,
        })
        .await
        .unwrap();
    let connection = repository.conn.lock().await;
    let (health, last_error): (String, Option<String>) = connection
        .query_row(
            "SELECT health_status, last_error FROM portals WHERE subdomain = ?1",
            params![PORTAL_SUBDOMAIN],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(health, "degraded");
    assert!(
        last_error.as_deref().is_some_and(|reason| reason.contains("OCR_OUTPUT_EMPTY")),
        "a source flagged for attention must say why, got {last_error:?}"
    );
}
