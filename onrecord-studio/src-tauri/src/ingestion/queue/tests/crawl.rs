use super::super::{
    crawl::{crawl_path_identity, insert_bounded_identity},
    structured::discover_structured_document_links,
};
use super::*;

#[test]
fn release_stress_harness_bounds_high_link_discovery() {
    use std::fmt::Write as _;

    let mut payload = String::from("{\"resources\":[");
    for index in 0..=MAXIMUM_STRUCTURED_DISCOVERY_LINKS {
        if index > 0 {
            payload.push(',');
        }
        write!(
            payload,
            "{{\"downloadUrl\":\"https://files.example.test/{index}.pdf\"}}"
        )
        .unwrap();
    }
    payload.push_str("]}");
    assert!(payload.len() < MAXIMUM_STRUCTURED_DISCOVERY_BYTES as usize);
    let discovery = discover_structured_document_links(payload.as_bytes(), None).unwrap();
    assert_eq!(
        discovery.candidates.len(),
        MAXIMUM_STRUCTURED_DISCOVERY_LINKS
    );
}

#[test]
fn san_diego_policy_categories_and_pages_keep_distinct_crawl_nodes() {
    let procedures =
        "https://www.sandiego.gov/police/data-transparency/policies-procedures/procedures";
    assert_eq!(
        crawl_path_identity(procedures),
        crawl_path_identity(&format!("{procedures}/?page=0"))
    );
    assert_ne!(
        crawl_path_identity(procedures),
        crawl_path_identity(&format!("{procedures}?page=1"))
    );
    assert_eq!(
        crawl_path_identity(&format!("{procedures}?page=1")),
        crawl_path_identity(&format!("{procedures}?page=1&tracking=123"))
    );
    let categories =
        "https://www.sandiego.gov/police/data-transparency/policies-procedures/category";
    let orders = format!("{categories}?category=Department%20Orders");
    let policies = format!("{categories}?category=Policies");
    assert_ne!(crawl_path_identity(&orders), crawl_path_identity(&policies));
    assert_ne!(
        crawl_path_identity(&orders),
        crawl_path_identity(&format!("{orders}&page=1"))
    );
    assert_eq!(
        crawl_path_identity(&format!("{procedures}?page=100")),
        crawl_path_identity(&format!("{procedures}?page=99999"))
    );
    // The exception never applies to a lookalike host or the City's other sections.
    let other = procedures.replace("www.sandiego.gov", "www.sandiego.gov.example.test");
    assert_eq!(
        crawl_path_identity(&other),
        crawl_path_identity(&format!("{other}?page=1"))
    );
    let calendar = "https://www.sandiego.gov/cpp/meetingsagendas";
    assert_eq!(
        crawl_path_identity(calendar),
        crawl_path_identity(&format!("{calendar}?page=1"))
    );
}

#[test]
fn faceted_search_variants_collapse_onto_one_crawl_path() {
    // Verbatim from https://data.ca.gov/dataset fetched 2026-08-25. Ten of these appear on a
    // single page and each one links ten more, which is the combinatorial expansion that made
    // the frontier unbounded.
    let seed = crawl_path_identity("https://data.ca.gov/dataset");
    for facet in [
        "https://data.ca.gov/dataset/?tags=species",
        "https://data.ca.gov/dataset/?tags=wildlife",
        "https://data.ca.gov/dataset/?tags=habitat",
        "https://data.ca.gov/dataset/?tags=CDFW",
        "https://data.ca.gov/dataset/",
    ] {
        assert_eq!(
            crawl_path_identity(facet),
            seed,
            "{facet} claimed its own node"
        );
    }
    // A genuinely different record keeps its own node.
    assert_ne!(
        crawl_path_identity("https://data.ca.gov/dataset/fish-hatchery-annual-report"),
        seed
    );
    // Host is part of the identity, so two sites never share a claim.
    assert_ne!(
        crawl_path_identity("https://data.cityofnewyork.us/dataset"),
        seed
    );
}

#[test]
fn crawl_identity_sets_never_grow_past_the_policy_budget() {
    let mut identities = HashSet::new();
    assert!(insert_bounded_identity(&mut identities, "first", 2));
    assert!(!insert_bounded_identity(&mut identities, "first", 2));
    assert!(insert_bounded_identity(&mut identities, "second", 2));
    assert!(!insert_bounded_identity(&mut identities, "third", 2));
    assert_eq!(
        identities,
        HashSet::from(["first".to_string(), "second".to_string()])
    );
}
