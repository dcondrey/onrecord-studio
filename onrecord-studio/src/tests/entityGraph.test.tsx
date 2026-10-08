// @vitest-environment jsdom
//
// The entity graph publishes a neighbourhood that was requested for one entity. If a slower request
// for an entity the investigator has already clicked away from is allowed to land, the picture on
// screen shows one entity's name at its centre and another entity's relationships around it. That
// is not a rendering glitch: this application presents evidence, and the graph would be asserting
// co-occurrences the corpus never recorded for the entity it is labelled with.
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";

const resolvers = new Map<string, (value: unknown) => void>();
const calls: { command: string; args: Record<string, unknown> }[] = [];
const counts = new Map<string, unknown>();
const clusters = new Map<string, () => Promise<unknown>>();

vi.mock("@tauri-apps/api/core", () => ({
  isTauri: () => false,
  invoke: (command: string, args?: Record<string, unknown>) => {
    calls.push({ command, args: args ?? {} });
    if (command === "get_affiliation_disclosure") {
      return Promise.resolve({
        status: "UNSET",
        sentence: null,
        affiliations: [],
        ownPortalKeys: [],
        ownAgencies: [],
        ownEntityIds: [],
      });
    }
    if (command === "get_profile")
      return Promise.resolve({ status: "UNSET", profile: null });
    if (command === "list_top_corpus_entities") {
      return Promise.resolve([
        node("entity-slow", "Housing Authority", 9),
        node("entity-fast", "rtfhsd.org", 4),
      ]);
    }
    if (command === "page_corpus_entity_neighbors") {
      // Held open so the test decides the resolution order, which is the whole point: the slow
      // request is the first one issued and lands last. A later page is held under its cursor.
      const key = args?.cursor ? `${String(args.entityId)}:${String(args.cursor)}` : String(args?.entityId);
      return new Promise((resolve) => resolvers.set(key, resolve));
    }
    if (command === "count_corpus_entity_neighbors") {
      const entityId = String(args?.entityId);
      return Promise.resolve(
        counts.has(entityId)
          ? counts.get(entityId)
          : { total: 1, unfilteredTotal: 1, byKind: [{ kind: "ORGANIZATION", count: 1 }] },
      );
    }
    if (command === "find_entity_paths") {
      const from = String(args?.fromEntityId);
      return new Promise((resolve) => resolvers.set(`path:${from}`, resolve));
    }
    if (command === "get_resolution_clusters") {
      const held = clusters.get(String(args?.entityId));
      return held ? held() : Promise.resolve(null);
    }
    return Promise.resolve([]);
  },
}));

import EntityGraph from "../components/dashboard/EntityGraph";

function node(entityId: string, displayValue: string, documentCount: number) {
  return {
    entityId,
    kind: "ORGANIZATION" as const,
    displayValue,
    documentCount,
    validationGrade: "GAZETTEER_HIT" as const,
  };
}

function neighborhood(
  centreId: string,
  centreLabel: string,
  neighborLabel: string,
) {
  return {
    center: node(centreId, centreLabel, 9),
    edges: [
      {
        neighbor: node(`${centreId}-neighbor`, neighborLabel, 3),
        basis: "COMPUTED" as const,
        kind: "CO_OCCURRENCE" as const,
        sharedDocumentCount: 3,
        confidenceBasisPoints: 7000,
      },
    ],
    nextCursor: null,
  };
}

afterEach(() => {
  resolvers.clear();
  counts.clear();
  clusters.clear();
  calls.length = 0;
  cleanup();
});

describe("EntityGraph combining", () => {
  it("never merges a lookalike's relationships into an entity the reader has since moved to", async () => {
    render(<EntityGraph />);
    await waitFor(() => expect(resolvers.has("entity-slow")).toBe(true));
    resolvers.get("entity-slow")?.({
      center: node("entity-slow", "Housing Authority", 9),
      edges: [
        {
          neighbor: node("entity-alias", "HOUSING AUTHORITY, Inc.", 2),
          basis: "COMPUTED" as const,
          kind: "CO_OCCURRENCE" as const,
          sharedDocumentCount: 2,
          confidenceBasisPoints: 6000,
        },
      ],
      nextCursor: null,
    });
    const combine = await screen.findByRole("button", { name: /combine in this view/i });
    fireEvent.click(combine);
    await waitFor(() => expect(resolvers.has("entity-alias")).toBe(true));

    // The reader moves on before the alias answers.
    screen.getByRole("button", { name: /rtfhsd\.org/ }).click();
    await waitFor(() => expect(resolvers.has("entity-fast")).toBe(true));
    resolvers.get("entity-fast")?.(neighborhood("entity-fast", "rtfhsd.org", "FAST NEIGHBOUR"));
    await screen.findAllByText("FAST NEIGHBOUR");

    resolvers.get("entity-alias")?.(neighborhood("entity-alias", "HOUSING AUTHORITY, Inc.", "LEAKED VENDOR"));
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(screen.queryByText("LEAKED VENDOR")).toBeNull();
  });
});

function reviewedCluster(centreId: string, memberId: string) {
  const key = (id: string) => ({ kind: "ORGANIZATION", value: id });
  return {
    entityId: centreId,
    key: key(centreId),
    members: [
      { entityId: centreId, key: key(centreId), displayValue: "Housing Authority" },
      { entityId: memberId, key: key(memberId), displayValue: "HOUSING AUTH. (SD)" },
    ],
    reasons: [
      {
        entryId: "entry-1",
        leftDisplayValue: "Housing Authority",
        rightDisplayValue: "HOUSING AUTH. (SD)",
        author: "Maria",
        decidedAt: "2026-03-04T00:00:00Z",
        reason: "same letterhead",
      },
    ],
    contradictions: [],
    resolutionId: "r".repeat(64),
    ledgerHead: "l".repeat(64),
    corpusStateDigest: "s".repeat(64),
  };
}

describe("EntityGraph with a reviewed resolution", () => {
  it("combines a reviewed member's neighbours without a click and says why", async () => {
    clusters.set("entity-slow", () => Promise.resolve(reviewedCluster("entity-slow", "entity-member")));
    render(<EntityGraph />);
    await waitFor(() => expect(resolvers.has("entity-slow")).toBe(true));
    resolvers.get("entity-slow")?.(neighborhood("entity-slow", "Housing Authority", "OWN NEIGHBOUR"));
    await waitFor(() => expect(resolvers.has("entity-member")).toBe(true));
    resolvers.get("entity-member")?.(neighborhood("entity-member", "HOUSING AUTH. (SD)", "MEMBER NEIGHBOUR"));

    await screen.findAllByText("MEMBER NEIGHBOUR");
    expect(screen.getAllByText("OWN NEIGHBOUR").length).toBeGreaterThan(0);
    expect(screen.getByText(/Combined by your review: Housing Authority · HOUSING AUTH\. \(SD\)/)).toBeTruthy();
    expect(screen.getByText(/Maria judged "Housing Authority" and "HOUSING AUTH\. \(SD\)" the same on 2026-03-04: same letterhead/)).toBeTruthy();
  });

  it("never applies a cluster that answers after the reader moved to another centre", async () => {
    let answerSlow: (value: unknown) => void = () => {};
    clusters.set("entity-slow", () => new Promise((resolve) => { answerSlow = resolve; }));
    render(<EntityGraph />);
    await waitFor(() => expect(resolvers.has("entity-slow")).toBe(true));

    screen.getByRole("button", { name: /rtfhsd\.org/ }).click();
    await waitFor(() => expect(resolvers.has("entity-fast")).toBe(true));
    resolvers.get("entity-fast")?.(neighborhood("entity-fast", "rtfhsd.org", "FAST NEIGHBOUR"));
    await screen.findAllByText("FAST NEIGHBOUR");

    answerSlow(reviewedCluster("entity-slow", "entity-member"));
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(resolvers.has("entity-member")).toBe(false);
    expect(screen.queryByText(/Combined by your review/)).toBeNull();
  });
});

describe("EntityGraph", () => {
  it("discards a neighbourhood that resolves after the investigator moved to another entity", async () => {
    render(<EntityGraph />);
    await waitFor(() => expect(resolvers.has("entity-slow")).toBe(true));

    // The investigator clicks through to the second entity while the first is still in flight.
    screen.getByRole("button", { name: /rtfhsd\.org/ }).click();
    await waitFor(() => expect(resolvers.has("entity-fast")).toBe(true));
    resolvers.get("entity-fast")?.(
      neighborhood("entity-fast", "rtfhsd.org", "CURRENT NEIGHBOUR"),
    );
    await screen.findAllByText("CURRENT NEIGHBOUR");

    // Now the superseded request lands. Its result belongs to an entity that is no longer selected.
    resolvers.get("entity-slow")?.(
      neighborhood("entity-slow", "Housing Authority", "SUPERSEDED NEIGHBOUR"),
    );

    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(screen.queryByText("SUPERSEDED NEIGHBOUR")).toBeNull();
    expect(screen.getAllByText("CURRENT NEIGHBOUR").length).toBeGreaterThan(0);
  });

  // A neighbourhood that has not arrived yet and an entity the index no longer holds are different
  // facts. Rendering the first as the second tells the investigator their selection is stale when
  // the index simply has not answered.
  it("does not report a neighbourhood still in flight as a missing entity", async () => {
    render(<EntityGraph />);
    await waitFor(() => expect(resolvers.has("entity-slow")).toBe(true));

    expect(screen.queryByText(/no longer in the index/)).toBeNull();
    expect(screen.getByText(/Reading the neighborhood/)).toBeTruthy();
  });

  it("does not carry one entity's missing verdict onto the next entity while it loads", async () => {
    render(<EntityGraph />);
    await waitFor(() => expect(resolvers.has("entity-slow")).toBe(true));
    resolvers.get("entity-slow")?.(null);
    await screen.findByText(/no longer in the index/);

    screen.getByRole("button", { name: /rtfhsd\.org/ }).click();
    await waitFor(() => expect(resolvers.has("entity-fast")).toBe(true));

    expect(screen.queryByText(/no longer in the index/)).toBeNull();
    expect(screen.getByText(/Reading the neighborhood/)).toBeTruthy();
  });

  // A path is a statement about the pair it was traced for. The origin is the centre entity at
  // submit time, so a result that lands, or is still shown, after the centre has moved is a claim
  // about a pair the investigator never asked about.
  it("discards a path traced from an entity the investigator has since moved away from", async () => {
    render(<EntityGraph />);
    await waitFor(() => expect(resolvers.has("entity-slow")).toBe(true));
    resolvers.get("entity-slow")?.(
      neighborhood("entity-slow", "Housing Authority", "FIRST NEIGHBOUR"),
    );
    await screen.findAllByText("FIRST NEIGHBOUR");

    fireEvent.change(screen.getByLabelText(/Trace a path to/), {
      target: { value: "entity-elsewhere" },
    });
    fireEvent.submit(screen.getByRole("button", { name: "Trace" }));
    await waitFor(() => expect(resolvers.has("path:entity-slow")).toBe(true));

    screen.getByRole("button", { name: /rtfhsd\.org/ }).click();
    await waitFor(() => expect(resolvers.has("entity-fast")).toBe(true));
    resolvers.get("entity-fast")?.(
      neighborhood("entity-fast", "rtfhsd.org", "SECOND NEIGHBOUR"),
    );
    await screen.findAllByText("SECOND NEIGHBOUR");

    resolvers.get("path:entity-slow")?.({ outcome: "notConnected", maxHops: 4 });
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(screen.queryByText(/No path within 4 hops/)).toBeNull();
  });

  it("drops a traced path once the centre entity changes", async () => {
    render(<EntityGraph />);
    await waitFor(() => expect(resolvers.has("entity-slow")).toBe(true));
    resolvers.get("entity-slow")?.(
      neighborhood("entity-slow", "Housing Authority", "FIRST NEIGHBOUR"),
    );
    await screen.findAllByText("FIRST NEIGHBOUR");

    fireEvent.change(screen.getByLabelText(/Trace a path to/), {
      target: { value: "entity-elsewhere" },
    });
    fireEvent.submit(screen.getByRole("button", { name: "Trace" }));
    await waitFor(() => expect(resolvers.has("path:entity-slow")).toBe(true));
    resolvers.get("path:entity-slow")?.({ outcome: "notConnected", maxHops: 4 });
    await screen.findByText(/No path within 4 hops/);

    screen.getByRole("button", { name: /rtfhsd\.org/ }).click();
    await waitFor(() => expect(resolvers.has("entity-fast")).toBe(true));
    expect(screen.queryByText(/No path within 4 hops/)).toBeNull();
  });

  // An id the index does not hold is a different statement from two entities that are not
  // connected; the trace must say which.
  it("reports an unknown target id as unknown, not as not connected", async () => {
    render(<EntityGraph />);
    await waitFor(() => expect(resolvers.has("entity-slow")).toBe(true));
    resolvers.get("entity-slow")?.(
      neighborhood("entity-slow", "Housing Authority", "FIRST NEIGHBOUR"),
    );
    await screen.findAllByText("FIRST NEIGHBOUR");

    fireEvent.change(screen.getByLabelText(/Trace a path to/), {
      target: { value: "entity-missing" },
    });
    fireEvent.submit(screen.getByRole("button", { name: "Trace" }));
    await waitFor(() => expect(resolvers.has("path:entity-slow")).toBe(true));
    resolvers.get("path:entity-slow")?.({ outcome: "unknownEntity", entityId: "entity-missing" });
    await screen.findByText(/entity-missing is not in the entity index/);
    expect(screen.queryByText(/not connected/)).toBeNull();
  });
});

function manyEdges(prefix: string, count: number, kind: "ORGANIZATION" | "PERSON" = "ORGANIZATION") {
  return Array.from({ length: count }, (_, index) => ({
    neighbor: { ...node(`${prefix}-${index}`, `${prefix} ${index}`, 2), kind },
    basis: "COMPUTED" as const,
    kind: "CO_OCCURRENCE" as const,
    sharedDocumentCount: count - index,
    confidenceBasisPoints: 5000,
  }));
}

describe("EntityGraph with more neighbours than a drawing holds", () => {
  it("draws a bounded budget, names the rest by kind, and reaches every neighbour through paging", async () => {
    counts.set("entity-slow", {
      total: 30,
      unfilteredTotal: 30,
      byKind: [{ kind: "ORGANIZATION", count: 25 }, { kind: "PERSON", count: 5 }],
    });
    const { container } = render(<EntityGraph />);
    await waitFor(() => expect(resolvers.has("entity-slow")).toBe(true));
    resolvers.get("entity-slow")?.({
      center: node("entity-slow", "Housing Authority", 9),
      edges: manyEdges("org", 20),
      nextCursor: "page-2",
    });
    await screen.findByText(/20 of 30 neighbors loaded/);

    // The drawing holds its budget of 18 even though 20 are loaded; the rest are named, not dropped.
    expect(container.querySelectorAll("svg [data-edge-family]").length).toBe(18);
    expect(screen.getByRole("button", { name: /7 more of Organization/ })).toBeTruthy();
    expect(screen.getByRole("button", { name: /5 more of Person/ })).toBeTruthy();
    expect(screen.queryByText(/cut at the limit/)).toBeNull();

    // Neighbours beyond the first page are reachable: the window is near the end of the loaded
    // rows, so the next page is requested by cursor without a click.
    await waitFor(() => expect(resolvers.has("entity-slow:page-2")).toBe(true));
    resolvers.get("entity-slow:page-2")?.({
      center: node("entity-slow", "Housing Authority", 9),
      edges: manyEdges("tail", 10),
      nextCursor: null,
    });
    await screen.findByText(/30 of 30 neighbors loaded/);
    expect(screen.getByText("End of the list.")).toBeTruthy();
  });

  it("expands a kind group by filtering the index, not the rows already loaded", async () => {
    counts.set("entity-slow", {
      total: 21,
      unfilteredTotal: 21,
      byKind: [{ kind: "ORGANIZATION", count: 18 }, { kind: "PERSON", count: 3 }],
    });
    render(<EntityGraph />);
    await waitFor(() => expect(resolvers.has("entity-slow")).toBe(true));
    resolvers.get("entity-slow")?.({
      center: node("entity-slow", "Housing Authority", 9),
      edges: manyEdges("org", 18),
      nextCursor: "page-2",
    });
    fireEvent.click(await screen.findByRole("button", { name: /3 more of Person/ }));
    await waitFor(() =>
      expect(
        calls.some(
          (call) =>
            call.command === "page_corpus_entity_neighbors" &&
            (call.args.filter as { kind: string | null } | undefined)?.kind === "PERSON",
        ),
      ).toBe(true),
    );
  });

  // A page for the entity the reader left is a statement about that entity. Landing under the
  // current one it would list another entity's neighbours as this entity's relationships.
  it("drops a later page that lands after the reader moved to another entity", async () => {
    counts.set("entity-slow", { total: 30, unfilteredTotal: 30, byKind: [{ kind: "ORGANIZATION", count: 30 }] });
    render(<EntityGraph />);
    await waitFor(() => expect(resolvers.has("entity-slow")).toBe(true));
    resolvers.get("entity-slow")?.({
      center: node("entity-slow", "Housing Authority", 9),
      edges: manyEdges("slow", 20),
      nextCursor: "page-2",
    });
    await waitFor(() => expect(resolvers.has("entity-slow:page-2")).toBe(true));

    screen.getByRole("button", { name: /rtfhsd\.org/ }).click();
    await waitFor(() => expect(resolvers.has("entity-fast")).toBe(true));
    resolvers.get("entity-fast")?.(neighborhood("entity-fast", "rtfhsd.org", "FAST NEIGHBOUR"));
    await screen.findAllByText("FAST NEIGHBOUR");

    resolvers.get("entity-slow:page-2")?.({
      center: node("entity-slow", "Housing Authority", 9),
      edges: manyEdges("LEAKED", 10),
      nextCursor: null,
    });
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(screen.queryByText(/LEAKED/)).toBeNull();
    expect(screen.getByText(/1 of 1 neighbors loaded/)).toBeTruthy();
  });

  it("says the total could not be counted rather than showing it as zero", async () => {
    counts.set("entity-slow", null);
    render(<EntityGraph />);
    await waitFor(() => expect(resolvers.has("entity-slow")).toBe(true));
    resolvers.get("entity-slow")?.(neighborhood("entity-slow", "Housing Authority", "ONLY NEIGHBOUR"));
    await screen.findAllByText("ONLY NEIGHBOUR");
    await screen.findByText(/the total could not be counted/);
    expect(screen.queryByText(/\b0 neighbours/)).toBeNull();
  });
});
