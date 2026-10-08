//! Spending cases in the findings inbox.
//!
//! Runs the financial investigation for each fund at its retrieved fiscal years and lifts the
//! cases `analysis::spending_cases` raises into `Finding`s. A case is cited by the exact imported
//! rows it compares, each resolved to the document the dataset was imported from (matched on the
//! snapshot's source hash and URL) and rendered in the citation quote, with the query id that
//! produced it in the label. A legal-claim case whose rows cannot all be tied to an origin document
//! is not recorded: it becomes an evidence gap, never a weaker finding.
//!
//! Rules come from documents classified as policies, extracted by word-token phrase matching, and
//! apply only to a dataset from the same jurisdiction. A policy's own stated effective date (or,
//! failing that, its upload date) bounds which rows the rule can be held against.

use super::public_documents::{load_public_documents, PublicDocument};
use crate::{
    analysis::{
        document_class::{classify, DocumentClass},
        finding::{
            finding_id, Finding, FindingCitation, FindingConfidence, FindingKind, FindingSeverity,
            FindingSubject,
        },
        gaps::{EvidenceGap, GapRole},
        spending_cases::{
            self, PolicyRule, RuleDocument, SpendingCase, APPROPRIATION_EXCEEDED,
            APPROVAL_RECORD_ABSENT, APPROVAL_THRESHOLD_EXCEEDED, CEILING_EXCEEDED, COUNTED,
            PAYMENT_OUTSIDE_TERM, PAYMENT_WITHOUT_CONTRACT, RESTRICTED_OFF_PROGRAM,
            SPLIT_PURCHASE_PATTERN, SPLIT_PURCHASE_PROHIBITED,
        },
        spending_rules::{extract_rules, RuleKind},
    },
    error::AppError,
    ingestion::regional::source_jurisdiction,
    storage::DuckDbRepository,
    types::{
        FinancialAuditFinding, FinancialCitation, FinancialGap, FinancialReport, FinancialScope,
        FinancialWorkspace,
    },
};
use chrono::Utc;
use duckdb::params;
use std::collections::{BTreeMap, BTreeSet, HashMap};

pub const SLUG: &str = "spending_cases";

const MAX_DOCUMENTS: i64 = 5_000;
/// Each scope costs one financial investigation.
const MAX_SCOPES: usize = 12;
const MAX_FINDINGS: usize = 200;
const QUOTE_CHARACTERS: usize = 400;
const RULE_QUOTE_CHARACTERS: usize = 300;

/// Kinds whose comparison does not depend on one fiscal year: contracts, amendments and payments
/// are retrieved cumulatively through the scope's year, so only the newest year per fund is used.
fn cumulative(kind: &str) -> bool {
    kind != APPROPRIATION_EXCEEDED && kind != RESTRICTED_OFF_PROGRAM
}

fn finding_kind(kind: &str) -> Option<FindingKind> {
    Some(match kind {
        CEILING_EXCEEDED => FindingKind::ContractCeilingExceeded,
        PAYMENT_OUTSIDE_TERM => FindingKind::PaymentOutsideContractTerm,
        PAYMENT_WITHOUT_CONTRACT => FindingKind::PaymentWithoutContractRecord,
        APPROPRIATION_EXCEEDED => FindingKind::AppropriationExceeded,
        RESTRICTED_OFF_PROGRAM => FindingKind::RestrictedFundOffProgram,
        APPROVAL_THRESHOLD_EXCEEDED => FindingKind::ApprovalThresholdExceeded,
        APPROVAL_RECORD_ABSENT => FindingKind::ApprovalRecordAbsent,
        SPLIT_PURCHASE_PATTERN => FindingKind::SplitPurchasePattern,
        SPLIT_PURCHASE_PROHIBITED => FindingKind::SplitPurchaseProhibitedByPolicy,
        _ => return None,
    })
}

/// One evaluated scope: the report the rows came from and the cases raised against it.
struct Evaluated {
    report: FinancialReport,
    cases: Vec<SpendingCase>,
}

pub struct Sweep {
    evaluated: Vec<Evaluated>,
    rules: Vec<PolicyRule>,
    pub gaps: Vec<FinancialGap>,
}

fn chars_before(text: &str, byte: usize) -> usize {
    text.get(..byte).map_or(0, |prefix| prefix.chars().count())
}

fn truncate(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}

/// Policy rules from documents classified as policies. A document states its own effective date
/// or, without one, stands in by its upload date.
fn policy_rules(documents: &[PublicDocument]) -> Vec<(PolicyRule, String)> {
    let mut rules = Vec::new();
    for document in documents {
        if classify(document.file_name.as_deref(), &document.text) != Some(DocumentClass::Policy) {
            continue;
        }
        let Some(effective) = document
            .stated_effective_date()
            .or_else(|| document.posted_date())
        else {
            continue;
        };
        let name = document.name();
        for rule in extract_rules(&document.text) {
            let Some(sentence) = document.text.get(rule.byte_start..rule.byte_end) else {
                continue;
            };
            let quote = truncate(sentence.trim(), RULE_QUOTE_CHARACTERS);
            let char_start = chars_before(&document.text, rule.byte_start);
            let char_end = chars_before(&document.text, rule.byte_end);
            rules.push((
                PolicyRule {
                    rule,
                    document: RuleDocument {
                        portal_key: document.portal_key.clone(),
                        document_id: document.document_id,
                        name: name.clone(),
                        quote,
                        char_start,
                        char_end,
                        effective,
                        effective_stated: stated.is_some(),
                    },
                },
                document.portal_key.clone(),
            ));
        }
    }
    rules
}

fn dataset_jurisdiction(workspace: &FinancialWorkspace, role: &str) -> Option<&'static str> {
    let mapping = workspace.mappings.iter().find(|m| m.role == role)?;
    let dataset = workspace
        .datasets
        .iter()
        .find(|d| d.snapshot_id == mapping.snapshot_id)?;
    let jurisdiction = source_jurisdiction(dataset.source_url.as_deref()?);
    (jurisdiction != source_jurisdiction("")).then_some(jurisdiction)
}

/// A rule binds only to a role whose dataset comes from the policy's own jurisdiction.
fn bind_rules(
    rules: Vec<(PolicyRule, String)>,
    workspace: &FinancialWorkspace,
    portal_jurisdictions: &HashMap<String, &'static str>,
) -> (Vec<PolicyRule>, usize) {
    let payment = dataset_jurisdiction(workspace, "payment");
    let contract = dataset_jurisdiction(workspace, "contract");
    let unclassified = source_jurisdiction("");
    let mut bound = Vec::new();
    let mut unbound = 0;
    for (mut policy, portal) in rules {
        let jurisdiction = portal_jurisdictions
            .get(&portal)
            .copied()
            .filter(|j| *j != unclassified);
        let payments = policy.rule.payments && jurisdiction.is_some() && jurisdiction == payment;
        let contracts = policy.rule.contracts && jurisdiction.is_some() && jurisdiction == contract;
        let splitting = policy.rule.kind == RuleKind::SplittingProhibited;
        if payments || contracts || (splitting && jurisdiction.is_some() && jurisdiction == payment)
        {
            policy.rule.payments = payments;
            policy.rule.contracts = contracts;
            bound.push(policy);
        } else if policy.rule.limit.is_some()
            || matches!(policy.rule.kind, RuleKind::OvertimeAuthorization)
        {
            unbound += 1;
        }
    }
    (bound, unbound)
}

pub async fn sweep(repository: &DuckDbRepository) -> Result<Sweep, AppError> {
    let workspace = repository.financial_workspace().await?;
    let mut result = Sweep {
        evaluated: Vec::new(),
        rules: Vec::new(),
        gaps: Vec::new(),
    };
    if workspace.mappings.is_empty() {
        return Ok(result);
    }
    let portal_jurisdictions: HashMap<String, &'static str> = repository
        .list_portal_registrations()
        .await?
        .into_iter()
        .map(|source| {
            (
                source.registration.portal_key,
                source_jurisdiction(&source.registration.canonical_url),
            )
        })
        .collect();
    let documents = load_public_documents(repository, MAX_DOCUMENTS).await?;
    let (rules, unbound) = bind_rules(policy_rules(&documents), &workspace, &portal_jurisdictions);
    if unbound > 0 {
        result.gaps.push(FinancialGap {
            title: "Jurisdiction of the policy and the ledger".into(),
            records_needed: "A source URL on the payment or contract dataset, or a policy from the same jurisdiction as the ledger.".into(),
            why: format!("{unbound} spending rule(s) in policy documents could not be tied to the payment or contract dataset because the two come from different or unrecognised jurisdictions."),
            citations: vec![],
        });
    }
    // Scopes: newest years first, every fund, bounded.
    let mut newest: BTreeMap<String, i32> = BTreeMap::new();
    let mut scopes = Vec::new();
    for year in &workspace.fiscal_years {
        for fund in &workspace.funds {
            if scopes.len() >= MAX_SCOPES {
                break;
            }
            newest.entry(fund.clone()).or_insert(*year);
            scopes.push((*year, fund.clone()));
        }
    }
    for (year, fund) in scopes {
        let report = repository
            .run_financial_investigation(FinancialScope {
                fiscal_year: Some(year),
                fund: Some(fund.clone()),
                department: None,
            })
            .await?;
        let is_newest = newest.get(&fund) == Some(&year);
        let mut cases: Vec<SpendingCase> = report
            .findings
            .iter()
            .filter(|item| finding_kind(&item.kind).is_some())
            .filter(|item| is_newest || !cumulative(&item.kind))
            .map(|item| SpendingCase {
                finding: item.clone(),
                rules: vec![],
            })
            .collect();
        if is_newest {
            let outcome =
                spending_cases::rule_cases(&report.records, &report.incomplete_roles, &rules);
            cases.extend(outcome.cases);
            result.gaps.extend(outcome.gaps);
        }
        for gap in &report.gaps {
            if spending_cases::gap_role(&gap.title).is_some() {
                result.gaps.push(gap.clone());
            }
        }
        result.evaluated.push(Evaluated { report, cases });
    }
    result.rules = rules;
    let mut seen = BTreeSet::new();
    result.gaps.retain(|gap| seen.insert(gap.title.clone()));
    Ok(result)
}

/// Origin document of a dataset: the ingested file whose hash and URL the snapshot recorded.
async fn origin_documents(
    repository: &DuckDbRepository,
    sources: BTreeSet<(String, Option<String>)>,
) -> Result<HashMap<(String, Option<String>), (String, i64)>, AppError> {
    repository
        .query_connection("Spending case origin documents", move |conn| {
            let mut found = HashMap::new();
            let mut statement = conn
                .prepare(
                    "SELECT portal_subdomain, id FROM documents
                     WHERE raw_file_sha256 = ?1 AND download_url IS NOT DISTINCT FROM ?2
                     ORDER BY id LIMIT 1",
                )
                .map_err(|e| AppError::Storage(e.to_string()))?;
            for (sha, url) in sources {
                let row = statement.query_row(params![sha, url], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
                });
                if let Ok(origin) = row {
                    found.insert((sha, url), origin);
                }
            }
            Ok(found)
        })
        .await
}

fn row_quote(citation: &FinancialCitation) -> String {
    let cells: Vec<String> = citation
        .columns
        .iter()
        .zip(&citation.row)
        .filter(|(column, value)| !column.starts_with('_') && !value.is_null())
        .map(|(column, value)| {
            let text = value
                .as_str()
                .map_or_else(|| value.to_string(), str::to_string);
            format!("{column}={text}")
        })
        .collect();
    truncate(&cells.join("; "), QUOTE_CHARACTERS)
}

fn short(id: &str) -> &str {
    id.get(..8).unwrap_or(id)
}

fn lift(
    case: &SpendingCase,
    report: &FinancialReport,
    rules: &[PolicyRule],
    origins: &HashMap<(String, Option<String>), (String, i64)>,
    gaps: &mut Vec<FinancialGap>,
) -> Option<Finding> {
    let item: &FinancialAuditFinding = &case.finding;
    let kind = finding_kind(&item.kind)?;
    let legal = kind.is_legal_claim();
    let by_id: HashMap<&str, &FinancialCitation> = report
        .evidence
        .iter()
        .map(|citation| (citation.id.as_str(), citation))
        .collect();
    let mut citations = Vec::new();
    let mut queries = BTreeSet::new();
    let mut unresolved = 0;
    let mut portal_key = None;
    for rule_index in &case.rules {
        let document = &rules[*rule_index].document;
        portal_key.get_or_insert_with(|| document.portal_key.clone());
        citations.push(FindingCitation {
            portal_key: document.portal_key.clone(),
            document_id: document.document_id,
            character_start: i64::try_from(document.char_start).ok(),
            character_end: i64::try_from(document.char_end).ok(),
            quote: Some(document.quote.clone()),
            label: Some(format!("Rule stated in '{}'", document.name)),
        });
    }
    for id in &item.citations {
        let Some(citation) = by_id.get(id.as_str()) else {
            unresolved += 1;
            continue;
        };
        queries.insert(citation.query_id.clone());
        let origin = origins.get(&(citation.source_sha256.clone(), citation.source_url.clone()));
        let Some((portal, document_id)) = origin else {
            unresolved += 1;
            continue;
        };
        portal_key.get_or_insert_with(|| portal.clone());
        citations.push(FindingCitation {
            portal_key: portal.clone(),
            document_id: *document_id,
            character_start: None,
            character_end: None,
            quote: Some(row_quote(citation)),
            label: Some(format!(
                "{} row {} · query {} · snapshot {}",
                citation.role,
                short(&citation.id),
                short(&citation.query_id),
                short(&citation.snapshot_id)
            )),
        });
    }
    if legal && unresolved > 0 {
        gaps.push(FinancialGap {
            title: "Origin document for cited ledger rows".into(),
            records_needed: "The ingested file each dataset was imported from (import through the source catalog so the snapshot keeps its document), or the original export.".into(),
            why: format!("{unresolved} row(s) behind a {} case cannot be tied to an ingested document, so the case is withheld rather than recorded with partial evidence.", kind.as_str()),
            citations: vec![],
        });
        return None;
    }
    let confidence = item.confidence_basis_points?;
    let subject = FindingSubject {
        portal_key,
        ..Default::default()
    };
    let calculations: Vec<String> = item
        .calculations
        .iter()
        .map(|c| format!("{}: {} {} ({})", c.label, c.result, c.unit, c.formula))
        .collect();
    let queries: Vec<String> = queries.into_iter().collect();
    let evidence_class = if item.basis == COUNTED {
        "This compares recorded rows with a written limit or ceiling."
    } else {
        "This is a labelled pattern, not a finding that a rule was broken."
    };
    let explanation = format!(
        "{} {} {} Competing explanations: {} Next test: {} Query id(s): {}. Cited row ids: {}.{}",
        item.observation,
        evidence_class,
        if calculations.is_empty() {
            String::new()
        } else {
            format!("Calculation: {}.", calculations.join("; "))
        },
        item.alternatives.join(" "),
        item.next_test,
        queries.join(", "),
        item.citations.join(", "),
        if unresolved > 0 {
            format!(" {unresolved} cited row(s) could not be tied to an ingested document and are listed by id only.")
        } else {
            String::new()
        },
    );
    let now = Utc::now();
    Some(Finding {
        id: finding_id(kind, SLUG, &subject, &[item.id.as_str()]),
        kind,
        severity: if item.basis == COUNTED {
            FindingSeverity::Serious
        } else {
            FindingSeverity::Notable
        },
        confidence: FindingConfidence::measured(confidence),
        subject,
        headline: item.title.clone(),
        explanation,
        citations,
        discovered_at: now,
        last_seen_at: now,
        dismissed_at: None,
        source_detector: SLUG.into(),
    })
}

/// The findings and the gaps of one sweep. A case whose rows cannot all be tied to an origin
/// document adds a gap instead of a finding.
pub async fn evaluate(
    repository: &DuckDbRepository,
) -> Result<(Vec<Finding>, Vec<FinancialGap>), AppError> {
    let swept = sweep(repository).await?;
    let mut sources = BTreeSet::new();
    for evaluated in &swept.evaluated {
        for citation in &evaluated.report.evidence {
            sources.insert((citation.source_sha256.clone(), citation.source_url.clone()));
        }
    }
    let origins = origin_documents(repository, sources).await?;
    let mut findings = Vec::new();
    let mut gaps = swept.gaps.clone();
    for evaluated in &swept.evaluated {
        for case in &evaluated.cases {
            if let Some(finding) = lift(case, &evaluated.report, &swept.rules, &origins, &mut gaps)
            {
                findings.push(finding);
            }
        }
    }
    let mut seen = BTreeSet::new();
    findings.retain(|finding| seen.insert(finding.id.clone()));
    findings.sort_by(|a, b| {
        b.severity
            .rank_weight()
            .cmp(&a.severity.rank_weight())
            .then_with(|| b.confidence.rank_weight().cmp(&a.confidence.rank_weight()))
            .then_with(|| a.id.cmp(&b.id))
    });
    findings.truncate(MAX_FINDINGS);
    let mut seen = BTreeSet::new();
    gaps.retain(|gap| seen.insert(gap.title.clone()));
    Ok((findings, gaps))
}

pub async fn run(repository: &DuckDbRepository) -> Result<Vec<Finding>, AppError> {
    Ok(evaluate(repository).await?.0)
}

/// Routes the ledger could not test, with the table that would allow each check.
pub async fn gaps(repository: &DuckDbRepository) -> Result<Vec<EvidenceGap>, AppError> {
    Ok(evaluate(repository)
        .await?
        .1
        .into_iter()
        .map(|gap| EvidenceGap {
            detector: SLUG,
            key: format!("spending_cases:{}", gap.title),
            route: "Spending rows were compared with contract terms, appropriations, fund restrictions and stated policy limits".into(),
            blocked: gap.why,
            role: spending_cases::gap_role(&gap.title)
                .and_then(GapRole::from_financial_role)
                .unwrap_or(GapRole::Payments),
            request: format!("Provide existing records: {}", gap.records_needed),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{FinancialMapping, PortalRegistration};
    use std::collections::BTreeMap;

    const CONTRACT_SOURCE: &str = "https://seshat.datasd.org/contracts/contracts.csv";
    const PAYMENT_SOURCE: &str = "https://seshat.datasd.org/payments/payments.csv";
    const AMENDMENT_SOURCE: &str = "https://seshat.datasd.org/contracts/amendments.csv";
    const POLICY: &str = "Purchasing Policy and Procedures. This policy governs purchases by City departments. Purchases over $25,000 require council approval. Staff shall follow this policy.";

    struct Fixture {
        _temp: tempfile::TempDir,
        repo: DuckDbRepository,
    }

    #[allow(clippy::too_many_arguments)]
    fn seed_table(
        conn: &duckdb::Connection,
        snapshot: &str,
        table: &str,
        source: &str,
        sha: &str,
        create: &str,
        rows: &str,
        origin: bool,
    ) {
        conn.execute_batch(&format!(
            "CREATE TABLE {table}({create}); INSERT INTO {table} VALUES {rows};"
        ))
        .unwrap();
        conn.execute(
            "INSERT INTO analytical_snapshots(snapshot_id,dataset_kind,display_name,table_name,source_uri,local_path,source_sha256,retrieved_at,validation_summary_json,imported_rows) VALUES (?1,'OPERATING_BUDGET',?1,?2,?3,'/fixture',?4,'2026-09-26','{}',3)",
            params![snapshot, table, source, sha],
        )
        .unwrap();
        let mut statement = conn
            .prepare("SELECT column_name,data_type,ordinal_position FROM information_schema.columns WHERE table_name=?1 ORDER BY ordinal_position")
            .unwrap();
        let columns = statement
            .query_map(params![table], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        for (name, kind, ordinal) in columns {
            conn.execute(
                "INSERT INTO analytical_columns(snapshot_id,table_name,column_name,logical_type,ordinal) VALUES (?1,?2,?3,?4,?5)",
                params![snapshot, table, name, if kind.starts_with("DECIMAL") { "DECIMAL" } else { "TEXT" }, ordinal],
            )
            .unwrap();
        }
        if origin {
            conn.execute(
                "INSERT INTO documents(id,request_id,portal_subdomain,file_name,download_url,visibility,created_at,updated_at,raw_file_sha256) VALUES (?1,1,'sandiego',?2,?3,'public','2026-09-01T00:00:00Z','2026-09-01T00:00:00Z',?4)",
                params![i64::from(sha.as_bytes()[0]), format!("{snapshot}.csv"), source, sha],
            )
            .unwrap();
        }
    }

    async fn fixture(origin: bool, payments: &str, policy_portal: Option<&str>) -> Fixture {
        let temp = tempfile::tempdir().unwrap();
        let repo = DuckDbRepository::new(temp.path().join("core.db")).unwrap();
        repo.run_migrations(include_str!("../../../migrations/0001_initial_schema.sql"))
            .await
            .unwrap();
        let portal: PortalRegistration = serde_json::from_value(serde_json::json!({"portalKey":"sandiego","displayName":"City of San Diego","canonicalUrl":"https://sandiego.nextrequest.com","originUrl":"https://sandiego.nextrequest.com","verifiedAt":"2026-09-13T00:00:00Z","sourceKind":"NEXT_REQUEST","apiLayout":{"requestsEndpoint":"https://sandiego.nextrequest.com/api","requestsPointer":"/resources","pageParameter":"page","pageSizeParameter":"limit"}})).unwrap();
        repo.insert_or_update_portal_registration(&portal)
            .await
            .unwrap();
        {
            let conn = repo.analytics_connection().await.unwrap();
            seed_table(&conn, "budget", "analysis_fin_budget", "https://seshat.datasd.org/operating_budget/budget_operating_datasd.csv", &"b".repeat(64),
                "report_fy INTEGER,fund_number VARCHAR,account_number VARCHAR,funds_center_number VARCHAR,dept_name VARCHAR,amount DECIMAL(28,4),budget_cycle VARCHAR",
                "(23,'100000','5001','001','Police',100,'adopted')", false);
            seed_table(&conn, "actual", "analysis_fin_actual", "https://seshat.datasd.org/operating_actuals/actuals_operating_datasd.csv", &"a".repeat(64),
                "report_fy INTEGER,fund_number VARCHAR,account_number VARCHAR,funds_center_number VARCHAR,dept_name VARCHAR,amount DECIMAL(28,4)",
                "(23,'100000','5001','001','Police',50)", false);
            seed_table(&conn, "contract", "analysis_fin_contract", CONTRACT_SOURCE, &"c".repeat(64),
                "fy INTEGER,fund VARCHAR,contract_id VARCHAR,vendor_id VARCHAR,amount DECIMAL(28,4),status VARCHAR,period_start VARCHAR,period_end VARCHAR",
                "(23,'100000','C1','V1',100,'executed','2023-01-01','2023-12-31')", origin);
            seed_table(&conn, "amendment", "analysis_fin_amendment", AMENDMENT_SOURCE, &"d".repeat(64),
                "fy INTEGER,fund VARCHAR,contract_id VARCHAR,amendment_id VARCHAR,amount DECIMAL(28,4),status VARCHAR,period_end VARCHAR",
                "(23,'100000','C1','A1',0,'executed','2023-12-31')", origin);
            seed_table(&conn, "payment", "analysis_fin_payment", PAYMENT_SOURCE, &"e".repeat(64),
                "fy INTEGER,fund VARCHAR,contract_id VARCHAR,vendor_id VARCHAR,transaction_id VARCHAR,invoice_id VARCHAR,amount DECIMAL(28,4),status VARCHAR,transaction_date VARCHAR,approval VARCHAR",
                payments, origin);
            if let Some(portal_key) = policy_portal {
                conn.execute("INSERT INTO documents(id,request_id,portal_subdomain,file_name,download_url,visibility,created_at,updated_at,extracted_text_sha256) VALUES (9001,1,?1,'purchasing-policy.pdf','https://sandiego.example/policy.pdf','public','2023-01-02T00:00:00Z','2023-01-02T00:00:00Z',?2)", params![portal_key, "f".repeat(64)]).unwrap();
                conn.execute("INSERT INTO documents_fts(document_id,portal_subdomain,file_name,extracted_text) VALUES (9001,?1,'purchasing-policy.pdf',?2)", params![portal_key, POLICY]).unwrap();
            }
        }
        for (role, snapshot, columns) in [
            (
                "contract",
                "contract",
                vec![
                    ("fy", "fy"),
                    ("fund", "fund"),
                    ("contract_id", "contract_id"),
                    ("vendor_id", "vendor_id"),
                    ("amount", "amount"),
                    ("status", "status"),
                    ("period_start", "period_start"),
                    ("period_end", "period_end"),
                ],
            ),
            (
                "amendment",
                "amendment",
                vec![
                    ("fy", "fy"),
                    ("fund", "fund"),
                    ("contract_id", "contract_id"),
                    ("amendment_id", "amendment_id"),
                    ("amount", "amount"),
                    ("status", "status"),
                    ("period_end", "period_end"),
                ],
            ),
            (
                "payment",
                "payment",
                vec![
                    ("fy", "fy"),
                    ("fund", "fund"),
                    ("contract_id", "contract_id"),
                    ("vendor_id", "vendor_id"),
                    ("transaction_id", "transaction_id"),
                    ("invoice_id", "invoice_id"),
                    ("amount", "amount"),
                    ("status", "status"),
                    ("transaction_date", "transaction_date"),
                    ("approval", "approval"),
                ],
            ),
        ] {
            repo.save_financial_mapping(FinancialMapping {
                role: role.into(),
                snapshot_id: snapshot.into(),
                columns: columns
                    .into_iter()
                    .map(|(a, b)| (a.into(), b.into()))
                    .collect::<BTreeMap<_, _>>(),
                currency: Some("USD".into()),
                agency: Some("City of San Diego".into()),
            })
            .await
            .unwrap();
        }
        Fixture { _temp: temp, repo }
    }

    #[tokio::test]
    async fn a_ceiling_case_cites_every_row_with_its_query_and_origin_document_and_is_recordable() {
        let f = fixture(true, "(23,'100000','C1','V1','T1','I1',60,'paid','2023-03-01','yes'),(23,'100000','C1','V1','T2','I2',40.01,'paid','2023-04-01','yes')", None).await;
        let (findings, _) = evaluate(&f.repo).await.unwrap();
        let kinds: Vec<FindingKind> = findings.iter().map(|x| x.kind).collect();
        assert_eq!(
            kinds,
            [FindingKind::ContractCeilingExceeded],
            "{findings:#?}"
        );
        let finding = &findings[0];
        assert_eq!(finding.subject.portal_key.as_deref(), Some("sandiego"));
        assert!(finding.citations.iter().any(|c| c
            .label
            .as_deref()
            .is_some_and(|l| l.starts_with("payment row") && l.contains("query"))));
        assert!(finding.citations.iter().any(|c| c
            .quote
            .as_deref()
            .is_some_and(|q| q.contains("contract_id=C1"))));
        assert!(finding.explanation.contains("Query id(s): "));
        assert!(finding.explanation.contains("0.01"));
        f.repo
            .record_findings(findings)
            .await
            .expect("a legal case with cited rows is recordable");
    }

    #[tokio::test]
    async fn rows_with_no_origin_document_withhold_the_legal_case_and_name_the_gap() {
        let f = fixture(
            false,
            "(23,'100000','C1','V1','T1','I1',100.01,'paid','2023-03-01','yes')",
            None,
        )
        .await;
        let (findings, gaps) = evaluate(&f.repo).await.unwrap();
        assert!(findings.is_empty(), "{findings:#?}");
        assert!(gaps
            .iter()
            .any(|g| g.title == "Origin document for cited ledger rows"));
    }

    #[tokio::test]
    async fn a_clean_ledger_raises_nothing() {
        let f = fixture(
            true,
            "(23,'100000','C1','V1','T1','I1',100,'paid','2023-03-01','yes')",
            None,
        )
        .await;
        let (findings, _) = evaluate(&f.repo).await.unwrap();
        assert!(findings.is_empty(), "{findings:#?}");
    }

    #[tokio::test]
    async fn a_policy_rule_is_cited_with_its_sentence_and_binds_only_within_the_jurisdiction() {
        let payments = "(23,'100000','C1','V1','T1','I1',30000,'paid','2023-03-01','no')";
        let f = fixture(true, payments, Some("sandiego")).await;
        let (findings, _) = evaluate(&f.repo).await.unwrap();
        let case = findings
            .iter()
            .find(|x| x.kind == FindingKind::ApprovalThresholdExceeded)
            .unwrap_or_else(|| panic!("{findings:#?}"));
        assert!(case.citations[0]
            .quote
            .as_deref()
            .is_some_and(|q| q.contains("over $25,000")));
        assert!(case.citations[0].character_start.is_some());
        assert_eq!(
            case.confidence,
            FindingConfidence::measured(spending_cases::UPLOAD_DATE_CONFIDENCE)
        );
        f.repo.record_findings(findings).await.expect("recordable");

        // A policy from a portal with no recognised jurisdiction is not held against the ledger.
        let f = fixture(true, payments, Some("nowhere")).await;
        let (findings, gaps) = evaluate(&f.repo).await.unwrap();
        assert!(findings
            .iter()
            .all(|x| x.kind != FindingKind::ApprovalThresholdExceeded));
        assert!(gaps
            .iter()
            .any(|g| g.title == "Jurisdiction of the policy and the ledger"));
    }
}
