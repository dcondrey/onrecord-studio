//! Fetches a tax-exempt organisation's public filings: the IRS e-file index and returns, the
//! ProPublica Nonprofit Explorer summary, and Federal Audit Clearinghouse single audits.

use crate::error::AppError;
use crate::ingestion::client::ApiClient;
use crate::ingestion::remote_zip::{read_members, RangeSource};
use crate::integrity::sha256_hex;
use crate::parsers::form990::parse_form_990;
use crate::storage::sqlite::DuckDbRepository;
use crate::types::{
    FilingLine, IndexYearCheck, OrganisationFilings, OrganisationMatch, OrganisationReturn,
    OrganisationSummary, OrganisationSummaryYear, SingleAudit, SingleAuditFinding,
    SingleAuditHistory,
};
use chrono::{Datelike, Utc};
use reqwest::Url;
use serde_json::Value;
use std::collections::BTreeMap;
use std::io::Read;

const IRS_RETURNS: &str = "https://apps.irs.gov/pub/epostcard/990/xml";
const NONPROFIT_EXPLORER: &str = "https://projects.propublica.org/nonprofits/api/v2";
const AUDIT_CLEARINGHOUSE: &str = "https://api.fac.gov";
/// The first index year whose rows name the archive holding each return. Earlier indexes
/// (2017 to 2023) list returns without it.
const FIRST_INDEX_YEAR_WITH_ARCHIVE: i32 = 2024;
const MAX_API_BYTES: usize = 8 * 1024 * 1024;
const MAX_ARCHIVE_RANGE_BYTES: usize = 64 * 1024 * 1024;
const MAX_QUERY_CHARS: usize = 200;
const ONLY_FORM_990: &str = "This return type is not read; only Form 990 is.";

fn invalid(message: impl Into<String>) -> AppError {
    AppError::DataParsing(message.into())
}

/// Nine digits, with the customary dash accepted.
pub fn normalise_ein(input: &str) -> Result<String, AppError> {
    let digits = input.trim().replace('-', "");
    if digits.len() == 9 && digits.bytes().all(|b| b.is_ascii_digit()) {
        Ok(digits)
    } else {
        Err(invalid("An EIN is nine digits, for example 33-0215585"))
    }
}

fn url(value: &str, query: &[(&str, &str)]) -> Result<Url, AppError> {
    Url::parse_with_params(value, query)
        .map_err(|error| AppError::Internal(format!("Organisation filings URL: {error}")))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexRow {
    pub ein: String,
    pub object_id: String,
    pub return_type: String,
    pub tax_period: String,
    pub taxpayer_name: String,
    pub batch_id: Option<String>,
}

/// Reads every row of one IRS index into `sink`. Columns are found by header name because the
/// index gained `XML_BATCH_ID` in 2024.
pub fn read_index<R: Read>(
    reader: R,
    mut sink: impl FnMut(IndexRow) -> Result<(), AppError>,
) -> Result<u64, AppError> {
    let mut csv = csv::ReaderBuilder::new().flexible(true).from_reader(reader);
    let headers = csv
        .headers()
        .map_err(|error| invalid(format!("The IRS index has no readable header: {error}")))?
        .clone();
    let column = |name: &str| headers.iter().position(|header| header.trim() == name);
    let required = |name: &str| {
        column(name).ok_or_else(|| invalid(format!("The IRS index has no {name} column")))
    };
    let (ein_at, object_at, type_at, period_at, name_at) = (
        required("EIN")?,
        required("OBJECT_ID")?,
        required("RETURN_TYPE")?,
        required("TAX_PERIOD")?,
        required("TAXPAYER_NAME")?,
    );
    let batch_at = column("XML_BATCH_ID");
    let mut rows = 0;
    for record in csv.byte_records() {
        let record =
            record.map_err(|error| invalid(format!("The IRS index is not readable: {error}")))?;
        let field = |at: usize| {
            String::from_utf8_lossy(record.get(at).unwrap_or_default())
                .trim()
                .to_string()
        };
        let object_id = field(object_at);
        if object_id.is_empty() || !object_id.bytes().all(|b| b.is_ascii_digit()) {
            return Err(invalid(
                "The IRS index lists a return without a numeric OBJECT_ID",
            ));
        }
        sink(IndexRow {
            ein: field(ein_at),
            object_id,
            return_type: field(type_at),
            tax_period: field(period_at),
            taxpayer_name: field(name_at),
            batch_id: batch_at.map(field).filter(|batch| {
                !batch.is_empty()
                    && batch
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_')
            }),
        })?;
        rows += 1;
    }
    Ok(rows)
}

/// Downloads one year's index (54 to 93 MB) and replaces the library's copy of that year, then
/// deletes the file. Every organisation's rows are kept, so the next organisation checked costs
/// no download for the year.
async fn refresh_index_year(
    client: &ApiClient,
    repository: &DuckDbRepository,
    year: i32,
) -> Result<IndexYearCheck, AppError> {
    let staged = std::env::temp_dir().join(format!(".irs-index.{}.partial", uuid::Uuid::new_v4()));
    let index_url = format!("{IRS_RETURNS}/{year}/index_{year}.csv");
    let outcome = async {
        client
            .download_url_to_path(url(&index_url, &[])?, &staged)
            .await?;
        repository
            .replace_irs_index_year(year, &staged, &index_url)
            .await
    }
    .await;
    let _ = tokio::fs::remove_file(&staged).await;
    outcome
}

/// How long a copy of the current or previous year's index is used before it is fetched again.
/// The IRS adds to an index through the year after it; older years are read once.
const INDEX_REFRESH_DAYS: i64 = 7;

async fn index_year(
    client: &ApiClient,
    repository: &DuckDbRepository,
    year: i32,
    now: chrono::DateTime<Utc>,
) -> Result<IndexYearCheck, AppError> {
    if let Some(held) = repository.irs_index_year(year).await? {
        let settled = year < now.year() - 1;
        let fresh = chrono::DateTime::parse_from_rfc3339(&held.checked_at)
            .map(|at| now.signed_duration_since(at).num_days() < INDEX_REFRESH_DAYS)
            .unwrap_or(false);
        if settled || fresh {
            return Ok(held);
        }
    }
    refresh_index_year(client, repository, year).await
}

fn parse_content_range(value: &str) -> Option<(u64, u64, u64)> {
    let (span, total) = value.strip_prefix("bytes ")?.split_once('/')?;
    let (first, last) = span.split_once('-')?;
    Some((first.parse().ok()?, last.parse().ok()?, total.parse().ok()?))
}

struct HttpRange<'a> {
    client: &'a ApiClient,
    url: Url,
}

impl HttpRange<'_> {
    async fn fetch(&self, range: String, expect: u64) -> Result<(Vec<u8>, u64, u64), AppError> {
        let limit = usize::try_from(expect)
            .ok()
            .filter(|&bytes| bytes <= MAX_ARCHIVE_RANGE_BYTES)
            .ok_or_else(|| invalid("A requested archive range exceeds the 64 MB limit"))?;
        let response = self
            .client
            .get_direct(
                self.url.clone(),
                // A recompressed body would make the byte offsets meaningless.
                &[("Range", &range), ("Accept-Encoding", "identity")],
                limit,
            )
            .await?;
        let refused = || {
            AppError::PortalDiscovery(format!(
                "'{}' did not answer the byte range that was asked for",
                self.url
            ))
        };
        let (first, last, total) = response
            .content_range
            .as_deref()
            .filter(|_| response.status == 206)
            .and_then(parse_content_range)
            .ok_or_else(refused)?;
        if last < first || response.bytes.len() as u64 != last - first + 1 || last >= total {
            return Err(refused());
        }
        Ok((response.bytes, first, total))
    }
}

impl RangeSource for HttpRange<'_> {
    async fn span(&self, first: u64, last: u64) -> Result<Vec<u8>, AppError> {
        let (bytes, got_first, _) = self
            .fetch(format!("bytes={first}-{last}"), last - first + 1)
            .await?;
        if got_first != first || bytes.len() as u64 != last - first + 1 {
            return Err(AppError::PortalDiscovery(format!(
                "'{}' answered a different byte range than was asked for",
                self.url
            )));
        }
        Ok(bytes)
    }

    async fn suffix(&self, length: u64) -> Result<(Vec<u8>, u64), AppError> {
        let (bytes, _, total) = self.fetch(format!("bytes=-{length}"), length).await?;
        Ok((bytes, total))
    }
}

/// Reads the named returns out of one batch archive: each return's member name and XML, `None`
/// for a return the archive does not hold.
async fn fetch_returns(
    client: &ApiClient,
    archive_url: &str,
    object_ids: &[String],
) -> Result<Vec<Option<(String, Vec<u8>)>>, AppError> {
    let names = object_ids
        .iter()
        .map(|object_id| format!("{object_id}_public.xml"))
        .collect::<Vec<_>>();
    let source = HttpRange {
        client,
        url: url(archive_url, &[])?,
    };
    read_members(&source, &names).await
}

async fn get_json(
    client: &ApiClient,
    url: Url,
    headers: &[(&'static str, &str)],
    source: &str,
) -> Result<Value, AppError> {
    let response = client.get_direct(url, headers, MAX_API_BYTES).await?;
    serde_json::from_slice(&response.bytes)
        .map_err(|error| invalid(format!("{source} returned unreadable JSON: {error}")))
}

fn string(value: &Value, key: &str) -> Option<String> {
    match value.get(key)? {
        Value::String(text) if !text.trim().is_empty() => Some(text.trim().to_string()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

fn whole_amount(value: &Value, key: &str) -> Option<i64> {
    value.get(key)?.as_i64()
}

fn padded_ein(value: &Value) -> Option<String> {
    value.get("ein")?.as_u64().map(|ein| format!("{ein:09}"))
}

pub async fn search_organisations(
    client: &ApiClient,
    query: &str,
) -> Result<Vec<OrganisationMatch>, AppError> {
    let query = query.trim();
    if query.is_empty() || query.chars().count() > MAX_QUERY_CHARS {
        return Err(invalid(format!(
            "An organisation search needs between 1 and {MAX_QUERY_CHARS} characters"
        )));
    }
    let body = get_json(
        client,
        url(
            &format!("{NONPROFIT_EXPLORER}/search.json"),
            &[("q", query)],
        )?,
        &[],
        "ProPublica Nonprofit Explorer",
    )
    .await?;
    Ok(body
        .get("organizations")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|organisation| {
            Some(OrganisationMatch {
                ein: padded_ein(organisation)?,
                name: string(organisation, "name")?,
                city: string(organisation, "city"),
                state: string(organisation, "state"),
            })
        })
        .collect())
}

fn summary_from(body: &Value, source_url: String, retrieved_at: String) -> OrganisationSummary {
    let organisation = body.get("organization").unwrap_or(&Value::Null);
    let mut years = body
        .get("filings_with_data")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|filing| {
            Some(OrganisationSummaryYear {
                tax_year: i32::try_from(filing.get("tax_prd_yr")?.as_i64()?).ok()?,
                tax_period: string(filing, "tax_prd"),
                form_type: match filing.get("formtype").and_then(Value::as_i64) {
                    Some(0) => Some("990".to_string()),
                    Some(1) => Some("990EZ".to_string()),
                    Some(2) => Some("990PF".to_string()),
                    _ => None,
                },
                total_revenue: whole_amount(filing, "totrevenue"),
                total_expenses: whole_amount(filing, "totfuncexpns"),
                total_assets_end_of_year: whole_amount(filing, "totassetsend"),
                total_liabilities_end_of_year: whole_amount(filing, "totliabend"),
                officer_compensation: whole_amount(filing, "compnsatncurrofcr"),
            })
        })
        .collect::<Vec<_>>();
    years.sort_by(|a, b| {
        b.tax_year
            .cmp(&a.tax_year)
            .then(b.tax_period.cmp(&a.tax_period))
    });
    OrganisationSummary {
        name: string(organisation, "name"),
        city: string(organisation, "city"),
        state: string(organisation, "state"),
        years,
        source_url,
        retrieved_at,
    }
}

async fn summary(client: &ApiClient, ein: &str) -> Result<OrganisationSummary, AppError> {
    let api_url = url(
        &format!("{NONPROFIT_EXPLORER}/organizations/{ein}.json"),
        &[],
    )?;
    let body = get_json(client, api_url, &[], "ProPublica Nonprofit Explorer").await?;
    Ok(summary_from(
        &body,
        format!("https://projects.propublica.org/nonprofits/organizations/{ein}"),
        Utc::now().to_rfc3339(),
    ))
}

fn recorded_lines(row: &Value, labelled: &[(&str, &str)]) -> Vec<FilingLine> {
    labelled
        .iter()
        .filter_map(|(key, label)| {
            Some(FilingLine {
                label: (*label).to_string(),
                value: string(row, key)?,
            })
        })
        .collect()
}

fn audits_from(general: &Value, findings: &Value) -> Vec<SingleAudit> {
    let mut by_report: BTreeMap<String, Vec<SingleAuditFinding>> = BTreeMap::new();
    for finding in findings.as_array().into_iter().flatten() {
        let Some(report_id) = string(finding, "report_id") else {
            continue;
        };
        by_report
            .entry(report_id)
            .or_default()
            .push(SingleAuditFinding {
                reference_number: string(finding, "reference_number"),
                compliance_requirement: string(finding, "type_requirement"),
                lines: recorded_lines(
                    finding,
                    &[
                        ("is_material_weakness", "Material weakness"),
                        ("is_significant_deficiency", "Significant deficiency"),
                        ("is_modified_opinion", "Modified opinion"),
                        ("is_other_findings", "Other findings"),
                        ("is_other_matters", "Other matters"),
                        ("is_questioned_costs", "Questioned costs"),
                        ("is_repeat_finding", "Repeat finding"),
                    ],
                ),
            });
    }
    general
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|audit| {
            let report_id = string(audit, "report_id")?;
            Some(SingleAudit {
                findings: by_report.remove(&report_id).unwrap_or_default(),
                report_id,
                audit_year: string(audit, "audit_year"),
                fiscal_year_end: string(audit, "fy_end_date"),
                accepted_date: string(audit, "fac_accepted_date"),
                auditor: string(audit, "auditor_firm_name"),
                federal_amount_expended: whole_amount(audit, "total_amount_expended"),
                lines: recorded_lines(
                    audit,
                    &[
                        ("gaap_results", "Opinion on the financial statements"),
                        (
                            "is_going_concern_included",
                            "Going concern paragraph included",
                        ),
                        (
                            "is_internal_control_deficiency_disclosed",
                            "Significant deficiency in internal control disclosed",
                        ),
                        (
                            "is_internal_control_material_weakness_disclosed",
                            "Material weakness in internal control disclosed",
                        ),
                        (
                            "is_material_noncompliance_disclosed",
                            "Material noncompliance disclosed",
                        ),
                        ("is_low_risk_auditee", "Low-risk auditee"),
                    ],
                ),
            })
        })
        .collect()
}

async fn single_audits(
    client: &ApiClient,
    ein: &str,
    key: &str,
) -> Result<SingleAuditHistory, AppError> {
    if key.is_empty() || key.len() > 128 || !key.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(invalid(
            "The Federal Audit Clearinghouse key must be 1 to 128 printable characters",
        ));
    }
    let headers = [("X-Api-Key", key)];
    let source = "The Federal Audit Clearinghouse";
    let general = get_json(
        client,
        url(
            &format!("{AUDIT_CLEARINGHOUSE}/general"),
            &[
                ("auditee_ein", &format!("eq.{ein}")),
                ("order", "audit_year.desc"),
            ],
        )?,
        &headers,
        source,
    )
    .await?;
    let report_ids = general
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|audit| string(audit, "report_id"))
        .filter(|id| id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'))
        .collect::<Vec<_>>();
    let findings = if report_ids.is_empty() {
        Value::Array(Vec::new())
    } else {
        get_json(
            client,
            url(
                &format!("{AUDIT_CLEARINGHOUSE}/findings"),
                &[("report_id", &format!("in.({})", report_ids.join(",")))],
            )?,
            &headers,
            source,
        )
        .await?
    };
    Ok(SingleAuditHistory {
        audits: audits_from(&general, &findings),
        source_url: format!("https://app.fac.gov/dissemination/search/?auditee_ein={ein}"),
        retrieved_at: Utc::now().to_rfc3339(),
    })
}

fn read_statement(listed: &mut OrganisationReturn, xml: &str) {
    match parse_form_990(xml) {
        Ok(statement) => {
            listed.statement = Some(statement);
            listed.unread_reason = None;
        }
        Err(error) => {
            listed.statement = None;
            listed.unread_reason = Some(error.to_string());
        }
    }
}

/// Fetches what is not yet held for `ein`, stores it, and returns the whole record. A source that
/// fails leaves a warning and whatever an earlier check stored from it.
pub async fn check_organisation(
    client: &ApiClient,
    repository: &DuckDbRepository,
    ein: &str,
    audit_key: Option<&str>,
) -> Result<OrganisationFilings, AppError> {
    let ein = normalise_ein(ein)?;
    let previous = repository.organisation_filings_document(&ein).await?;
    let now = Utc::now();
    let mut warnings = Vec::new();
    let mut index_years = Vec::new();
    let mut returns = Vec::new();

    for year in FIRST_INDEX_YEAR_WITH_ARCHIVE..=now.year() {
        let held = match index_year(client, repository, year, now).await {
            Ok(held) => held,
            Err(error) => {
                warnings.push(format!("IRS index for {year} was not read: {error}"));
                // What an earlier check read from this year stands until the index is read again.
                if let Some(previous) = &previous {
                    if let Some(held) = previous.index_years.iter().find(|c| c.year == year) {
                        index_years.push(held.clone());
                    }
                    returns.extend(
                        previous
                            .returns
                            .iter()
                            .filter(|r| r.index_year == year)
                            .cloned(),
                    );
                }
                continue;
            }
        };
        let rows = repository.irs_index_rows(year, &ein).await?;
        index_years.push(IndexYearCheck {
            returns_listed: rows.len() as u32,
            ..held
        });
        returns.extend(rows.into_iter().map(|row| {
            OrganisationReturn {
                // The index writes some archive names in lower case (`2024_TEOS_XML_05a`) where
                // the published file is upper case.
                archive_url: row.batch_id.as_ref().map(|batch| {
                    format!("{IRS_RETURNS}/{year}/{}.zip", batch.to_ascii_uppercase())
                }),
                member_name: None,
                object_id: row.object_id,
                index_year: year,
                return_type: row.return_type,
                tax_period: row.tax_period,
                taxpayer_name: row.taxpayer_name,
                xml_sha256: None,
                retrieved_at: None,
                statement: None,
                unread_reason: None,
            }
        }));
    }

    let mut to_fetch: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for listed in &mut returns {
        if listed.return_type != "990" {
            listed.unread_reason = Some(ONLY_FORM_990.to_string());
            continue;
        }
        if let Some(stored) = repository
            .organisation_return_xml(&listed.object_id)
            .await?
        {
            listed.member_name = Some(stored.member_name);
            listed.xml_sha256 = Some(stored.sha256);
            listed.retrieved_at = Some(stored.retrieved_at);
            read_statement(listed, &stored.xml);
            continue;
        }
        match &listed.archive_url {
            Some(archive_url) => to_fetch
                .entry(archive_url.clone())
                .or_default()
                .push(listed.object_id.clone()),
            None => {
                listed.unread_reason =
                    Some("The IRS index names no archive for this return.".to_string());
            }
        }
    }
    for (archive_url, object_ids) in to_fetch {
        let fetched = match fetch_returns(client, &archive_url, &object_ids).await {
            Ok(fetched) => fetched,
            Err(error) => {
                warnings.push(format!("IRS archive {archive_url} was not read: {error}"));
                for listed in returns
                    .iter_mut()
                    .filter(|r| object_ids.contains(&r.object_id))
                {
                    listed.unread_reason = Some("The IRS archive could not be read.".to_string());
                }
                continue;
            }
        };
        for (object_id, fetched) in object_ids.iter().zip(fetched) {
            let Some(listed) = returns.iter_mut().find(|r| &r.object_id == object_id) else {
                continue;
            };
            let Some((member_name, xml)) =
                fetched.and_then(|(name, bytes)| Some((name, String::from_utf8(bytes).ok()?)))
            else {
                listed.unread_reason =
                    Some("The IRS archive holds no readable return under this id.".to_string());
                continue;
            };
            let sha256 = sha256_hex(xml.as_bytes());
            let retrieved_at = Utc::now().to_rfc3339();
            repository
                .save_organisation_return_xml(
                    object_id,
                    &ein,
                    &member_name,
                    &sha256,
                    &retrieved_at,
                    &xml,
                )
                .await?;
            listed.member_name = Some(member_name);
            listed.xml_sha256 = Some(sha256);
            listed.retrieved_at = Some(retrieved_at);
            read_statement(listed, &xml);
        }
    }
    returns.sort_by(|a, b| {
        b.tax_period
            .cmp(&a.tax_period)
            .then(b.object_id.cmp(&a.object_id))
    });

    let summary = match summary(client, &ein).await {
        Ok(summary) => Some(summary),
        Err(error) => {
            warnings.push(format!(
                "ProPublica Nonprofit Explorer summary was not read: {error}"
            ));
            previous.as_ref().and_then(|p| p.summary.clone())
        }
    };
    let held_audits = previous.as_ref().and_then(|p| p.single_audits.clone());
    let single_audits = match audit_key.map(str::trim).filter(|key| !key.is_empty()) {
        None => held_audits,
        Some(key) => match single_audits(client, &ein, key).await {
            Ok(audits) => Some(audits),
            Err(error) => {
                warnings.push(format!("Federal Audit Clearinghouse was not read: {error}"));
                held_audits
            }
        },
    };
    let name = summary
        .as_ref()
        .and_then(|s| s.name.clone())
        .or_else(|| returns.first().map(|r| r.taxpayer_name.clone()))
        .or_else(|| previous.as_ref().and_then(|p| p.name.clone()));
    let document = OrganisationFilings {
        ein: ein.clone(),
        name,
        checked_at: now.to_rfc3339(),
        index_years,
        returns,
        summary,
        single_audits,
        warnings,
        public_money: Vec::new(),
        public_money_notes: Vec::new(),
    };
    repository.save_organisation_filings(&document).await?;
    repository
        .organisation_filings(&ein)
        .await?
        .ok_or_else(|| AppError::Storage("The organisation record was not stored".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_reader_takes_both_header_shapes_and_refuses_a_row_without_an_id() {
        let with_batch = "RETURN_ID,FILING_TYPE,EIN,TAX_PERIOD,SUB_DATE,TAXPAYER_NAME,RETURN_TYPE,DLN,OBJECT_ID,XML_BATCH_ID\n\
22398228,EFILE,630809530,202312,2024,WALKER COUNTY HUMANE SOCIETY,990EZ,93492022012314,202410229349201231,2024_TEOS_XML_01A\n\
,EFILE,330215585,202306,2024,\"ALPHA PROJECT, INC\",990,1,202411359349306146,2024_TEOS_XML_05a\n";
        let mut rows = Vec::new();
        let count = read_index(with_batch.as_bytes(), |row| {
            rows.push(row);
            Ok(())
        })
        .unwrap();
        assert_eq!(count, 2);
        assert_eq!(
            rows[1],
            IndexRow {
                ein: "330215585".into(),
                object_id: "202411359349306146".into(),
                return_type: "990".into(),
                tax_period: "202306".into(),
                taxpayer_name: "ALPHA PROJECT, INC".into(),
                batch_id: Some("2024_TEOS_XML_05a".into()),
            }
        );
        let without_batch = "RETURN_ID,FILING_TYPE,EIN,TAX_PERIOD,SUB_DATE,TAXPAYER_NAME,RETURN_TYPE,DLN,OBJECT_ID\n\
21319700,EFILE,330215585,202112,2023,ALPHA PROJECT FOR THE HOMELESS,990,93493018011043,202340189349301104\n";
        let mut rows = Vec::new();
        read_index(without_batch.as_bytes(), |row| {
            rows.push(row);
            Ok(())
        })
        .unwrap();
        assert_eq!(rows[0].batch_id, None);
        assert!(read_index("EIN,TAX_PERIOD\n1,2\n".as_bytes(), |_| Ok(())).is_err());
        assert!(read_index(
            "EIN,TAX_PERIOD,TAXPAYER_NAME,RETURN_TYPE,OBJECT_ID\n1,2,n,990,abc\n".as_bytes(),
            |_| Ok(())
        )
        .is_err());
    }

    #[test]
    fn ein_accepts_the_dash_and_nothing_else() {
        assert_eq!(normalise_ein(" 33-0215585 ").unwrap(), "330215585");
        for bad in ["33021558", "3302155850", "33-02I5585", ""] {
            assert!(normalise_ein(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn content_range_is_read_only_in_its_complete_form() {
        assert_eq!(parse_content_range("bytes 5-9/100"), Some((5, 9, 100)));
        assert_eq!(parse_content_range("bytes 5-9/*"), None);
        assert_eq!(parse_content_range("bytes */100"), None);
    }

    #[test]
    fn summary_and_audits_keep_absent_values_absent() {
        let body = serde_json::json!({
            "organization": {"ein": 330215585, "name": "Alpha Project For The Homeless", "city": "San Diego", "state": "CA"},
            "filings_with_data": [
                {"tax_prd_yr": 2022, "tax_prd": 202206, "formtype": 0, "totrevenue": 10, "totfuncexpns": 9, "totassetsend": 8, "totliabend": 7, "compnsatncurrofcr": 6},
                {"tax_prd_yr": 2023, "tax_prd": 202306, "formtype": 0, "totrevenue": 41418753, "totfuncexpns": null}
            ]
        });
        let summary = summary_from(&body, "u".into(), "t".into());
        assert_eq!(summary.years[0].tax_year, 2023);
        assert_eq!(summary.years[0].total_revenue, Some(41_418_753));
        assert_eq!(summary.years[0].total_expenses, None);
        assert_eq!(summary.years[0].officer_compensation, None);

        let general = serde_json::json!([
            {"report_id": "2023-06-GSAFAC-0000033715", "audit_year": "2023", "fy_end_date": "2023-06-30", "total_amount_expended": 1225257, "gaap_results": "unmodified_opinion", "is_going_concern_included": "No", "is_low_risk_auditee": ""}
        ]);
        let findings = serde_json::json!([
            {"report_id": "2023-06-GSAFAC-0000033715", "reference_number": "2023-001", "type_requirement": "B", "is_questioned_costs": "Y", "is_repeat_finding": "N"},
            {"report_id": "another", "reference_number": "x"}
        ]);
        let audits = audits_from(&general, &findings);
        assert_eq!(audits.len(), 1);
        assert_eq!(audits[0].federal_amount_expended, Some(1_225_257));
        assert_eq!(audits[0].lines.len(), 2);
        assert_eq!(audits[0].lines[0].value, "unmodified_opinion");
        assert_eq!(audits[0].findings.len(), 1);
        assert_eq!(audits[0].findings[0].lines[0].label, "Questioned costs");
        assert_eq!(audits[0].findings[0].lines[0].value, "Y");
    }

    /// The whole check against the live sources, ignored by default (several hundred MB):
    /// `ONRECORD_EVAL_EIN=330215585 ONRECORD_EVAL_FAC_KEY=<key> cargo test --lib
    /// live_organisation_check -- --ignored --nocapture`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "downloads IRS indexes and archive ranges from the live sources"]
    async fn live_organisation_check() {
        let ein = std::env::var("ONRECORD_EVAL_EIN").expect("ONRECORD_EVAL_EIN");
        let key = std::env::var("ONRECORD_EVAL_FAC_KEY").ok();
        let temp = tempfile::tempdir().unwrap();
        let repository = DuckDbRepository::new(temp.path().join("core.db")).unwrap();
        repository
            .run_migrations(include_str!("../../migrations/0001_initial_schema.sql"))
            .await
            .unwrap();
        let client = ApiClient::new(Default::default()).unwrap();
        let found = search_organisations(&client, "alpha project for the homeless")
            .await
            .unwrap();
        println!("search: {} matches, first {:?}", found.len(), found.first());
        let filings = check_organisation(&client, &repository, &ein, key.as_deref())
            .await
            .unwrap();
        println!("name {:?} warnings {:?}", filings.name, filings.warnings);
        println!("index years {:?}", filings.index_years);
        for listed in &filings.returns {
            let statement = listed.statement.as_ref();
            println!(
                "{} {} {} sha {:?} unread {:?} revenue {:?} diversion {:?} former {:?} people {} J {} severance {:?} L {}",
                listed.tax_period,
                listed.return_type,
                listed.object_id,
                listed.xml_sha256.as_deref().map(|h| &h[..12]),
                listed.unread_reason,
                statement.and_then(|s| s.total_revenue),
                statement.and_then(|s| s.significant_diversion_reported),
                statement.and_then(|s| s.former_listed),
                statement.map_or(0, |s| s.people.len()),
                statement.map_or(0, |s| s.schedule_j_people.len()),
                statement.and_then(|s| s.severance_payment_reported),
                statement.map_or(0, |s| s.schedule_l_transactions.len()),
            );
        }
        println!(
            "summary years {:?}",
            filings.summary.as_ref().map(|s| s
                .years
                .iter()
                .map(|y| y.tax_year)
                .collect::<Vec<_>>())
        );
        for audit in filings.single_audits.iter().flat_map(|h| &h.audits) {
            println!(
                "audit {} {:?} findings {}",
                audit.report_id,
                audit.federal_amount_expended,
                audit.findings.len()
            );
        }
        assert!(filings
            .returns
            .iter()
            .all(|listed| listed.return_type != "990" || listed.statement.is_some()));
        let again = check_organisation(&client, &repository, &ein, None)
            .await
            .unwrap();
        assert_eq!(again.returns, filings.returns);
        assert_eq!(again.single_audits, filings.single_audits);
    }
}
