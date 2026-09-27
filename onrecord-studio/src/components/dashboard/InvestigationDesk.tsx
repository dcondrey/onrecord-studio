import { useEffect, useState } from "react";
import { isTauri } from "@tauri-apps/api/core";
import { invokeCommand } from "../../ipc";
import { useAppStore } from "../../store/useAppStore";
import { useLatestRequestGate } from "../../hooks/useLatestRequestGate";
import { cn } from "../../utils/cn";
import {
  INVESTIGATION_STORAGE_KEY,
  INVESTIGATION_TOPICS,
  buildRecordsRequest,
  loadInvestigationWorkspace,
  type InvestigationDraft,
} from "../../utils/investigations";
import {
  SAN_DIEGO_RECORDS_URL,
  SAN_DIEGO_SOURCES,
  openFileImport,
  sanDiegoGuidance,
  sanDiegoSourceForUrl,
  type SanDiegoSource,
} from "../../utils/sanDiego";

const emptyDraft: InvestigationDraft = {
  dateFrom: "",
  dateTo: "",
  notes: "",
  requestText: "",
};

export default function InvestigationDesk() {
  const [workspace, setWorkspace] = useState(() =>
    loadInvestigationWorkspace(window.localStorage),
  );
  const [error, setError] = useState("");
  const [storageError, setStorageError] = useState("");
  const topic = INVESTIGATION_TOPICS.find(
    (item) => item.id === workspace.topicId,
  )!;
  const draft = workspace.drafts[topic.id] ?? emptyDraft;
  const state = useAppStore;

  useEffect(() => {
    try {
      window.localStorage.setItem(
        INVESTIGATION_STORAGE_KEY,
        JSON.stringify({ version: 1, ...workspace }),
      );
      setStorageError("");
    } catch {
      setStorageError(
        "Local saving failed. Download your investigation notes before leaving this view.",
      );
    }
  }, [workspace]);

  const updateDraft = (patch: Partial<InvestigationDraft>) => {
    setWorkspace((current) => ({
      ...current,
      drafts: {
        ...current.drafts,
        [topic.id]: { ...(current.drafts[topic.id] ?? emptyDraft), ...patch },
      },
    }));
    setError("");
  };
  const search = (term: string) => {
    state.getState().setActivePortal(null);
    state.getState().setSearchFilters(term, null, null, null);
    state.getState().setSelectedRequest(null);
    state.getState().setWorkspaceView("evidence");
  };
  const exportNotes = () =>
    download(
      `${topic.id}-investigation.md`,
      [
        `# ${topic.title}`,
        "",
        topic.question,
        "",
        `Record period: ${draft.dateFrom || "Not selected"} through ${draft.dateTo || "Not selected"}`,
        "",
        "## Records needed",
        ...topic.records.map((item) => `- ${item}`),
        "",
        "## Method",
        topic.method,
        topic.limitation,
        "",
        "## Starting sources (collection does not establish a finding)",
        ...topic.sources.map((item) => {
          const source = SAN_DIEGO_SOURCES.find(
            (source) => source.id === item.id,
          )!;
          return `- [${source.label}](${source.url}): ${item.purpose}`;
        }),
        "",
        "## Research notes (unverified unless supported by cited records)",
        draft.notes || "No notes recorded.",
        "",
        "## Request draft (not submitted)",
        draft.requestText || "No draft prepared.",
      ].join("\n"),
    );

  return (
    <section aria-label="San Diego investigations" className="space-y-4">
      <div className="onboarding-panel rounded-2xl border p-5 sm:p-6">
        <div className="app-eyebrow text-cyan-300">Investigate San Diego</div>
        <h3 className="mt-2 text-xl font-semibold text-slate-100">
          Follow the money. Check the services.
        </h3>
        <p className="mt-2 text-sm text-slate-400">
          Choose a question, collect the underlying records, and document what
          the evidence supports.
        </p>
        <div
          className="mt-4 flex flex-wrap gap-2"
          role="group"
          aria-label="Investigation topics"
        >
          {INVESTIGATION_TOPICS.map((item) => (
            <button
              key={item.id}
              type="button"
              aria-pressed={topic.id === item.id}
              className={cn(
                "focus-ring rounded-lg border px-3 py-2 text-sm",
                topic.id === item.id
                  ? "border-cyan-400/40 bg-cyan-500/10 text-cyan-200"
                  : "border-slate-700 text-slate-300",
              )}
              onClick={() => {
                setWorkspace((current) => ({ ...current, topicId: item.id }));
                setError("");
              }}
            >
              {item.title}
            </button>
          ))}
        </div>
      </div>
      <div className="app-panel space-y-5 p-5 sm:p-6">
        <div>
          <h3 className="text-lg font-semibold text-slate-100">
            {topic.question}
          </h3>
          <p className="mt-2 text-sm leading-relaxed text-slate-300">
            {topic.method}
          </p>
          <p className="mt-2 text-xs leading-relaxed text-slate-400">
            {topic.limitation}
          </p>
        </div>
        <div>
          <h4 className="text-sm font-semibold text-slate-200">
            1. Obtain the starting records
          </h4>
          <div className="mt-3 grid gap-3 md:grid-cols-2">
            {topic.sources.map((item) => (
              <InvestigationSource
                key={item.id}
                source={SAN_DIEGO_SOURCES.find(
                  (source) => source.id === item.id,
                )!}
                purpose={item.purpose}
              />
            ))}
          </div>
        </div>
        <div>
          <h4 className="text-sm font-semibold text-slate-200">
            2. Review and compare
          </h4>
          <p className="mt-1 text-xs text-slate-400">
            Searches cover records already in your library. In Analysis, import
            a collected table, then compare department allocations or budgets
            with actuals.
          </p>
          <div className="mt-3 flex flex-wrap gap-2">
            {topic.searches.map((term) => (
              <button
                type="button"
                key={term}
                className="analysis-button"
                onClick={() => search(term)}
              >
                Search “{term}”
              </button>
            ))}
            <button
              type="button"
              className="primary-action focus-ring rounded-lg px-3 py-2 text-xs font-semibold text-white"
              onClick={() => state.getState().setWorkspaceView("analysis")}
            >
              Compare spending
            </button>
            <button
              type="button"
              className="analysis-button"
              onClick={() => state.getState().setWorkspaceView("briefing")}
            >
              Open evidence briefing
            </button>
          </div>
        </div>
        <details className="rounded-xl border border-slate-700 p-4">
          <summary className="cursor-pointer text-sm font-semibold text-slate-200">
            3. Request the missing records
          </summary>
          <p className="mt-3 text-xs text-slate-400">
            Suggested custodians: {topic.custodians}
          </p>
          <ul className="mt-3 list-disc space-y-2 pl-5 text-sm text-slate-300">
            {topic.records.map((record) => (
              <li key={record}>{record}</li>
            ))}
          </ul>
          <div className="mt-4 flex flex-wrap items-end gap-3">
            <label className="text-xs text-slate-300">
              Records from
              <input
                type="date"
                className="analysis-input mt-1 block"
                value={draft.dateFrom}
                onChange={(event) =>
                  updateDraft({ dateFrom: event.target.value })
                }
              />
            </label>
            <label className="text-xs text-slate-300">
              Records through
              <input
                type="date"
                className="analysis-input mt-1 block"
                value={draft.dateTo}
                onChange={(event) =>
                  updateDraft({ dateTo: event.target.value })
                }
              />
            </label>
            <button
              type="button"
              className="analysis-button"
              disabled={Boolean(draft.requestText)}
              onClick={() => {
                try {
                  updateDraft({
                    requestText: buildRecordsRequest(
                      topic,
                      draft.dateFrom,
                      draft.dateTo,
                    ),
                  });
                } catch (reason) {
                  setError(
                    String(reason instanceof Error ? reason.message : reason),
                  );
                }
              }}
            >
              Prepare request draft
            </button>
          </div>
          {error && (
            <p role="alert" className="mt-2 text-sm text-rose-300">
              {error}
            </p>
          )}
          {draft.requestText && (
            <>
              <label className="mt-4 block text-xs text-slate-300">
                Editable request draft
                <textarea
                  className="analysis-input mt-2 min-h-72 w-full"
                  value={draft.requestText}
                  onChange={(event) =>
                    updateDraft({ requestText: event.target.value })
                  }
                />
              </label>
              <p className="mt-2 text-xs text-slate-400">
                Review the dates, custodians, and scope in this text before
                submitting. Date controls do not replace an existing draft.
                Clear the text to generate a new one.
              </p>
              <button
                type="button"
                className="analysis-button mt-3"
                onClick={() =>
                  download(`${topic.id}-records-request.txt`, draft.requestText)
                }
              >
                Download request draft
              </button>
            </>
          )}
          <div className="mt-3 flex flex-wrap items-center gap-3 text-xs">
            <a
              href={SAN_DIEGO_RECORDS_URL}
              target="_blank"
              rel="noreferrer"
              className="focus-ring text-cyan-300"
            >
              Open City request portal
            </a>
            <a
              href="https://sandiegohousingcommission.nextrequest.com"
              target="_blank"
              rel="noreferrer"
              className="focus-ring text-cyan-300"
            >
              Open Housing Commission portal
            </a>
            <button
              type="button"
              className="analysis-button"
              onClick={openFileImport}
            >
              Import response files
            </button>
          </div>
          <p className="mt-2 text-xs text-slate-400">
            Drafts stay on this device. Submit them yourself in the appropriate
            agency portal; no request has been sent.
          </p>
        </details>
        <label className="block text-sm font-semibold text-slate-200">
          Investigation notes
          <textarea
            value={draft.notes}
            className="analysis-input mt-2 min-h-28 w-full font-normal"
            placeholder="Record a question or finding, its source and page, what remains unverified, and the next record you need."
            onChange={(event) => updateDraft({ notes: event.target.value })}
          />
        </label>
        <div className="flex flex-wrap items-center gap-3">
          <button
            type="button"
            className="analysis-button"
            onClick={exportNotes}
          >
            Download investigation notes
          </button>
          <span className="text-xs text-slate-400">
            Notes and request drafts are saved separately for each topic on this
            device.
          </span>
        </div>
        {storageError && (
          <p role="alert" className="text-sm text-rose-300">
            {storageError}
          </p>
        )}
      </div>
    </section>
  );
}

function InvestigationSource({
  source,
  purpose,
}: {
  source: SanDiegoSource;
  purpose: string;
}) {
  const [pending, setPending] = useState(false);
  const [error, setError] = useState("");
  const gate = useLatestRequestGate();
  const portals = useAppStore((state) => state.portals);
  const syncing = useAppStore((state) => state.syncingPortalKeys);
  const initialization = useAppStore(
    (state) => state.workspaceInitializationStatus,
  );
  const saved = portals.find(
    ({ registration }) =>
      sanDiegoSourceForUrl(registration.canonicalUrl)?.id === source.id,
  );
  const collecting =
    pending || Boolean(saved && syncing.includes(saved.registration.portalKey));
  const collect = async () => {
    if (collecting) return;
    const token = gate.begin();
    setPending(true);
    setError("");
    try {
      const result = await invokeCommand("start_data_source", {
        url: source.url,
        crawlPolicy: source.crawlPolicy,
        guidance: sanDiegoGuidance(source),
        rateLimitPolicy: null,
        metadataPageWindow: null,
      });
      // Registration belongs to the returned source even if the operator has changed topics.
      // Do not change the active source or view when background collection starts.
      const store = useAppStore.getState();
      store.upsertPortal(result.source);
      store.setPortalSyncState(result.source.portalKey, true, result.syncRunId);
      store.appendSyncLog(
        `${source.label}: collection started from investigation desk.`,
      );
    } catch (reason) {
      if (gate.isCurrent(token))
        setError(`Collection could not start: ${String(reason)}`);
    } finally {
      if (gate.isCurrent(token)) setPending(false);
    }
  };
  const review = () => {
    if (!saved) return;
    const store = useAppStore.getState();
    store.setActivePortal(saved.registration.portalKey);
    store.setSearchFilters("", null, null, null);
    store.setSelectedRequest(null);
    store.setWorkspaceView("evidence");
  };
  return (
    <article className="rounded-xl border border-slate-700/70 p-4">
      <h5 className="text-sm font-semibold text-slate-200">{source.label}</h5>
      <p className="mt-1 text-xs leading-relaxed text-slate-300">{purpose}</p>
      <p className="mt-2 text-xs leading-relaxed text-slate-400">
        {source.coverage}
      </p>
      <div className="mt-3 flex flex-wrap items-center gap-2">
        {source.access !== "export" && (
          <button
            type="button"
            className="analysis-button"
            disabled={collecting || initialization !== "ready" || !isTauri()}
            onClick={() => void collect()}
          >
            {collecting
              ? "Collecting…"
              : saved
                ? "Update records"
                : "Collect records"}
          </button>
        )}
        <a
          className="focus-ring rounded px-2 py-1 text-xs text-cyan-300"
          href={source.url}
          target="_blank"
          rel="noreferrer"
        >
          Open source
        </a>
        {saved && (
          <button type="button" className="analysis-button" onClick={review}>
            Review saved records
          </button>
        )}
      </div>
      {!isTauri() && source.access !== "export" && (
        <p className="mt-2 text-xs text-slate-400">
          Open the desktop app to collect records.
        </p>
      )}
      {saved?.lastError && (
        <p role="alert" className="mt-2 text-xs text-amber-200">
          Latest source error: {saved.lastError}
        </p>
      )}
      {error && (
        <p role="alert" className="mt-2 text-xs text-rose-300">
          {error} Open the source to download files, then use Import response
          files.
        </p>
      )}
    </article>
  );
}

function download(name: string, content: string) {
  const url = URL.createObjectURL(
    new Blob([content], { type: "text/plain;charset=utf-8" }),
  );
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = name;
  anchor.click();
  window.setTimeout(() => URL.revokeObjectURL(url), 1000);
}
