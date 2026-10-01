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

vi.mock("@tauri-apps/api/core", () => ({
  isTauri: () => false,
  invoke: (command: string, args?: Record<string, unknown>) => {
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
    if (command === "get_corpus_entity_neighborhood") {
      const entityId = String(args?.entityId);
      // Held open so the test decides the resolution order, which is the whole point: the slow
      // request is the first one issued and lands last.
      return new Promise((resolve) => resolvers.set(entityId, resolve));
    }
    if (command === "find_entity_paths") {
      const from = String(args?.fromEntityId);
      return new Promise((resolve) => resolvers.set(`path:${from}`, resolve));
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
    truncated: false,
  };
}

afterEach(() => {
  resolvers.clear();
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
      truncated: false,
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
    expect(screen.getByText(/Reading the neighbourhood/)).toBeTruthy();
  });

  it("does not carry one entity's missing verdict onto the next entity while it loads", async () => {
    render(<EntityGraph />);
    await waitFor(() => expect(resolvers.has("entity-slow")).toBe(true));
    resolvers.get("entity-slow")?.(null);
    await screen.findByText(/no longer in the index/);

    screen.getByRole("button", { name: /rtfhsd\.org/ }).click();
    await waitFor(() => expect(resolvers.has("entity-fast")).toBe(true));

    expect(screen.queryByText(/no longer in the index/)).toBeNull();
    expect(screen.getByText(/Reading the neighbourhood/)).toBeTruthy();
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

    fireEvent.change(screen.getByLabelText(/Find path to entity id/), {
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

    fireEvent.change(screen.getByLabelText(/Find path to entity id/), {
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

    fireEvent.change(screen.getByLabelText(/Find path to entity id/), {
      target: { value: "entity-missing" },
    });
    fireEvent.submit(screen.getByRole("button", { name: "Trace" }));
    await waitFor(() => expect(resolvers.has("path:entity-slow")).toBe(true));
    resolvers.get("path:entity-slow")?.({ outcome: "unknownEntity", entityId: "entity-missing" });
    await screen.findByText(/entity-missing is not in the entity index/);
    expect(screen.queryByText(/not connected/)).toBeNull();
  });
});
