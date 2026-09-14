//! Small, deterministic adapters for published regional URLs. No speculative endpoint crawling.
use reqwest::Url;

/// Publisher scope comes from the registered source, never a substring in a request ID or title.
pub(crate) fn source_jurisdiction(source: &str) -> &'static str {
    let Ok(url) = Url::parse(source) else {
        return "Unclassified source";
    };
    match url
        .host_str()
        .unwrap_or_default()
        .trim_start_matches("www.")
    {
        "sandiego.gov"
        | "data.sandiego.gov"
        | "budget.sandiego.gov"
        | "sandiego.nextrequest.com"
        | "sandiego.hylandcloud.com"
        | "sandiego.granicus.com"
        | "efile.sandiego.gov"
        | "insidesandiego.org"
        | "thinkblue.org"
        | "sdcers.gov" => "City of San Diego",
        "sandiegocounty.gov"
        | "pra.sandiegocounty.gov"
        | "engage.sandiegocounty.gov"
        | "data.sandiegocounty.gov"
        | "sandiegocounty.budget.socrata.com"
        | "sdvote.com"
        | "sdarcc.gov"
        | "sdcera.org" => "San Diego County",
        "sdsheriff.gov" => "San Diego County Sheriff",
        "sdcda.org" => "San Diego County District Attorney",
        "sandiegohousingcommission.nextrequest.com" | "sdhc.org" => "San Diego Housing Commission",
        "sandag.org" | "opendata.sandag.org" | "sdgis-sandag.opendata.arcgis.com" => {
            "SANDAG / SanGIS"
        }
        "data.ca.gov" | "waterboards.ca.gov" | "hcd.ca.gov" | "information.auditor.ca.gov" => {
            "California"
        }
        "hud.gov" | "data.transportation.gov" => "United States",
        "catalog.data.gov" | "data.sandiegodata.org" | "opendatasandiego.com" => {
            "Multi-publisher catalogs"
        }
        "sdmts.com" => "San Diego Metropolitan Transit System",
        "escondido.gov" | "escondido.org" => "City of Escondido",
        "sandiegounified.org" => "San Diego Unified School District",
        "powayusd.com" => "Poway Unified School District",
        "data.sdcoe.net" => "San Diego County Office of Education",
        "library.ucsd.edu"
        | "massive-ftp.ucsd.edu"
        | "homelessnesshub.ucsd.edu"
        | "datanexus.ucsd.edu"
        | "hhubsandiego-ucsdonline.hub.arcgis.com" => "UC San Diego",
        "humandynamics.sdsu.edu" | "asir.sdsu.edu" => "San Diego State University",
        "ppic.org" | "csh.org" => "Research and policy organizations",
        "voiceofsandiego.org"
        | "kpbs.org"
        | "fox5sandiego.com"
        | "calmatters.org"
        | "timesofsandiego.com"
        | "cbs8.com"
        | "inewsource.org"
        | "thecoastnews.com"
        | "sdvoice.info"
        | "axios.com" => "News publishers",
        "transparenthomelessdata.com" | "san-diego-homelessness-analytics.vercel.app" => {
            "Independent dashboards"
        }
        "sdapcd.org" => "San Diego Air Pollution Control District",
        "arjis.org"
        | "livewellsd.org"
        | "rtfhsd.org"
        | "211sandiego.org"
        | "projectcleanwater.org"
        | "sdirwmp.org"
        | "innovate78.com"
        | "elcajoncollaborative.org"
        | "luckyduckfoundation.org"
        | "sdrescue.org"
        | "downtownsandiego.org" => "San Diego regional organizations",
        "ssl.netfile.com" if url.path().starts_with("/static/agency/csd/") => "City of San Diego",
        "mccmeetingspublic.blob.core.usgovcloudapi.net"
            if url.path().starts_with("/escondidca-meet-") =>
        {
            "City of Escondido"
        }
        _ => "Unclassified source",
    }
}

pub(crate) fn is_publication_site(url: &Url) -> bool {
    matches!(
        url.host_str()
            .unwrap_or_default()
            .trim_start_matches("www."),
        "data.sandiego.gov"
            | "budget.sandiego.gov"
            | "sandiego.gov"
            | "sandiegocounty.gov"
            | "sandag.org"
            | "opendata.sandag.org"
            | "sandiegocounty.budget.socrata.com"
            | "data.sandiegocounty.gov"
            | "sdgis-sandag.opendata.arcgis.com"
            | "data.ca.gov"
            | "library.ucsd.edu"
            | "gis.sandiego.org"
            | "arjis.org"
            | "data.sdcoe.net"
            | "rtfhsd.org"
            | "211sandiego.org"
            | "livewellsd.org"
            | "innovate78.com"
            | "opendatasandiego.com"
            | "escondido.org"
            | "escondido.gov"
            | "humandynamics.sdsu.edu"
            | "sdsheriff.gov"
            | "downtownsandiego.org"
            | "homelessdata.com"
            | "sandiego.hylandcloud.com"
            | "sandiego.granicus.com"
            | "sdapcd.org"
            | "sdirwmp.org"
            | "projectcleanwater.org"
            | "thinkblue.org"
            | "waterboards.ca.gov"
            | "ssl.netfile.com"
            | "sdvote.com"
            | "sdcers.gov"
            | "sdcera.org"
            | "sdarcc.gov"
            | "sandiegounified.org"
            | "efile.sandiego.gov"
            | "homelessnesshub.ucsd.edu"
            | "data.sandiegodata.org"
            | "voiceofsandiego.org"
            | "insidesandiego.org"
            | "elcajoncollaborative.org"
            | "transparenthomelessdata.com"
            | "kpbs.org"
            | "san-diego-homelessness-analytics.vercel.app"
            | "ppic.org"
            | "fox5sandiego.com"
            | "calmatters.org"
            | "sdcda.org"
            | "hhubsandiego-ucsdonline.hub.arcgis.com"
            | "timesofsandiego.com"
            | "cbs8.com"
            | "hcd.ca.gov"
            | "luckyduckfoundation.org"
            | "inewsource.org"
            | "sdrescue.org"
            | "thecoastnews.com"
            | "sdvoice.info"
            | "axios.com"
            | "powayusd.com"
            | "hud.gov"
            | "sdhc.org"
            | "sdmts.com"
            | "datanexus.ucsd.edu"
            | "catalog.data.gov"
            | "data.transportation.gov"
            | "asir.sdsu.edu"
            | "csh.org"
            | "information.auditor.ca.gov"
            | "engage.sandiegocounty.gov"
    ) || (url.host_str() == Some("mccmeetingspublic.blob.core.usgovcloudapi.net")
        && url.path().starts_with("/escondidca-meet-"))
}

pub(crate) fn is_catalog_root(url: &Url) -> bool {
    // A selected package, filtered search, API query or file must never expand into the entire portal.
    url.query().is_none()
        && matches!(
            url.path().trim_end_matches('/'),
            "" | "/browse" | "/catalog" | "/dataset" | "/datasets"
        )
}

pub(crate) fn socrata_host(host: &str) -> bool {
    matches!(
        host,
        "opendata.sandag.org" | "data.sandiegocounty.gov" | "data.transportation.gov"
    )
}

/// Data.gov publishes these metadata links on dataset pages. Treat only its exact public
/// record route as a payload, so a selected record can expose its DCAT distribution URLs.
pub(crate) fn is_datagov_metadata_url(url: &Url) -> bool {
    if url.host_str() != Some("catalog.data.gov") || url.query().is_some() {
        return false;
    }
    let parts = url.path().trim_matches('/').split('/').collect::<Vec<_>>();
    parts.len() == 3
        && parts[0] == "harvest_record"
        && uuid::Uuid::parse_str(parts[1]).is_ok()
        && matches!(parts[2], "raw" | "transformed")
}

/// Read public OpenGov bootstrap JSON without evaluating its enclosing script. Only this
/// verified City tenant and explicitly public report IDs are eligible for companion requests.
pub(crate) fn opengov_companions(url: &Url, html: &str) -> Vec<(String, String)> {
    if url.host_str() != Some("budget.sandiego.gov")
        || !matches!(url.path().trim_end_matches('/'), "" | "/transparency")
    {
        return Vec::new();
    }
    let pattern = regex::Regex::new(r"\bvar\s+gon\s*=\s*").expect("static pattern");
    let Some(start) = pattern.find(html) else {
        return Vec::new();
    };
    let Some(Ok(value)) = serde_json::Deserializer::from_str(&html[start.end()..])
        .into_iter::<serde_json::Value>()
        .next()
    else {
        return Vec::new();
    };
    if value
        .pointer("/entity/custom_domain")
        .and_then(serde_json::Value::as_str)
        != Some("budget.sandiego.gov")
        || value
            .pointer("/entity/published")
            .and_then(serde_json::Value::as_bool)
            != Some(true)
    {
        return Vec::new();
    }
    let mut links = Vec::new();
    for report in value
        .get("reports")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .take(100)
    {
        if report.get("visibility").and_then(serde_json::Value::as_str) != Some("public") {
            continue;
        }
        if let Some(id) = report
            .get("id")
            .and_then(serde_json::Value::as_u64)
            .filter(|id| *id > 0)
        {
            let name = report
                .get("name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("City budget report");
            links.push((
                format!("https://budget.sandiego.gov/api/v1/reports/{id}"),
                format!(
                    "{} — report metadata",
                    name.chars().take(200).collect::<String>()
                ),
            ));
        }
    }
    for document in value
        .pointer("/entity/documents")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .take(100)
    {
        let Some(target) = document
            .get("file_url")
            .and_then(serde_json::Value::as_str)
            .and_then(|s| Url::parse(s).ok())
        else {
            continue;
        };
        if target.scheme() == "https" && target.username().is_empty() && target.password().is_none()
        {
            let name = document
                .get("name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("OpenGov supporting document");
            links.push((target.to_string(), name.chars().take(256).collect()));
        }
    }
    links
}

/// A hash-selected report must not be broadened into the default dashboard. Filtered visual
/// states require a browser export because their client-side selection is not a raw API query.
pub(crate) fn normalize_opengov_selection(url: &Url) -> Result<Option<Url>, &'static str> {
    if url.host_str() != Some("budget.sandiego.gov") {
        return Ok(None);
    }
    let Some(fragment) = url.fragment().filter(|s| !s.trim_matches('/').is_empty()) else {
        return Ok(None);
    };
    let parts = fragment.trim_matches('/').split('/').collect::<Vec<_>>();
    if matches!(url.path().trim_end_matches('/'), "" | "/transparency")
        && url.query().is_none()
        && parts.len() == 3
        && parts[0] == "sandiegoca"
        && parts[1] == "reports"
        && parts[2].parse::<u64>().is_ok_and(|id| id > 0)
    {
        return Ok(url.join(&format!("/api/v1/reports/{}", parts[2])).ok());
    }
    Err("This OpenGov URL includes a filtered visual state. Export its selected spreadsheet in the browser and import that file, or choose an unfiltered public report URL.")
}

fn socrata_id(value: &str) -> bool {
    value.len() == 9
        && value.as_bytes()[4] == b'-'
        && value
            .bytes()
            .enumerate()
            .all(|(i, b)| i == 4 || b.is_ascii_lowercase() || b.is_ascii_digit())
}

/// Companion metadata and complete exports for a *selected* dataset, never a portal-wide query.
pub(crate) fn dataset_companions(url: &Url) -> Vec<(String, String)> {
    let host = url.host_str().unwrap_or_default();
    if host == "data.ca.gov" {
        let parts = url
            .path_segments()
            .map(|parts| parts.collect::<Vec<_>>())
            .unwrap_or_default();
        if parts.len() >= 2 && parts[0] == "dataset" && !parts[1].is_empty() {
            let mut api = url
                .join("/api/3/action/package_show")
                .expect("valid absolute URL");
            api.query_pairs_mut().append_pair("id", parts[1]);
            return vec![(
                api.to_string(),
                "Dataset metadata and resource links".into(),
            )];
        }
    }
    if socrata_host(host) {
        // Only human-facing dataset routes. Never replace a filtered API query or its chosen format.
        let path = url.path();
        if !path.starts_with("/api/") && !path.starts_with("/resource/") {
            if let Some(id) = path
                .trim_end_matches('/')
                .rsplit('/')
                .next()
                .filter(|v| socrata_id(v))
            {
                return vec![
                    (
                        format!("https://{host}/api/views/{id}.json"),
                        "Dataset schema and attribution".into(),
                    ),
                    (
                        format!("https://{host}/api/views/{id}/rows.csv?accessType=DOWNLOAD"),
                        "Published CSV export".into(),
                    ),
                ];
            }
        }
    }
    Vec::new()
}

/// These query parameters identify distinct public meeting documents, not search facets.
pub(crate) fn meeting_identity(url: &Url) -> Option<String> {
    let host = url.host_str()?;
    let path = url.path().to_ascii_lowercase();
    let keys: &[&str] =
        if host == "sandiego.hylandcloud.com" && path.contains("/meetings/viewmeeting") {
            &["id", "doctype", "site"]
        } else if host == "sandiego.hylandcloud.com" && path.ends_with("/documents/viewagenda") {
            &["meetingId", "type", "doctype"]
        } else if host == "sandiego.granicus.com"
            && matches!(
                path.as_str(),
                "/viewpublisher.php"
                    | "/mediaplayer.php"
                    | "/generatedagendaviewer.php"
                    | "/agendaviewer.php"
            )
        {
            &["view_id", "clip_id", "event_id", "meta_id"]
        } else {
            return None;
        };
    let mut pairs = url
        .query_pairs()
        .filter(|(key, value)| keys.contains(&key.as_ref()) && !value.is_empty())
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect::<Vec<_>>();
    if pairs.is_empty() {
        return None;
    }
    pairs.sort();
    pairs.dedup();
    let mut identity = url.clone();
    identity.set_fragment(None);
    identity.set_query(None);
    identity.query_pairs_mut().extend_pairs(pairs);
    Some(identity.to_string())
}

/// Resolve the public OnBase document route that the selected meeting page itself declares.
pub(crate) fn hyland_document_companion(url: &Url, html: &str) -> Option<String> {
    if url.host_str()? != "sandiego.hylandcloud.com"
        || !url
            .path()
            .eq_ignore_ascii_case("/211agendaonlinecouncil/Meetings/ViewMeeting")
    {
        return None;
    }
    let pairs = url
        .query_pairs()
        .collect::<std::collections::BTreeMap<_, _>>();
    let id = pairs.get("id")?;
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let doctype = pairs.get("doctype").map(|s| s.as_ref()).unwrap_or("1");
    let kind = match doctype {
        "1" => "agenda",
        "2" => "minutes",
        "3" => "summary",
        _ => return None,
    };
    let template = format!(
        "/211agendaonlinecouncil/Documents/ViewAgenda?meetingId={id}&type=AGENDATYPEVALUE&doctype="
    );
    if !html.contains(&template) {
        return None;
    }
    let mut document = url
        .join("/211agendaonlinecouncil/Documents/ViewAgenda")
        .ok()?;
    document.query_pairs_mut().extend_pairs([
        ("meetingId", id.as_ref()),
        ("type", kind),
        ("doctype", doctype),
    ]);
    Some(document.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn opengov_bootstrap_resolves_only_published_tenant_reports_and_documents() {
        let url = Url::parse("https://budget.sandiego.gov/transparency").unwrap();
        let html = r#"<script>var gon = {"entity":{"custom_domain":"budget.sandiego.gov","published":true,"documents":[{"name":"Budget book","file_url":"https://files.example.test/book.pdf?X-Amz-Signature=fixture"},{"file_url":"javascript:alert(1)"}]},"reports":[{"id":7714,"visibility":"public","name":"Operating Budgets"},{"id":123,"visibility":"private"},{"id":999}]}; window.after = true;</script>"#;
        let links = opengov_companions(&url, html);
        assert_eq!(links.len(), 2);
        assert_eq!(
            links[0].0,
            "https://budget.sandiego.gov/api/v1/reports/7714"
        );
        assert!(links[1].0.ends_with("X-Amz-Signature=fixture"));
        assert!(opengov_companions(
            &url,
            &html.replace("\"published\":true", "\"published\":false")
        )
        .is_empty());
        assert!(opengov_companions(
            &Url::parse("https://budget.sandiego.gov.evil.test/transparency").unwrap(),
            html
        )
        .is_empty());
        assert!(opengov_companions(&url, "var gon = runCode()").is_empty());
        let selected =
            Url::parse("https://budget.sandiego.gov/transparency#/sandiegoca/reports/8082")
                .unwrap();
        assert_eq!(
            normalize_opengov_selection(&selected)
                .unwrap()
                .unwrap()
                .path(),
            "/api/v1/reports/8082"
        );
        assert!(normalize_opengov_selection(
            &Url::parse(&format!("{selected}?filter=some-account")).unwrap()
        )
        .is_err());
    }
    #[test]
    fn expanded_sources_keep_publisher_roles_and_selected_federal_datasets() {
        for (url, jurisdiction) in [
            ("https://www.insidesandiego.org/story", "City of San Diego"),
            ("https://voiceofsandiego.org/story", "News publishers"),
            (
                "https://sdhc.org/report.pdf",
                "San Diego Housing Commission",
            ),
            (
                "https://www.sdcda.org/reports",
                "San Diego County District Attorney",
            ),
            (
                "https://hhubsandiego-ucsdonline.hub.arcgis.com",
                "UC San Diego",
            ),
            (
                "https://catalog.data.gov/dataset/example",
                "Multi-publisher catalogs",
            ),
            ("https://data.transportation.gov", "United States"),
        ] {
            assert_eq!(source_jurisdiction(url), jurisdiction);
            assert!(is_publication_site(&Url::parse(url).unwrap()));
        }
        let links =
            dataset_companions(&Url::parse("https://data.transportation.gov/d/rki6-jcxp").unwrap());
        assert_eq!(links.len(), 2);
        assert!(links[1]
            .0
            .starts_with("https://data.transportation.gov/api/views/rki6-jcxp/"));
        assert!(dataset_companions(
            &Url::parse(
                "https://data.transportation.gov/resource/rki6-jcxp.csv?$where=state%3D'CA'"
            )
            .unwrap()
        )
        .is_empty());
        // These sites currently publish dataset pages, not the old CKAN service.
        assert!(dataset_companions(
            &Url::parse("https://catalog.data.gov/dataset/example").unwrap()
        )
        .is_empty());
        assert!(dataset_companions(
            &Url::parse("https://data.sandiegodata.org/dataset/example").unwrap()
        )
        .is_empty());
    }

    #[test]
    fn federal_metadata_download_identity_is_exact() {
        let path = "/harvest_record/9f3e9265-1a5a-414b-8551-fe420d3a79ec/raw";
        assert!(is_datagov_metadata_url(
            &Url::parse(&format!("https://catalog.data.gov{path}")).unwrap()
        ));
        assert!(!is_datagov_metadata_url(
            &Url::parse(&format!("https://catalog.data.gov.evil.test{path}")).unwrap()
        ));
        assert!(!is_datagov_metadata_url(
            &Url::parse("https://catalog.data.gov/harvest_record/missing/raw").unwrap()
        ));
        assert_eq!(
            source_jurisdiction(
                "https://mccmeetingspublic.blob.core.usgovcloudapi.net/another-city/report.pdf"
            ),
            "Unclassified source"
        );
    }
    #[test]
    fn individual_datasets_and_filters_never_trigger_catalog_enumeration() {
        for value in [
            "https://data.ca.gov/dataset/health",
            "https://opendata.sandag.org/d/x123-ab45",
            "https://data.ca.gov/dataset?q=San+Diego",
            "https://data.ca.gov/api/3/action/package_show?id=one",
        ] {
            assert!(!is_catalog_root(&Url::parse(value).unwrap()));
        }
        assert!(is_catalog_root(
            &Url::parse("https://data.ca.gov/dataset/").unwrap()
        ));
        let links =
            dataset_companions(&Url::parse("https://opendata.sandag.org/d/x123-ab45").unwrap());
        assert_eq!(links.len(), 2);
        assert!(links[1]
            .0
            .ends_with("/x123-ab45/rows.csv?accessType=DOWNLOAD"));
        assert!(dataset_companions(
            &Url::parse("https://opendata.sandag.org/resource/x123-ab45.json?$limit=10").unwrap()
        )
        .is_empty());
    }
    #[test]
    fn meeting_ids_survive_deduplication_but_tracking_does_not() {
        let a = Url::parse("https://sandiego.hylandcloud.com/211agendaonlinecouncil/Meetings/ViewMeeting?id=7000&doctype=1&site=council&utm_source=x").unwrap();
        let b = Url::parse("https://sandiego.hylandcloud.com/211agendaonlinecouncil/Meetings/ViewMeeting?site=council&doctype=1&id=7000").unwrap();
        assert_eq!(meeting_identity(&a), meeting_identity(&b));
        let c = Url::parse(&b.as_str().replace("7000", "7001")).unwrap();
        assert_ne!(meeting_identity(&a), meeting_identity(&c));
        assert!(meeting_identity(
            &Url::parse("https://example.com/Meetings/ViewMeeting?id=1").unwrap()
        )
        .is_none());
    }

    #[test]
    fn onbase_document_route_requires_a_published_matching_template() {
        let url = Url::parse("https://sandiego.hylandcloud.com/211agendaonlinecouncil/Meetings/ViewMeeting?id=7141&doctype=3").unwrap();
        let template = "url = '/211agendaonlinecouncil/Documents/ViewAgenda?meetingId=7141&type=AGENDATYPEVALUE&doctype=' + doctype";
        let document = hyland_document_companion(&url, template).unwrap();
        assert!(document.contains("meetingId=7141&type=summary&doctype=3"));
        assert!(meeting_identity(&Url::parse(&document).unwrap()).is_some());
        assert!(hyland_document_companion(&url, "missing endpoint").is_none());
        assert!(hyland_document_companion(&url, &template.replace("7141", "7142")).is_none());
    }
}
#[test]
fn publisher_scope_never_guesses_city_from_a_county_or_unrelated_hostname() {
    assert_eq!(
        source_jurisdiction("https://pra.sandiegocounty.gov"),
        "San Diego County"
    );
    assert_eq!(
        source_jurisdiction("https://sandiegohousingcommission.nextrequest.com"),
        "San Diego Housing Commission"
    );
    assert_eq!(
        source_jurisdiction("https://data.sandiego.gov"),
        "City of San Diego"
    );
    assert_eq!(
        source_jurisdiction("https://sandiego.gov.evil.test"),
        "Unclassified source"
    );
    assert_eq!(
        source_jurisdiction("onrecord://local/sandiego"),
        "Unclassified source"
    );
}
