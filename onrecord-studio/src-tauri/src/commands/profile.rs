//! IPC commands for the local profile, affiliation matching and the disclosure.
//!
//! The profile is read here and only here on the way to the UI and to outgoing artifacts. It is
//! not an input to finding detection, ranking, or any statutory evaluation.

use crate::{
    blocking,
    error::AppError,
    profile::{
        self,
        matching::{self, AgencyTarget, PortalTarget, ENTITY_SEARCH_LIMIT, LIMITATIONS},
        DisclosureContext, ProfileStore,
    },
    storage::{corpus::CorpusIndex, sqlite::DuckDbRepository},
    types::profile::*,
    types::CorpusEntityNode,
};
use std::sync::Arc;
use tauri::State;

fn state_of(profile: Option<UserProfile>) -> ProfileState {
    ProfileState {
        status: if profile.is_some() {
            ProfileStatus::Set
        } else {
            ProfileStatus::Unset
        },
        profile,
    }
}

async fn on_store<T, F>(
    store: &Arc<ProfileStore>,
    task: &'static str,
    operation: F,
) -> Result<T, AppError>
where
    T: Send + 'static,
    F: FnOnce(&ProfileStore) -> Result<T, AppError> + Send + 'static,
{
    let store = Arc::clone(store);
    blocking::spawn(task, move || operation(&store)).await?
}

#[tauri::command]
pub async fn get_profile(store: State<'_, Arc<ProfileStore>>) -> Result<ProfileState, AppError> {
    on_store(&store, "Profile read", |store| store.load().map(state_of)).await
}

#[tauri::command]
pub async fn save_profile(
    input: ProfileInput,
    store: State<'_, Arc<ProfileStore>>,
) -> Result<ProfileState, AppError> {
    on_store(&store, "Profile save", move |store| {
        store
            .update(|current| profile::apply_input(current, input))
            .map(Some)
            .map(state_of)
    })
    .await
}

#[tauri::command]
pub async fn delete_profile(
    store: State<'_, Arc<ProfileStore>>,
) -> Result<ProfileDeletionOutcome, AppError> {
    on_store(&store, "Profile delete", |store| store.delete()).await
}

const MAXIMUM_AGENCIES_SEARCHED: i64 = 5_000;

struct Sources {
    portals: Vec<PortalTarget>,
    agencies: Vec<AgencyTarget>,
    entities: Vec<CorpusEntityNode>,
}

async fn gather_sources(
    repository: &DuckDbRepository,
    corpus: &CorpusIndex,
) -> Result<Sources, AppError> {
    let portals = repository
        .list_portal_registrations()
        .await?
        .into_iter()
        .map(|saved| PortalTarget {
            portal_key: saved.registration.portal_key,
            display_name: saved.registration.display_name,
        })
        .collect();
    let agencies = repository
        .query_connection("Agency listing for affiliation matching", |connection| {
            let mut statement = connection
                .prepare(
                    "SELECT portal_subdomain, id, name FROM agencies
                     ORDER BY portal_subdomain, id LIMIT ?1",
                )
                .map_err(|error| {
                    AppError::Storage(format!("Failed to prepare the agency listing: {error}"))
                })?;
            let rows = statement
                .query_map([MAXIMUM_AGENCIES_SEARCHED], |row| {
                    Ok(AgencyTarget {
                        portal_key: row.get(0)?,
                        agency_id: row.get(1)?,
                        name: row.get(2)?,
                    })
                })
                .map_err(|error| {
                    AppError::Storage(format!("Failed to read the agency listing: {error}"))
                })?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| {
                    AppError::Storage(format!("Failed to read an agency row: {error}"))
                })?;
            Ok(rows)
        })
        .await?;
    let entities = corpus.top_entities(ENTITY_SEARCH_LIMIT).await?;
    Ok(Sources {
        portals,
        agencies,
        entities,
    })
}

#[tauri::command]
pub async fn list_affiliation_matches(
    store: State<'_, Arc<ProfileStore>>,
    repository: State<'_, DuckDbRepository>,
    corpus: State<'_, CorpusIndex>,
) -> Result<AffiliationMatchList, AppError> {
    let profile = on_store(&store, "Profile read", |store| store.load()).await?;
    let limitations = LIMITATIONS.iter().map(|text| (*text).to_string()).collect();
    let Some(profile) = profile else {
        return Ok(AffiliationMatchList {
            status: AffiliationMatchStatus::Unset,
            candidates: Vec::new(),
            searched_portal_count: 0,
            searched_agency_count: 0,
            searched_entity_count: 0,
            limitations,
        });
    };
    if profile.affiliations.is_empty() {
        return Ok(AffiliationMatchList {
            status: AffiliationMatchStatus::NoAffiliationsDeclared,
            candidates: Vec::new(),
            searched_portal_count: 0,
            searched_agency_count: 0,
            searched_entity_count: 0,
            limitations,
        });
    }
    let sources = gather_sources(&repository, &corpus).await?;
    let candidates = matching::find_candidates(
        &profile,
        &sources.portals,
        &sources.agencies,
        &sources.entities,
    );
    let searched_entity_count = sources
        .entities
        .iter()
        .filter(|entity| {
            matches!(
                entity.kind,
                crate::types::CorpusEntityKind::Organization
                    | crate::types::CorpusEntityKind::Acronym
            )
        })
        .count() as u64;
    Ok(AffiliationMatchList {
        status: if candidates.is_empty() {
            AffiliationMatchStatus::NoMatchYet
        } else {
            AffiliationMatchStatus::HasCandidates
        },
        candidates,
        searched_portal_count: sources.portals.len() as u64,
        searched_agency_count: sources.agencies.len() as u64,
        searched_entity_count,
        limitations,
    })
}

async fn decide(
    store: &Arc<ProfileStore>,
    input: AffiliationMatchDecisionInput,
    decision: MatchDecisionKind,
) -> Result<ProfileState, AppError> {
    on_store(store, "Profile match decision", move |store| {
        store
            .update(|current| {
                let current = current.ok_or_else(|| {
                    AppError::DataParsing("There is no profile to record a match in".to_string())
                })?;
                profile::apply_decision(current, input, decision)
            })
            .map(Some)
            .map(state_of)
    })
    .await
}

#[tauri::command]
pub async fn confirm_affiliation_match(
    input: AffiliationMatchDecisionInput,
    store: State<'_, Arc<ProfileStore>>,
) -> Result<ProfileState, AppError> {
    decide(&store, input, MatchDecisionKind::Confirmed).await
}

#[tauri::command]
pub async fn reject_affiliation_match(
    input: AffiliationMatchDecisionInput,
    store: State<'_, Arc<ProfileStore>>,
) -> Result<ProfileState, AppError> {
    decide(&store, input, MatchDecisionKind::Rejected).await
}

pub(crate) async fn resolve_disclosure(
    store: &Arc<ProfileStore>,
    repository: &DuckDbRepository,
    corpus: &CorpusIndex,
) -> Result<AffiliationDisclosure, AppError> {
    let profile = on_store(store, "Profile read", |store| store.load()).await?;
    let own = match profile
        .as_ref()
        .filter(|profile| !profile.match_decisions.is_empty())
    {
        Some(profile) => {
            let sources = gather_sources(repository, corpus).await?;
            matching::resolve_confirmed_subjects(
                profile,
                &sources.portals,
                &sources.agencies,
                &sources.entities,
            )
        }
        None => matching::ConfirmedSubjects::default(),
    };
    Ok(profile::build_disclosure(profile.as_ref(), own))
}

/// What every outgoing artifact must carry. An unreadable profile file is an error rather than an
/// empty disclosure: exporting without a declared affiliation because the file was corrupt would
/// be the silent omission the disclosure exists to prevent.
pub(crate) async fn disclosure_context(
    store: &Arc<ProfileStore>,
    repository: &DuckDbRepository,
    corpus: &CorpusIndex,
) -> Result<DisclosureContext, AppError> {
    resolve_disclosure(store, repository, corpus)
        .await
        .map(|disclosure| DisclosureContext::from_disclosure(&disclosure))
}

#[tauri::command]
pub async fn get_affiliation_disclosure(
    store: State<'_, Arc<ProfileStore>>,
    repository: State<'_, DuckDbRepository>,
    corpus: State<'_, CorpusIndex>,
) -> Result<AffiliationDisclosure, AppError> {
    resolve_disclosure(&store, &repository, &corpus).await
}
