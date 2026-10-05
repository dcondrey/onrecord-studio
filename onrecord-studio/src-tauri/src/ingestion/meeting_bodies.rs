//! Registry of public meeting bodies, read from the one definition the source catalogue also
//! reads (`src/content/meetingBodies.json`, consumed by `src/utils/meetingBodySources.ts`).
//!
//! The registry is data: listing it contacts nothing. A body with a `verifiedAsOf` date had its
//! index fetched and an agenda link confirmed on that date and is offered in the
//! `city-boards-and-commissions` starter set; a body without one is only a catalogue source the
//! operator may choose, because a starter entry asserts a confirmed address.

use super::starter_sets::{StarterEntry, StarterEntryKind, StarterSet};
use serde::Deserialize;
use std::sync::LazyLock;

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PublishingPattern {
    /// A sandiego.gov page linking agenda PDFs under `/sites/default/files/`.
    CityPagePdf,
    /// A sandiego.gov page whose agenda is one address overwritten for each meeting.
    CityFixedUrl,
    /// OnBase Agenda Online: a revised docket keeps its meeting address.
    Onbase,
    /// An agency's own site.
    AgencyPage,
}

impl PublishingPattern {
    fn what(self) -> &'static str {
        match self {
            Self::CityPagePdf => "The body's City page and the agenda and minutes files it links.",
            Self::CityFixedUrl => {
                "The body's City page and its agenda, published at one address that is overwritten for each meeting."
            }
            Self::Onbase => "Meeting pages, agendas and item attachments from OnBase Agenda Online.",
            Self::AgencyPage => "The body's meeting page and the agendas, minutes and reports it links.",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingBody<'a> {
    pub id: &'a str,
    /// The body's name as its publisher writes it.
    pub name: &'a str,
    pub url: &'a str,
    pub pattern: PublishingPattern,
    pub publisher: &'a str,
    /// `None` means the index address was taken from the City's roster and not opened.
    #[serde(borrow)]
    pub verified_as_of: Option<&'a str>,
    #[serde(borrow)]
    pub note: Option<&'a str>,
    /// Other names the body's own agendas use in their heading.
    #[serde(borrow)]
    pub heading_names: Vec<&'a str>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Registry<'a> {
    set_id: &'a str,
    #[serde(borrow)]
    bodies: Vec<MeetingBody<'a>>,
}

static REGISTRY: LazyLock<Registry<'static>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../../../src/content/meetingBodies.json"))
        .expect("src/content/meetingBodies.json is a valid meeting body registry")
});

static VERIFIED_ENTRIES: LazyLock<Vec<StarterEntry>> = LazyLock::new(|| {
    REGISTRY
        .bodies
        .iter()
        .filter_map(|body| {
            Some(StarterEntry {
                id: body.id,
                kind: StarterEntryKind::Source,
                title: body.name,
                url: body.url,
                what: body.pattern.what(),
                publisher: body.publisher,
                terms_note: None,
                verified_as_of: body.verified_as_of?,
            })
        })
        .collect()
});

pub fn meeting_bodies() -> &'static [MeetingBody<'static>] {
    &REGISTRY.bodies
}

/// The verified bodies as one starter set, so they can be added together.
pub fn boards_starter_set() -> StarterSet {
    StarterSet {
        id: REGISTRY.set_id,
        title: "City boards and commissions",
        summary: "Agenda pages of City of San Diego boards, commissions and Council committees, plus the Housing Commission board and the Housing Authority. Only bodies whose page was opened and found to link agendas are included.",
        entries: &VERIFIED_ENTRIES,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn every_body_is_addressable_and_named_once() {
        let mut ids = HashSet::new();
        let mut names = HashSet::new();
        for body in meeting_bodies() {
            let url = reqwest::Url::parse(body.url).expect(body.id);
            assert_eq!(url.scheme(), "https", "{}", body.id);
            assert!(ids.insert(body.id), "duplicate id {}", body.id);
            assert!(names.insert(body.name), "duplicate name {}", body.name);
            match body.pattern {
                PublishingPattern::CityPagePdf | PublishingPattern::CityFixedUrl => {
                    assert_eq!(url.host_str(), Some("www.sandiego.gov"), "{}", body.id)
                }
                PublishingPattern::Onbase => {
                    assert_eq!(url.host_str(), Some("sandiego.hylandcloud.com"))
                }
                PublishingPattern::AgencyPage => {}
            }
        }
    }

    #[test]
    fn the_starter_set_holds_exactly_the_verified_bodies() {
        let set = boards_starter_set();
        let verified = meeting_bodies()
            .iter()
            .filter(|body| body.verified_as_of.is_some())
            .map(|body| (body.id, body.url))
            .collect::<Vec<_>>();
        assert_eq!(
            set.entries
                .iter()
                .map(|entry| (entry.id, entry.url))
                .collect::<Vec<_>>(),
            verified
        );
        // The owner's priority bodies are all verified, so "all City boards" includes them.
        for id in [
            "sd-body-hracoc",
            "sdhc-board-meetings",
            "sdhc-housing-authority-meetings",
            "sd-cpp-meetings",
            "sd-body-human-relations",
            "sd-body-privacy-advisory",
            "sd-hyland-committees",
            "sd-body-cpab",
            "sd-body-planning-commission",
            "sd-body-park-rec-board",
            "sd-body-balboa-park",
            "sd-body-mission-bay-park",
            "sd-body-iroc",
            "sd-body-civil-service",
            "sd-body-ethics",
        ] {
            assert!(set.entries.iter().any(|entry| entry.id == id), "{id}");
        }
    }
}
