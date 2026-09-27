export interface InvestigationTopic {
  id: string;
  title: string;
  question: string;
  sources: { id: string; purpose: string }[];
  searches: string[];
  records: string[];
  method: string;
  limitation: string;
  custodians: string;
}

export const INVESTIGATION_TOPICS: readonly InvestigationTopic[] = [
  {
    id: "homelessness",
    title: "Homelessness spending",
    question:
      "What did San Diego spend on homelessness, and what outcomes did those programs report?",
    sources: [
      {
        id: "sd-operating-budget",
        purpose:
          "Establish appropriations by department, fund, and fiscal year.",
      },
      {
        id: "sd-operating-actuals",
        purpose: "Compare allocations with reported spending.",
      },
      {
        id: "iba-fy25-homelessness-revised",
        purpose:
          "Identify programs and funding streams in the revised FY2025 IBA analysis.",
      },
      {
        id: "rtfh-reports-data",
        purpose:
          "Find outcome measures and their definitions; check geography and period.",
      },
    ],
    searches: [
      "homelessness",
      "shelter",
      "housing placement",
      "contract amendment",
    ],
    records: [
      "Program-level adopted and revised budgets, actual expenditures, funding sources, and City-to-SDHC transfers.",
      "Executed provider contracts, amendments, invoices, payments, and performance requirements.",
      "Monthly program reports with capacity, occupied bed nights, exits to permanent housing, follow-up outcomes, and measure definitions. Aggregate reports only; no client identifiers.",
    ],
    method:
      "Reconcile City and SDHC transfers before adding spending. Compare each program’s actual costs with outcomes covering the same period and population.",
    limitation:
      "Shelter capacity, people served, housing exits, and one-night homelessness estimates are different measures. Missing outcome reporting is a gap to investigate.",
    custodians:
      "City Homelessness Strategies and Solutions and Department of Finance. Request SDHC-held contracts and reports separately from the Housing Commission.",
  },
  {
    id: "abatement",
    title: "Encampment abatement",
    question:
      "What do encampment abatements cost, and what happens after each operation?",
    sources: [
      {
        id: "sd-abatement-notices",
        purpose: "Identify the published notice and property process.",
      },
      {
        id: "sd-get-it-done",
        purpose:
          "Examine reported demand and repeat locations; reports are not abatement events.",
      },
      {
        id: "state-audit-homelessness-2023-102-2",
        purpose:
          "Review the State Auditor’s assessment and the City’s response.",
      },
    ],
    searches: [
      "abatement",
      "encampment",
      "impounded property",
      "Environmental Services",
    ],
    records: [
      "Abatement event logs with event ID, date, block or site, notice date, participating departments, and work-order references.",
      "Staff hours, overtime, contractor invoices, equipment costs, and cost allocation methods for encampment operations.",
      "Aggregate outreach contacts, shelter offers and acceptance, placements, and follow-up outcomes by event where recorded; property storage, return, and disposal totals. No personal identifiers.",
    ],
    method:
      "Join costs to documented operations using event or work-order IDs. Examine repeat operations by location and compare costs with recorded housing outcomes.",
    limitation:
      "Complaint closure does not establish a completed abatement or housing placement. A repeated location is not proof that the same people returned.",
    custodians:
      "City Environmental Services, Homelessness Strategies and Solutions, Police, and Department of Finance.",
  },
  {
    id: "bathrooms",
    title: "Public bathrooms",
    question:
      "Where and when are public bathrooms actually available, and what funding decisions create gaps?",
    sources: [
      {
        id: "sd-restroom-availability",
        purpose:
          "Inspect the official beach and bay map as a partial inventory.",
      },
      {
        id: "sd-get-it-done",
        purpose:
          "Find reported restroom problems and their response histories.",
      },
      {
        id: "sd-iba",
        purpose:
          "Find analyses of operating reductions, restorations, and service impacts.",
      },
    ],
    searches: ["restroom", "comfort station", "portable toilet", "handwashing"],
    records: [
      "Public restroom and portable sanitation inventory: facility ID, location, responsible department, accessibility, opening hours, and access restrictions.",
      "Closure and reopening logs with effective dates, reasons, maintenance work orders, and changes to operating hours.",
      "Facility-level operating budgets, actual costs, cleaning contracts, proposed and adopted reductions, and funding restorations.",
    ],
    method:
      "Compare accessible facility hours by neighborhood and date, then connect closures and restorations to adopted funding and operating records.",
    limitation:
      "A point on a map does not establish an open or accessible bathroom. The beach and bay map does not cover downtown or every City facility.",
    custodians:
      "City Parks and Recreation, Homelessness Strategies and Solutions, and Department of Finance. Identify any separately operated facilities before requesting their records.",
  },
  {
    id: "police",
    title: "Police budget priorities",
    question:
      "How much of the same City fund goes to police compared with housing and public services?",
    sources: [
      {
        id: "sd-operating-budget",
        purpose:
          "Compare department allocations within one fund and adopted fiscal year.",
      },
      {
        id: "sd-operating-actuals",
        purpose: "Check actual spending, including overtime accounts.",
      },
      {
        id: "sd-iba",
        purpose: "Read explanations of departmental changes and alternatives.",
      },
    ],
    searches: ["overtime", "Police", "budget reduction", "vacancy"],
    records: [
      "Police adopted and revised budgets and actual expenditures by fiscal year, fund, division, and account, including overtime.",
      "The same budget and expenditure breakdowns for homelessness services, libraries, and parks, with transfers and one-time funding identified.",
      "Staffing and vacancy reports, overtime explanations, budget change proposals, service-level assessments, and adopted Council modifications.",
    ],
    method:
      "Use department allocation shares for a consistent fund, year, cycle, and expense scope. Examine overtime, changes, and service consequences alongside those shares.",
    limitation:
      "A spending share describes priorities; deciding whether police are overallocated also requires service needs, outcomes, and feasible alternatives. Department names may change between years.",
    custodians:
      "City Department of Finance and Police, with service-level records from the departments being compared.",
  },
  {
    id: "waste",
    title: "Contracts and budget waste",
    question:
      "Which expenditures need scrutiny because delivered services do not match the approved cost or contract?",
    sources: [
      {
        id: "sd-auditor-reports",
        purpose:
          "Start with documented findings and unresolved recommendations.",
      },
      {
        id: "sd-operating-actuals",
        purpose: "Locate spending accounts requiring supporting invoices.",
      },
      {
        id: "sd-iba",
        purpose: "Review fiscal assumptions and Council decision context.",
      },
    ],
    searches: [
      "cost overrun",
      "sole source",
      "contract amendment",
      "audit recommendation",
    ],
    records: [
      "Executed contracts and amendments, procurement evaluations, approved scopes, deliverables, and cost ceilings for the selected program or vendor.",
      "Invoices, payment ledger, acceptance records, performance reports, and reconciliations for those contracts.",
      "Relevant audit findings, management responses, corrective-action plans, and implementation evidence.",
    ],
    method:
      "Reconcile approved scope and amendments with payments and accepted deliverables. Document unexplained differences and seek the agency’s explanation.",
    limitation:
      "A large payment, variance, or sole-source award is a lead to examine, not a finding of waste by itself.",
    custodians:
      "City Purchasing and Contracting, Department of Finance, and the department administering the selected contract.",
  },
];

export const INVESTIGATION_STORAGE_KEY = "onrecord:investigations:v1";
export interface InvestigationDraft {
  dateFrom: string;
  dateTo: string;
  notes: string;
  requestText: string;
}
export interface InvestigationWorkspace {
  topicId: string;
  drafts: Record<string, InvestigationDraft>;
}

export function loadInvestigationWorkspace(
  storage: Pick<Storage, "getItem">,
): InvestigationWorkspace {
  const fallback: InvestigationWorkspace = {
    topicId: "homelessness",
    drafts: {},
  };
  try {
    const parsed = JSON.parse(
      storage.getItem(INVESTIGATION_STORAGE_KEY) ?? "null",
    );
    if (!parsed || parsed.version !== 1) return fallback;
    const drafts: Record<string, InvestigationDraft> = {};
    for (const topic of INVESTIGATION_TOPICS) {
      const draft = parsed.drafts?.[topic.id];
      if (
        draft &&
        ["dateFrom", "dateTo", "notes", "requestText"].every(
          (key) => typeof draft[key] === "string",
        )
      )
        drafts[topic.id] = draft;
    }
    return {
      topicId: INVESTIGATION_TOPICS.some((topic) => topic.id === parsed.topicId)
        ? parsed.topicId
        : fallback.topicId,
      drafts,
    };
  } catch {
    return fallback;
  }
}

export function buildRecordsRequest(
  topic: InvestigationTopic,
  dateFrom: string,
  dateTo: string,
): string {
  const validDate = (value: string) =>
    /^\d{4}-\d{2}-\d{2}$/.test(value) &&
    !Number.isNaN(new Date(`${value}T00:00:00Z`).valueOf()) &&
    new Date(`${value}T00:00:00Z`).toISOString().slice(0, 10) === value;
  if (
    !dateFrom ||
    !dateTo ||
    !validDate(dateFrom) ||
    !validDate(dateTo) ||
    dateFrom > dateTo
  )
    throw new Error(
      "Choose a valid start and end date, with the start on or before the end.",
    );
  return `Subject: Public records request — ${topic.title}\n\nTo the City of San Diego public records team:\n\nI request existing records covering ${dateFrom} through ${dateTo}, inclusive, concerning ${topic.title.toLowerCase()} within the City of San Diego.\n\nRecords requested:\n${topic.records.map((record, i) => `${i + 1}. ${record}`).join("\n")}\n\nSuggested custodians: ${topic.custodians}\n\nPlease provide existing electronic records in their native format where available, including CSV or spreadsheets for tabular data, together with field definitions. I am requesting existing records, not a new analysis. Aggregate or de-identified reporting is sufficient; I am not requesting names or personal identifiers of people receiving services.\n\nPlease identify records already publicly available with direct links, let me know if clarification would help locate records, and contact me before incurring any fees.\n\nThank you.`;
}
