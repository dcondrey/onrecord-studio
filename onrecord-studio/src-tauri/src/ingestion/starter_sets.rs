//! Versioned registry of public-source starter sets.
//!
//! A starter set is data: a named bundle of sources that the operator may add through the
//! existing `start_data_source` path. Nothing here fetches anything, and listing a set starts no
//! network activity. Every entry's `id` and `url` must equal a built-in San Diego source preset
//! in `src/utils/sanDiego.ts`, so the crawl policy and guidance that preset carries apply when the
//! entry is added; `src/tests/starterSets.test.tsx` enforces that. The boards and commissions set
//! is not declared here: it is derived from the meeting-body registry (`meeting_bodies.rs`).
//!
//! An entry is listed only after its URL was fetched and confirmed to be what `what` says it is on
//! `verified_as_of`. That date is a fact about one check, not a promise the page is still there:
//! a source that later fails to load is reported as unavailable when it is added.

use crate::error::AppError;
use serde::Serialize;
use std::collections::HashSet;

/// Bump when an entry is added, removed, or changes URL, so a stored reference to a set can tell
/// that the set it named is no longer the same set.
pub const STARTER_SET_REGISTRY_VERSION: u32 = 2;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum StarterEntryKind {
    /// Published records or data collected by crawling.
    Source,
    /// A public-records request portal: its released requests and attachments are collected, and
    /// it is where a request to that agency is filed.
    RecordRequestTarget,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StarterEntry {
    pub id: &'static str,
    pub kind: StarterEntryKind,
    pub title: &'static str,
    pub url: &'static str,
    pub what: &'static str,
    pub publisher: &'static str,
    /// What the publisher states about reuse, or `None` when no statement was found. Absence is
    /// not permission.
    pub terms_note: Option<&'static str>,
    /// Calendar date (YYYY-MM-DD) on which the URL was fetched and confirmed.
    pub verified_as_of: &'static str,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StarterSet {
    pub id: &'static str,
    pub title: &'static str,
    pub summary: &'static str,
    pub entries: &'static [StarterEntry],
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StarterSetsReport {
    pub registry_version: u32,
    pub sets: Vec<StarterSet>,
}

const VERIFIED: &str = "2026-09-30";
const CITY_OPEN_DATA_TERMS: &str = "Published under the City's Terms of Use and Open Data Policy (data.sandiego.gov/help/guides/terms); read them before redistributing.";

const fn entry(
    id: &'static str,
    kind: StarterEntryKind,
    title: &'static str,
    url: &'static str,
    what: &'static str,
    publisher: &'static str,
    terms_note: Option<&'static str>,
) -> StarterEntry {
    StarterEntry {
        id,
        kind,
        title,
        url,
        what,
        publisher,
        terms_note,
        verified_as_of: VERIFIED,
    }
}

use StarterEntryKind::{RecordRequestTarget, Source};

pub const STARTER_SETS: &[StarterSet] = &[
    StarterSet {
        id: "city-budget-and-actuals",
        title: "City budget and actuals",
        summary: "Adopted operating budget, reported actuals, the City's budget transparency site, and the oversight reports that interpret them.",
        entries: &[
            entry(
                "sd-operating-budget",
                Source,
                "City operating budget dataset",
                "https://data.sandiego.gov/datasets/operating-budget",
                "Operating budget line items by fiscal year from the open data portal.",
                "City of San Diego Department of Finance",
                Some(CITY_OPEN_DATA_TERMS),
            ),
            entry(
                "sd-operating-actuals",
                Source,
                "City operating actuals dataset",
                "https://data.sandiego.gov/datasets/operating-actuals/",
                "Reported operating expenditures and revenues, for comparison with the budget.",
                "City of San Diego Department of Finance",
                Some(CITY_OPEN_DATA_TERMS),
            ),
            entry(
                "sd-opengov-budget",
                Source,
                "City budget transparency",
                "https://budget.sandiego.gov/transparency",
                "The City's public budget reporting site.",
                "City of San Diego",
                None,
            ),
            entry(
                "sd-iba",
                Source,
                "Independent Budget Analyst reports",
                "https://www.sandiego.gov/iba/reports",
                "Budget analysis and recommendations to the City Council.",
                "City of San Diego Independent Budget Analyst",
                None,
            ),
            entry(
                "sd-auditor-reports",
                Source,
                "City Auditor reports",
                "https://www.sandiego.gov/auditor/reports",
                "Published audits with findings and management responses.",
                "City of San Diego City Auditor",
                None,
            ),
        ],
    },
    StarterSet {
        id: "council-and-commissions",
        title: "City Council meetings",
        summary: "Posted City Council agendas and the Council agenda archive. Boards, commissions and Council committees are in the City boards and commissions set.",
        entries: &[
            entry(
                "sd-council-agendas",
                Source,
                "City Council agendas",
                "https://www.sandiego.gov/city-clerk/officialdocs/legisdocs/brown-act",
                "Current and upcoming Council agendas with Brown Act notices.",
                "City of San Diego City Clerk",
                None,
            ),
            entry(
                "sd-hyland-agendas",
                Source,
                "Council agenda archive",
                "https://sandiego.hylandcloud.com/211agendaonlinecouncil",
                "Searchable archive of Council agendas and packets (OnBase Agenda Online).",
                "City of San Diego City Clerk",
                None,
            ),
        ],
    },
    StarterSet {
        id: "police-transparency",
        title: "Police policy and oversight",
        summary: "SDPD written policies and the Commission on Police Practices' published resources.",
        entries: &[
            entry(
                "sd-police-policies",
                Source,
                "SDPD policies and procedures",
                "https://www.sandiego.gov/police/data-transparency/policies-procedures",
                "Department policies, procedures, and training documents.",
                "City of San Diego Police Department",
                None,
            ),
            entry(
                "sd-cpp-resources",
                Source,
                "Commission on Police Practices news and resources",
                "https://www.sandiego.gov/cpp/reports",
                "The Commission's published reports and reference documents.",
                "City of San Diego Commission on Police Practices",
                None,
            ),
        ],
    },
    StarterSet {
        id: "open-data-portals",
        title: "Open data portals",
        summary: "City, regional, and County data catalogs, including the City's Get It Done service requests.",
        entries: &[
            entry(
                "sd-open-data",
                Source,
                "City of San Diego open data portal",
                "https://data.sandiego.gov",
                "The City's dataset catalog.",
                "City of San Diego",
                Some(CITY_OPEN_DATA_TERMS),
            ),
            entry(
                "sd-get-it-done",
                Source,
                "Get It Done service reports",
                "https://data.sandiego.gov/datasets/get-it-done-reports/",
                "Non-emergency service requests. They count requests, not unique encampments or completed work.",
                "City of San Diego",
                Some(CITY_OPEN_DATA_TERMS),
            ),
            entry(
                "sandag-open-data",
                Source,
                "SANDAG open data portal",
                "https://opendata.sandag.org",
                "Regional planning and demographic tables.",
                "SANDAG",
                None,
            ),
            entry(
                "sd-county-budget",
                Source,
                "County of San Diego budget portal",
                "https://sandiegocounty.budget.socrata.com",
                "The County's published budget tables.",
                "County of San Diego",
                None,
            ),
        ],
    },
    StarterSet {
        id: "cpra-request-targets",
        title: "Public-records request portals",
        summary: "The City, Housing Commission, and County portals where California Public Records Act requests are filed and released records are posted.",
        entries: &[
            entry(
                "sd-nextrequest",
                RecordRequestTarget,
                "City of San Diego public records portal",
                "https://sandiego.nextrequest.com",
                "City requests and released documents on NextRequest.",
                "City of San Diego",
                None,
            ),
            entry(
                "sdhc-nextrequest",
                RecordRequestTarget,
                "San Diego Housing Commission records portal",
                "https://sandiegohousingcommission.nextrequest.com",
                "The Housing Commission's preferred channel for California Public Records Act requests.",
                "San Diego Housing Commission",
                None,
            ),
            entry(
                "sd-county-pra",
                RecordRequestTarget,
                "County of San Diego Public Records Request Center",
                "https://pra.sandiegocounty.gov",
                "County requests and records processed since the platform launched on 2024-03-21.",
                "County of San Diego",
                None,
            ),
        ],
    },
];

/// The hand-declared sets followed by the meeting-body set derived from the shared registry.
pub fn all_starter_sets() -> Vec<StarterSet> {
    let mut sets = STARTER_SETS.to_vec();
    sets.push(super::meeting_bodies::boards_starter_set());
    sets
}

fn normalize_url(url: &str) -> String {
    url.trim_end_matches('/').to_ascii_lowercase()
}

/// Fails on a malformed registry: empty or duplicate sets, duplicate entry ids, non-https URLs, or
/// two entries naming the same URL (ignoring case and a trailing slash).
pub fn validate_registry(sets: &[StarterSet]) -> Result<(), AppError> {
    let mut set_ids = HashSet::new();
    let mut ids = HashSet::new();
    let mut urls = HashSet::new();
    for set in sets {
        if set.entries.is_empty() || !set_ids.insert(set.id) {
            return Err(AppError::Internal(format!(
                "starter set {} is empty or duplicated",
                set.id
            )));
        }
        for entry in set.entries {
            if !ids.insert(entry.id) {
                return Err(AppError::Internal(format!(
                    "duplicate starter entry id {}",
                    entry.id
                )));
            }
            if !entry.url.starts_with("https://") || !urls.insert(normalize_url(entry.url)) {
                return Err(AppError::Internal(format!(
                    "starter entry {} has a non-https or duplicate URL",
                    entry.id
                )));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set_of(entries: &'static [StarterEntry]) -> StarterSet {
        StarterSet {
            id: "s",
            title: "t",
            summary: "s",
            entries,
        }
    }

    #[test]
    fn shipped_registry_is_well_formed() {
        let sets = all_starter_sets();
        validate_registry(&sets).unwrap();
        for set in &sets {
            for entry in set.entries {
                assert!(reqwest::Url::parse(entry.url).is_ok(), "{} URL", entry.id);
                assert!(
                    !entry.title.is_empty()
                        && !entry.what.is_empty()
                        && !entry.publisher.is_empty()
                );
                let date: Vec<&str> = entry.verified_as_of.split('-').collect();
                assert_eq!(date.len(), 3, "{} verified_as_of", entry.id);
                assert!(date
                    .iter()
                    .all(|part| part.chars().all(|c| c.is_ascii_digit())));
            }
        }
    }

    #[test]
    fn a_duplicate_url_is_refused_despite_case_and_trailing_slash() {
        static ENTRIES: [StarterEntry; 2] = [
            fixture_const("https://x.example/p", "a"),
            fixture_const("https://X.example/p/", "b"),
        ];
        assert!(validate_registry(&[set_of(&ENTRIES)]).is_err());
    }

    #[test]
    fn plain_http_is_refused() {
        static ENTRIES: [StarterEntry; 1] = [fixture_const("http://x.example/p", "a")];
        assert!(validate_registry(&[set_of(&ENTRIES)]).is_err());
    }

    #[test]
    fn a_duplicate_id_is_refused() {
        static ENTRIES: [StarterEntry; 2] = [
            fixture_const("https://x.example/a", "a"),
            fixture_const("https://x.example/b", "a"),
        ];
        assert!(validate_registry(&[set_of(&ENTRIES)]).is_err());
    }

    const fn fixture_const(url: &'static str, id: &'static str) -> StarterEntry {
        entry(id, Source, "t", url, "w", "p", None)
    }
}
