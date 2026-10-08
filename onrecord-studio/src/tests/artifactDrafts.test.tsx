// @vitest-environment jsdom
import { afterEach, describe, expect, test, vi } from "vitest";
import {
  act,
  render,
  screen,
  waitFor,
  fireEvent,
  cleanup,
} from "@testing-library/react";
import ArtifactDrafts from "../components/dashboard/ArtifactDrafts";
import StatutoryDeadlineAppeal from "../components/dashboard/StatutoryDeadlineAppeal";
import * as ipc from "../ipc";
import type { DraftAvailability, DraftedArtifact } from "../types";

const followupDraft: DraftedArtifact = {
  id: "artifact-abc",
  kind: "FOLLOWUP_REQUEST",
  subjectFindingId: "finding-xyz",
  jurisdiction: "US-CA",
  templateId: "cpra_followup_generic_v1",
  generatedText:
    "Subject: Follow-up request\n\nDate: 2026-09-29\nTo: [AGENCY NAME]\nRe: Follow-up to [YOUR REQUEST NUMBER]\n\nUnder the California Public Records Act ...",
  disclosure: null,
  citedAuthorities: [{ statute: "Cal. Gov. Code § 7920.000", quote: null }],
  evidenceBasis: "PATTERN_OBSERVATION",
  confidence: { kind: "UNASSESSED" },
  citations: [
    {
      portalKey: "p",
      documentId: 5,
      characterStart: null,
      characterEnd: null,
      quote: null,
      label: null,
    },
  ],
  unknownFacts: ["Agency name", "Your name and contact details"],
  generatedAt: "2026-09-29T12:00:00Z",
};

const appealDraft: DraftedArtifact = {
  ...followupDraft,
  id: "artifact-appeal",
  kind: "APPEAL",
  templateId: "cpra_appeal_policy_commitment_v1",
  generatedText: "Subject: Complaint\n\nBody of the complaint ...",
  evidenceBasis: "COUNTED_POLICY_COMPARISON",
  confidence: { kind: "MEASURED", basisPoints: 8000 },
};

type Handlers = Partial<{
  availability: (args: { findingId: string }) => Promise<DraftAvailability>;
  followup: () => Promise<DraftedArtifact>;
  appeal: () => Promise<DraftedArtifact>;
}>;

function mockIpc(handlers: Handlers) {
  return vi.spyOn(ipc, "invokeCommand").mockImplementation(((
    command: string,
    args: { findingId: string },
  ) => {
    if (command === "draft_availability_for_finding") {
      return (
        handlers.availability ??
        (() => Promise.resolve({ followup: true, appeal: true }))
      )(args);
    }
    if (command === "draft_followup_from_finding") {
      return (handlers.followup ?? (() => Promise.resolve(followupDraft)))();
    }
    if (command === "draft_appeal_from_finding") {
      return (handlers.appeal ?? (() => Promise.resolve(appealDraft)))();
    }
    return Promise.reject(new Error(`unexpected ${command}`));
  }) as unknown as typeof ipc.invokeCommand);
}

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe("<ArtifactDrafts />", () => {
  test("the wrapper is a labelled region so its name is exposed", async () => {
    mockIpc({});
    render(<ArtifactDrafts findingId="finding-xyz" />);
    expect(screen.getByRole("region", { name: "Drafted artifacts" })).toBeTruthy();
    await screen.findByRole("button", { name: /draft records request/i });
  });

  test("a pattern finding is offered a records request and no appeal", async () => {
    mockIpc({
      availability: () => Promise.resolve({ followup: true, appeal: false }),
    });
    render(<ArtifactDrafts findingId="finding-xyz" />);
    expect(
      await screen.findByRole("button", { name: /draft records request/i }),
    ).toBeTruthy();
    expect(screen.queryByRole("button", { name: /appeal/i })).toBeNull();
    expect(screen.queryByRole("textbox")).toBeNull();
  });

  test("a legal-claim finding is offered both, and the appeal routes to the appeal command", async () => {
    const spy = mockIpc({});
    render(<ArtifactDrafts findingId="finding-xyz" />);
    fireEvent.click(
      await screen.findByRole("button", { name: /draft appeal or complaint/i }),
    );
    const textarea = (await screen.findByLabelText(
      /drafted artifact text/i,
    )) as HTMLTextAreaElement;
    expect(textarea.value).toContain("Body of the complaint");
    expect(spy).toHaveBeenCalledWith("draft_appeal_from_finding", {
      findingId: "finding-xyz",
    });
    expect(
      screen.getByText(
        /counted comparison with the agency's own stated commitment/i,
      ),
    ).toBeTruthy();
    expect(screen.getByText("80% measured confidence")).toBeTruthy();
  });

  test("unassessed confidence, unknown facts and missing ranges are shown, never a number", async () => {
    mockIpc({});
    render(<ArtifactDrafts findingId="finding-xyz" />);
    fireEvent.click(
      await screen.findByRole("button", { name: /draft records request/i }),
    );
    await screen.findByLabelText(/drafted artifact text/i);
    expect(screen.getByText("Confidence not assessed")).toBeTruthy();
    expect(screen.getByText("Agency name")).toBeTruthy();
    expect(screen.getByText(/character range not recorded/i)).toBeTruthy();
    expect(screen.getByText(/not sent/i)).toBeTruthy();
  });

  test("does not confirm a copy whose text was edited before the clipboard write settled", async () => {
    mockIpc({});
    let settle: () => void = () => {};
    const writeText = vi.fn(() => new Promise<void>((resolve) => { settle = resolve; }));
    Object.assign(navigator, { clipboard: { writeText } });
    render(<ArtifactDrafts findingId="finding-xyz" />);
    fireEvent.click(
      await screen.findByRole("button", { name: /draft records request/i }),
    );
    const textarea = await screen.findByLabelText(/drafted artifact text/i);
    fireEvent.click(screen.getByRole("button", { name: /^copy$/i }));
    fireEvent.change(textarea, { target: { value: "CHANGED AFTER COPY" } });
    await act(async () => settle());
    expect(screen.getByRole("button", { name: /^copy$/i })).toBeTruthy();
  });

  test("Copy hands over the edited body, and editing cannot remove the disclosure", async () => {
    const sentence =
      "Prepared by a user who has declared an affiliation with City Clerk (self-declared, not verified).";
    mockIpc({
      followup: async () => ({ ...followupDraft, disclosure: sentence }),
    });
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.assign(navigator, { clipboard: { writeText } });
    render(<ArtifactDrafts findingId="finding-xyz" />);
    fireEvent.click(
      await screen.findByRole("button", { name: /draft records request/i }),
    );
    const textarea = (await screen.findByLabelText(
      /drafted artifact text/i,
    )) as HTMLTextAreaElement;
    expect(screen.getByTestId("draft-disclosure").textContent).toContain(
      sentence,
    );
    expect(textarea.value).not.toContain(sentence);
    fireEvent.change(textarea, { target: { value: "EDITED BODY" } });
    fireEvent.click(screen.getByRole("button", { name: /^copy$/i }));
    await waitFor(() =>
      expect(writeText).toHaveBeenCalledWith(
        `EDITED BODY\n\n---\nDisclosure: ${sentence}\n`,
      ),
    );
    fireEvent.change(textarea, { target: { value: "" } });
    fireEvent.click(screen.getByRole("button", { name: /^copy$/i }));
    await waitFor(() =>
      expect(writeText).toHaveBeenLastCalledWith(
        expect.stringContaining(sentence),
      ),
    );
  });

  test("a backend error surfaces in an alert and re-enables the button", async () => {
    mockIpc({
      followup: () => Promise.reject(new Error("No template for this kind")),
    });
    render(<ArtifactDrafts findingId="finding-xyz" />);
    fireEvent.click(
      await screen.findByRole("button", { name: /draft records request/i }),
    );
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("No template for this kind");
    const button = screen.getByRole("button", {
      name: /draft records request/i,
    }) as HTMLButtonElement;
    expect(button.disabled).toBe(false);
  });

  test("a finding with no applicable draft says so", async () => {
    mockIpc({
      availability: () => Promise.resolve({ followup: false, appeal: false }),
    });
    render(<ArtifactDrafts findingId="finding-xyz" />);
    expect(await screen.findByText(/no draft applies/i)).toBeTruthy();
  });
});

describe("staleness guards", () => {
  test("a slow request draft that resolves after the user asks for the appeal is discarded", async () => {
    let resolveFollowup: (value: DraftedArtifact) => void = () => {};
    mockIpc({
      followup: () =>
        new Promise<DraftedArtifact>((resolve) => (resolveFollowup = resolve)),
    });
    render(<ArtifactDrafts findingId="finding-xyz" />);
    fireEvent.click(
      await screen.findByRole("button", { name: /draft records request/i }),
    );
    fireEvent.click(
      screen.getByRole("button", { name: /draft appeal or complaint/i }),
    );
    const textarea = (await screen.findByLabelText(
      /drafted artifact text/i,
    )) as HTMLTextAreaElement;
    resolveFollowup(followupDraft);
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(textarea.value).toContain(appealDraft.generatedText);
    expect(textarea.value).not.toContain(followupDraft.generatedText);
  });

  test("a draft for finding A that resolves after the viewer moved to finding B is never shown", async () => {
    let resolveA: (value: DraftedArtifact) => void = () => {};
    mockIpc({
      followup: () =>
        new Promise<DraftedArtifact>((resolve) => (resolveA = resolve)),
    });
    const { rerender } = render(<ArtifactDrafts findingId="A" />);
    fireEvent.click(
      await screen.findByRole("button", { name: /draft records request/i }),
    );
    rerender(<ArtifactDrafts findingId="B" />);
    await screen.findByRole("button", { name: /draft records request/i });
    resolveA({ ...followupDraft, generatedText: "ABOUT FINDING A" });
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(screen.queryByLabelText(/drafted artifact text/i)).toBeNull();
  });

  test("a shown draft is cleared when the finding changes", async () => {
    mockIpc({});
    const { rerender } = render(<ArtifactDrafts findingId="A" />);
    fireEvent.click(
      await screen.findByRole("button", { name: /draft records request/i }),
    );
    await screen.findByLabelText(/drafted artifact text/i);
    rerender(<ArtifactDrafts findingId="B" />);
    expect(screen.queryByLabelText(/drafted artifact text/i)).toBeNull();
  });

  test("availability fetched for finding A cannot decide the buttons shown for finding B", async () => {
    let resolveA: (value: DraftAvailability) => void = () => {};
    mockIpc({
      availability: ({ findingId }) =>
        findingId === "A"
          ? new Promise<DraftAvailability>((resolve) => (resolveA = resolve))
          : Promise.resolve({ followup: true, appeal: false }),
    });
    const { rerender } = render(<ArtifactDrafts findingId="A" />);
    rerender(<ArtifactDrafts findingId="B" />);
    await screen.findByRole("button", { name: /draft records request/i });
    resolveA({ followup: true, appeal: true });
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(screen.queryByRole("button", { name: /appeal/i })).toBeNull();
  });
});

describe("<StatutoryDeadlineAppeal />", () => {
  test("a draft for request 1 that resolves after the viewer moved to request 2 is never shown", async () => {
    let resolveFirst: (value: DraftedArtifact) => void = () => {};
    vi.spyOn(ipc, "invokeCommand").mockImplementation(
      () =>
        new Promise<DraftedArtifact>(
          (resolve) => (resolveFirst = resolve),
        ) as never,
    );
    const { rerender } = render(
      <StatutoryDeadlineAppeal portalKey="sd" requestId={1} />,
    );
    fireEvent.click(
      screen.getByRole("button", { name: /draft statutory deadline appeal/i }),
    );
    rerender(<StatutoryDeadlineAppeal portalKey="sd" requestId={2} />);
    resolveFirst({ ...appealDraft, generatedText: "ABOUT REQUEST 1" });
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(screen.queryByLabelText(/drafted artifact text/i)).toBeNull();
  });

  test("a backend refusal is shown as the alert text", async () => {
    vi.spyOn(ipc, "invokeCommand").mockRejectedValue(
      new Error("the determination date is not a sourced event"),
    );
    render(<StatutoryDeadlineAppeal portalKey="sd" requestId={1} />);
    fireEvent.click(
      screen.getByRole("button", { name: /draft statutory deadline appeal/i }),
    );
    expect((await screen.findByRole("alert")).textContent).toContain(
      "not a sourced event",
    );
  });
});
