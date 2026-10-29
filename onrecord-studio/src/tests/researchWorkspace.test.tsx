// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
} from "@testing-library/react";
import type { ResearchReport, ResearchStatus } from "../types";
const bridge = vi.hoisted(() => ({
  invoke: vi.fn(),
  addBriefingEvidence: vi.fn(),
  setWorkspaceView: vi.fn(),
}));
vi.mock("../ipc", async (importOriginal) => ({ ...(await importOriginal<typeof import("../ipc")>()), invokeCommand: bridge.invoke }));
vi.mock("@tauri-apps/api/core", () => ({ isTauri: () => true }));
vi.mock("../store/useAppStore", () => ({
  useAppStore: {
    getState: () => ({
      setActivePortal: vi.fn(),
      setSearchFilters: vi.fn(),
      setSelectedRequest: vi.fn(),
      setWorkspaceView: bridge.setWorkspaceView,
      addBriefingEvidence: bridge.addBriefingEvidence,
    }),
  },
}));
import ResearchWorkspace from "../components/dashboard/ResearchWorkspace";

const report: ResearchReport = {
  id: "case-1",
  topic: "homelessness",
  createdAt: "2026-09-26T12:00:00Z",
  model: "fixture",
  inputTokens: 10,
  outputTokens: 10,
  investigationLog: ["Compared contract terms with payment records"],
  evidence: [
    {
      id: "e1",
      label: "Shelter contract",
      sourceUrl: "https://www.sandiego.gov/example.pdf",
      sourceSha256: "abc",
      portalKey: null,
      requestId: null,
      documentId: null,
      characterOffset: null,
      sql: null,
      snapshotProvenance: [],
      sourceReferences: [],
      text: "Provider shall report occupied bed nights monthly.",
      limitations: "Fixture evidence",
    },
  ],
  synthesis: {
    title: "Shelter invoice reconciliation",
    summary: "Payments need reconciliation against required monthly reporting.",
    findings: [
      {
        title: "Invoices lack the contract’s reporting counterpart",
        interpretation:
          "The retrieved invoices do not include occupied bed nights.",
        whyItMatters: "Cost per delivered service cannot yet be tested.",
        citations: [
          {
            evidenceId: "e1",
            quote: "Provider shall report occupied bed nights monthly.",
          },
        ],
        counterEvidence: [],
        alternativeExplanations: [
          "Reports may exist in a separate contract file.",
        ],
        nextTest: "Obtain the reports and reconcile the same months.",
      },
    ],
    questions: [],
    requests: [
      {
        title: "Monthly shelter performance reports",
        custodian: "Housing Commission",
        gap: "Occupied bed nights for invoiced months",
        resolves: "Whether billed service volume was delivered",
        draft:
          "Please provide the existing monthly performance reports for the contract period.",
        evidenceIds: ["e1"],
      },
    ],
    meetings: [],
    limitations: ["Limited to retrieved records"],
  },
};
const status: ResearchStatus = {
  policy: {
    enabled: true,
    autoCollect: true,
    provider: "openai",
    model: "gpt-5.4",
    intervalHours: 6,
    dailyCallLimit: 24,
    researchPasses: 4,
    discoverSources: true,
  },
  credentialAvailable: true,
  phase: "Monitoring for new evidence",
  lastError: null,
  lastFinishedAt: null,
  callsToday: 4,
  reports: [report],
  sources: [],
  actionStates: {},
};
const deferred = <T,>() => {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((r) => {
    resolve = r;
  });
  return { promise, resolve };
};
beforeEach(() => {
  bridge.invoke.mockReset();
  bridge.invoke.mockResolvedValue(status);
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

async function openDossier() {
  fireEvent.click(await screen.findByRole("button", { name: "Open full investigation" }));
}

describe("research dossier workflow", () => {
  it("opens with exact evidence, unproven alternatives and a targeted next test instead of full dossier controls", async () => {
    render(<ResearchWorkspace />);
    await screen.findByRole("heading", { name: report.synthesis.findings[0].title });
    expect(screen.getByRole("heading", { name: "What remains unproven" })).toBeTruthy();
    expect(screen.getByText("Reports may exist in a separate contract file.")).toBeTruthy();
    expect(screen.getByText("Obtain the reports and reconcile the same months.")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Review targeted request" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Prepare case packet" })).toBeNull();
    expect(screen.getAllByRole("link", { name: "Open original source" })[0]).toHaveProperty("href", report.evidence[0].sourceUrl);
    fireEvent.click(screen.getByRole("button", { name: "Review targeted request" }));
    expect(screen.getByRole("button", { name: "Recommended records requests" }).getAttribute("aria-pressed")).toBe("true");
    expect(screen.getByText(/Suggested custodian: Housing Commission/)).toBeTruthy();
  });
  it("opens the matching request first while preserving its saved action identity", async () => {
    const unrelated = { ...report.synthesis.requests[0], title: "Unrelated records request", evidenceIds: [] };
    bridge.invoke.mockResolvedValue({ ...status, reports: [{ ...report, synthesis: { ...report.synthesis, requests: [unrelated, report.synthesis.requests[0]] } }] });
    render(<ResearchWorkspace />);
    fireEvent.click(await screen.findByRole("button", { name: "Review targeted request" }));
    expect(screen.getAllByRole("article")[0].querySelector("h4")?.textContent).toBe("Monthly shelter performance reports");
    expect(screen.getByLabelText("Follow-through case-1:request:1")).toBeTruthy();
    expect(screen.getAllByRole("article")[0].querySelector("details[open] pre")?.textContent).toBe(report.synthesis.requests[0].draft);
    expect(screen.queryByRole("heading", { name: "Unrelated records request" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Show the other 1 requests from this dossier" }));
    expect(screen.getByRole("heading", { name: "Unrelated records request" })).toBeTruthy();
  });
  it("hands the selected saved report to publication review without inferring approval", async () => {
    const review = vi.fn();
    render(<ResearchWorkspace onReview={review} />);
    fireEvent.click(await screen.findByRole("button", { name: "Review evidence for publication" }));
    expect(review).toHaveBeenCalledWith("case-1");
  });
  it("distinguishes a paused empty workspace from an investigation that is running", async () => {
    bridge.invoke.mockResolvedValue({ ...status, reports: [], policy: { ...status.policy, enabled: false } });
    render(<ResearchWorkspace />);
    await screen.findByRole("heading", { name: "No saved dossier yet; research is paused" });
    expect(screen.queryByText("Building the first investigation")).toBeNull();
    expect(screen.getByRole("button", { name: "Investigate now" })).toHaveProperty("disabled", false);
    fireEvent.click(screen.getByRole("button", { name: "Investigate now" }));
    expect(screen.getByText("Research settings").closest("details")?.open).toBe(true);
    expect(bridge.invoke).not.toHaveBeenCalledWith("run_research_now", expect.anything());
  });
  it("explains the selected provider credential and opens the relevant settings directly", async () => {
    bridge.invoke.mockResolvedValue({ ...status, reports: [], credentialAvailable: false, policy: { ...status.policy, provider: "openrouter" } });
    render(<ResearchWorkspace />);
    await screen.findByRole("heading", { name: "No saved dossier yet; model access needs setup" });
    expect(screen.getByText("OPENROUTER_API_KEY")).toBeTruthy();
    expect(screen.getByText("~/.env")).toBeTruthy();
    expect(screen.queryByText("OPENAI_API_KEY")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Open Research settings" }));
    expect(screen.getByText("Research settings").closest("details")?.open).toBe(true);
    expect(screen.getByRole("combobox", { name: "Provider" })).toHaveProperty("value", "openrouter");
  });
  it("uses actual saved dossiers rather than failed-attempt timestamps as completion evidence", async () => {
    bridge.invoke.mockResolvedValue({ ...status, lastFinishedAt: "2026-09-27T12:00:00Z", lastError: "Model request failed", reports: [report, { ...report, id: "newer", createdAt: "2026-09-27T11:00:00Z" }] });
    render(<ResearchWorkspace />);
    expect(await screen.findByText(`Latest saved dossier: ${new Date("2026-09-27T11:00:00Z").toLocaleString()}`)).toBeTruthy();
    expect(screen.queryByText(/Last completed run/)).toBeNull();
    expect(screen.queryByText(new Date("2026-09-27T12:00:00Z").toLocaleString(), { exact: false })).toBeNull();
  });
  it("does not call an empty failed attempt a saved result and explains an impossible daily cap", async () => {
    bridge.invoke.mockResolvedValue({ ...status, reports: [], lastFinishedAt: "2026-09-27T12:00:00Z", policy: { ...status.policy, dailyCallLimit: 7 }, callsToday: 0, phase: "Daily model limit cannot cover one investigation; update Research settings" });
    render(<ResearchWorkspace />);
    await screen.findByText(/The daily limit of 7 model calls cannot cover one investigation \(8 calls\)/);
    expect(screen.queryByText(/Latest saved dossier/)).toBeNull();
    expect(screen.getByRole("button", { name: "Investigate now" })).toHaveProperty("disabled", false);
    fireEvent.click(screen.getByRole("button", { name: "Open Research settings" }));
    const limit = screen.getByRole("spinbutton", { name: "Daily model-call limit" });
    expect(limit.getAttribute("min")).toBe("8");
    fireEvent.change(screen.getByRole("combobox", { name: "Provider" }), { target: { value: "ollama" } });
    expect(limit.getAttribute("min")).toBe("7");
  });
  it("explains when the remaining budget cannot start another full investigation", async () => {
    bridge.invoke.mockResolvedValue({ ...status, reports: [], callsToday: 20 });
    render(<ResearchWorkspace />);
    await screen.findByText(/4 model calls remain today; starting a complete investigation requires 8/);
    expect(screen.getByRole("button", { name: "Investigate now" })).toHaveProperty("disabled", false);
  });
  it("does not present a failed catalog load as empty records or active research", async () => {
    bridge.invoke.mockRejectedValue(new Error("Database unavailable"));
    render(<ResearchWorkspace />);
    await screen.findByRole("heading", { name: "The saved investigation catalog could not be read" });
    expect(screen.getByText("Database unavailable")).toBeTruthy();
    expect(screen.queryByText("Building the first investigation")).toBeNull();
    expect(screen.getByRole("button", { name: "Retry research status" })).toBeTruthy();
  });
  it("withholds unmatched findings instead of decorating them with a source link", async () => {
    bridge.invoke.mockResolvedValue({ ...status, reports: [{ ...report, evidence: [] }] });
    render(<ResearchWorkspace />);
    await screen.findByText(/1 saved finding was withheld/);
    expect(screen.queryByRole("heading", { name: report.synthesis.findings[0].title })).toBeNull();
    fireEvent.click(screen.getByText("Research controls and saved dossiers"));
    expect(screen.getByRole("option", { name: /Shelter invoice reconciliation/ })).toBeTruthy();
  });
  it("prepares a reviewable case packet with evidence and current follow-through, and offers both export formats", async () => {
    bridge.invoke.mockResolvedValue({
      ...status,
      actionStates: { "case-1:request:0": "IN_PROGRESS" },
    });
    render(<ResearchWorkspace />);
    await openDossier();
    fireEvent.click(await screen.findByRole("button", { name: "Prepare case packet" }));
    const packet = screen.getByLabelText("Review case packet") as HTMLTextAreaElement;
    expect(packet.value).toContain("[E1 · Shelter contract](#e1)");
    expect(packet.value).toContain(report.synthesis.requests[0].draft);
    expect(packet.value).toContain("Follow-through: In progress");
    expect(packet.value).toContain(report.evidence[0].sourceUrl);
    expect(screen.getByRole("button", { name: "Download Markdown packet" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Download evidence bundle (JSON)" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Select text to copy" }));
    expect(packet.selectionStart).toBe(0);
    expect(packet.selectionEnd).toBe(packet.value.length);
    fireEvent.click(screen.getByRole("button", { name: "Close packet preview" }));
    expect(screen.queryByLabelText("Review case packet")).toBeNull();
  });

  it("renders exact query amounts with headers, missing counterparts, and original source links", async () => {
    const quote = '[2023,"50818708.00",null]';
    const sqlReport: ResearchReport = {
      ...report,
      evidence: [
        {
          ...report.evidence[0],
          sql: "SELECT fiscal_year, actual_overtime, revised_authority FROM analysis_fixture",
          text: JSON.stringify({
            columns: ["fiscal_year", "actual_overtime", "revised_authority"],
            rows: JSON.parse(`[${quote}]`),
          }),
          sourceReferences: [
            {
              url: "https://seshat.datasd.org/operating_actuals/actuals_operating_datasd.csv",
              snapshot: "actuals-2023",
              sha256: "abc",
            },
          ],
        },
      ],
      synthesis: {
        ...report.synthesis,
        findings: [
          {
            ...report.synthesis.findings[0],
            citations: [{ evidenceId: "e1", quote }],
          },
        ],
      },
    };
    bridge.invoke.mockResolvedValue({ ...status, reports: [sqlReport] });
    render(<ResearchWorkspace />);
    expect(
      (await screen.findAllByRole("columnheader", { name: "actual overtime" }))
        .length,
    ).toBeGreaterThan(0);
    expect(screen.getAllByText("50818708.00").length).toBeGreaterThan(0);
    expect(screen.getAllByText("Not established").length).toBeGreaterThan(0);
    expect(screen.queryByText("0.00")).toBeNull();
    expect(
      screen
        .getAllByRole("link")
        .some(
          (link) =>
            link.getAttribute("href") ===
            sqlReport.evidence[0].sourceReferences[0].url,
        ),
    ).toBe(true);
  });

  it("adds cited evidence as a reviewable briefing lead", async () => {
    bridge.addBriefingEvidence.mockClear();
    bridge.setWorkspaceView.mockClear();
    const linked = {
      ...report,
      evidence: [
        {
          ...report.evidence[0],
          portalKey: "city",
          requestId: 12,
          documentId: 34,
          characterOffset: 500,
        },
      ],
    };
    bridge.invoke.mockResolvedValue({ ...status, reports: [linked] });
    render(<ResearchWorkspace />);
    await openDossier();
    fireEvent.click(
      await screen.findByRole("button", {
        name: "Add cited passages to briefing",
      }),
    );
    expect(bridge.addBriefingEvidence).toHaveBeenCalledWith(
      expect.objectContaining({
        portalKey: "city",
        requestId: 12,
        documentId: 34,
        characterOffset: 500,
        readiness: "LEAD",
        sourceUrl: "https://www.sandiego.gov/example.pdf",
        claim: expect.stringContaining("requiring review"),
      }),
    );
    expect(bridge.setWorkspaceView).toHaveBeenCalledWith("briefing");
  });

  it("presents specific findings, exact evidence, and targeted records requests", async () => {
    render(<ResearchWorkspace />);
    await screen.findByText(
      "Invoices lack the contract’s reporting counterpart",
    );
    await openDossier();
    expect(screen.getByText("Other explanations to test")).toBeTruthy();
    expect(
      screen.getAllByText(/Provider shall report occupied bed nights monthly/)
        .length,
    ).toBeGreaterThan(0);
    fireEvent.click(
      screen.getByRole("button", { name: "Recommended records requests" }),
    );
    expect(
      screen.getAllByText("Monthly shelter performance reports")[0],
    ).toBeTruthy();
    expect(
      screen.getByText(/Suggested custodian: Housing Commission/),
    ).toBeTruthy();
    fireEvent.click(screen.getByText("Review request draft"));
    expect(screen.getByText(/No request has been sent/)).toBeTruthy();
    fireEvent.click(
      screen.getByRole("button", { name: "Meetings and follow-through" }),
    );
    expect(
      screen.getByText(/No relevant meeting with a verified date/),
    ).toBeTruthy();
  });
  it("withdraws an open dossier when it leaves the current catalog without substituting another dossier", async () => {
    vi.useFakeTimers();
    render(<ResearchWorkspace />);
    await act(async () => {});
    fireEvent.click(screen.getByRole("button", { name: "Open full investigation" }));
    expect(screen.getByRole("heading", { name: report.synthesis.title })).toBeTruthy();
    const next = { ...report, id: "different", synthesis: { ...report.synthesis, title: "Different source dossier", findings: [] } };
    bridge.invoke.mockResolvedValue({ ...status, reports: [next] });
    await act(async () => { vi.advanceTimersByTime(8000); });
    expect(screen.queryByRole("heading", { name: report.synthesis.title })).toBeNull();
    expect(screen.queryByRole("heading", { name: "Different source dossier" })).toBeNull();
    expect(screen.getByText(/selected dossier is no longer in the current catalog/)).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Prepare case packet" })).toBeNull();
  });
  it("keeps a newer dossier when an older polling response arrives late", async () => {
    vi.useFakeTimers();
    render(<ResearchWorkspace />);
    await act(async () => {});
    fireEvent.click(screen.getByText("Research controls and saved dossiers"));
    const old = deferred<ResearchStatus>();
    bridge.invoke.mockImplementation((name: string) =>
      name === "get_research_status" ? old.promise : Promise.resolve(null),
    );
    await act(async () => {
      vi.advanceTimersByTime(8000);
    });
    const updated = {
      ...status,
      reports: [
        {
          ...report,
          id: "case-2",
          synthesis: {
            ...report.synthesis,
            title: "New evidence changes the case",
          },
        },
      ],
    };
    bridge.invoke.mockImplementation(async (name: string) =>
      name === "get_research_status" ? updated : null,
    );
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Investigate now" }));
    });
    expect(
      screen.getByRole("option", { name: /New evidence changes the case/ }),
    ).toBeTruthy();
    await act(async () => {
      old.resolve(status);
    });
    expect(
      screen.getByRole("option", { name: /New evidence changes the case/ }),
    ).toBeTruthy();
  });
  it("keeps saved settings when an earlier status poll arrives late", async () => {
    vi.useFakeTimers();
    render(<ResearchWorkspace />);
    await act(async () => {});
    fireEvent.click(screen.getByText("Research controls and saved dossiers"));
    const old = deferred<ResearchStatus>();
    bridge.invoke.mockImplementation((name: string) =>
      name === "get_research_status" ? old.promise : Promise.resolve(null),
    );
    await act(async () => {
      vi.advanceTimersByTime(8000);
    });
    bridge.invoke.mockImplementation((name: string) =>
      name === "set_research_policy"
        ? Promise.resolve({
            ...status,
            policy: { ...status.policy, enabled: false },
          })
        : old.promise,
    );
    fireEvent.click(screen.getByText("Research settings"));
    fireEvent.click(screen.getByLabelText("Autonomous research enabled"));
    await act(async () => {
      fireEvent.click(
        screen.getByRole("button", { name: "Save research settings" }),
      );
    });
    await act(async () => {
      old.resolve(status);
    });
    expect(
      screen.getByRole("button", { name: "Investigate now" }),
    ).toHaveProperty("disabled", false);
    expect(screen.getByLabelText("Autonomous research enabled")).toHaveProperty("checked", false);
  });
  it("persists follow-through without presenting a failed update as completed", async () => {
    bridge.invoke.mockImplementation(async (name: string) => {
      if (name === "set_research_action_state")
        throw new Error("Storage unavailable");
      return status;
    });
    render(<ResearchWorkspace />);
    await screen.findByText(
      "Invoices lack the contract’s reporting counterpart",
    );
    await openDossier();
    fireEvent.change(screen.getByLabelText("Follow-through case-1:finding:0"), {
      target: { value: "DONE" },
    });
    await screen.findByText("Storage unavailable");
    expect(
      screen.getByLabelText("Follow-through case-1:finding:0"),
    ).toHaveProperty("value", "OPEN");
  });
});
