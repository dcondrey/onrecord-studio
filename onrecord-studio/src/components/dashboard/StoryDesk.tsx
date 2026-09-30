import { useCallback, useEffect, useState } from 'react';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import { useAppStore } from '../../store/useAppStore';
import type { FindingKind, FindingSeverity, StoryLead } from '../../types';
import { isLegalClaimKind } from '../../types/findings';
import { findingToBriefingEvidence } from './BriefingWorkspace';
import LeadDossier from './LeadDossier';
import ArtifactDrafts from './ArtifactDrafts';

type DeskState =
  | { status: 'PENDING' }
  | { status: 'READY'; leads: StoryLead[] }
  | { status: 'FAILED'; message: string };

/**
 * Findings ordered by observable public-interest factors. The score is a ranking heuristic over
 * facts the detectors recorded; every factor shows the fact behind it, and none of it is a claim
 * about the record. Pattern signals stay labelled as such; only a counted violation may say so.
 */
export default function StoryDesk() {
  const [state, setState] = useState<DeskState>({ status: 'PENDING' });
  const [notice, setNotice] = useState<string | null>(null);
  const gate = useLatestRequestGate();
  const findingsRefreshRequests = useAppStore((s) => s.findingsRefreshRequests);
  const setActivePortal = useAppStore((s) => s.setActivePortal);
  const setSearchFilters = useAppStore((s) => s.setSearchFilters);
  const setWorkspaceView = useAppStore((s) => s.setWorkspaceView);
  const addBriefingEvidence = useAppStore((s) => s.addBriefingEvidence);
  const openLeadDossier = useAppStore((s) => s.openLeadDossier);

  const load = useCallback(() => {
    const token = gate.begin();
    setState({ status: 'PENDING' });
    invokeCommand('list_story_leads', { limit: 100 })
      .then((leads) => {
        if (!gate.isCurrent(token)) return;
        setState({ status: 'READY', leads });
      })
      .catch((reason) => {
        if (!gate.isCurrent(token)) return;
        setState({ status: 'FAILED', message: formatNativeError(reason) });
      });
  }, [gate]);

  useEffect(() => {
    load();
    return gate.invalidate;
  }, [findingsRefreshRequests, gate, load]);

  const inspect = useCallback(
    (lead: StoryLead) => {
      if (lead.portalKey) setActivePortal(lead.portalKey);
      setSearchFilters('', lead.subject.agencyId ?? null, null, null);
      setWorkspaceView('evidence');
    },
    [setActivePortal, setSearchFilters, setWorkspaceView],
  );

  const addToBriefing = useCallback(
    async (lead: StoryLead) => {
      const token = gate.begin();
      try {
        const findings = await invokeCommand('list_findings', {
          filter: { portalKey: lead.portalKey, requestId: lead.subject.requestId },
          limit: 200,
        });
        if (!gate.isCurrent(token)) return;
        const finding = findings.find((row) => row.id === lead.findingId);
        const evidence = finding ? findingToBriefingEvidence(finding) : null;
        if (!evidence) {
          setNotice('This lead has no request anchor, so it cannot be added to a briefing.');
          return;
        }
        addBriefingEvidence(evidence);
        setNotice(`Added to the briefing: ${lead.headline}`);
      } catch (reason) {
        if (!gate.isCurrent(token)) return;
        setNotice(formatNativeError(reason));
      }
    },
    [addBriefingEvidence, gate],
  );

  return (
    <main className="min-h-0 flex-1 overflow-y-auto">
      <div className="mx-auto w-full max-w-[1200px] space-y-4 p-4 lg:p-6">
        <header className="space-y-1">
          <h2 className="text-sm font-semibold text-slate-200">Story desk</h2>
          <p className="text-[11px] leading-relaxed text-slate-400">
            Findings ordered by what the records show: money at stake, how many documents, requests and
            agencies repeat the pattern, a record contradicting a statement, a counted violation, evidence
            strength and freshness. The score ranks attention; it is not a measure of newsworthiness and
            not a claim about any record.
          </p>
          <div className="flex items-center gap-2">
            <button type="button" onClick={load} className="focus-ring rounded-lg border border-slate-700 px-3 py-1.5 text-[10px] font-semibold text-slate-300 hover:border-cyan-400/40">
              Refresh
            </button>
            {notice && <span role="status" className="text-[10px] text-emerald-300">{notice}</span>}
          </div>
        </header>

        {state.status === 'PENDING' && <p role="status" className="text-[11px] text-slate-400">Ranking leads…</p>}
        {state.status === 'FAILED' && (
          <p role="alert" className="rounded-lg border border-rose-400/30 bg-rose-500/10 px-3 py-2 text-[11px] text-rose-200">
            Leads could not be loaded: {state.message}
          </p>
        )}
        {state.status === 'READY' && state.leads.length === 0 && (
          <p className="text-[11px] text-slate-400">No leads yet. Run analysis on an ingested source to produce findings.</p>
        )}
        {state.status === 'READY' && state.leads.length > 0 && (
          <ol className="space-y-3" aria-label="Ranked story leads">
            {state.leads.map((lead, index) => (
              <li key={lead.findingId} className="rounded-xl border border-slate-800 bg-slate-950/40 p-4">
                <div className="flex flex-wrap items-start justify-between gap-3">
                  <div className="min-w-0 flex-1">
                    <div className="text-[9px] font-semibold uppercase tracking-wider text-cyan-300/80">
                      #{index + 1} · {kindLabel(lead.kind)} · {severityLabel(lead.severity)}
                      {isLegalClaimKind(lead.kind) ? ' · counted violation' : ' · pattern signal'}
                    </div>
                    <h3 className="mt-1 text-[12px] font-semibold text-slate-200">{lead.headline}</h3>
                  </div>
                  <div className="text-right">
                    <div className="text-[9px] uppercase tracking-wider text-slate-400">Attention score</div>
                    <div className="font-mono text-sm text-slate-200" aria-label="Attention score">
                      {(lead.publicInterest.totalBasisPoints / 100).toFixed(1)}
                    </div>
                  </div>
                </div>
                <ul className="mt-3 flex flex-wrap gap-1.5" aria-label="Ranking factors">
                  {lead.publicInterest.factors.map((factor) => (
                    <li
                      key={factor.name}
                      title={factor.evidence}
                      className="rounded-full border border-slate-700 px-2 py-0.5 text-[9px] text-slate-300"
                    >
                      <span className="font-semibold">{factor.name}</span> +{(factor.basisPoints / 100).toFixed(0)}
                      <span className="text-slate-400"> · {factor.evidence}</span>
                    </li>
                  ))}
                </ul>
                <div className="mt-3 flex flex-wrap items-center gap-2 text-[10px]">
                  <button type="button" onClick={() => inspect(lead)} className="focus-ring rounded-lg border border-slate-700 px-2.5 py-1 font-semibold text-slate-300 hover:border-cyan-400/40">
                    Inspect records
                  </button>
                  <button type="button" onClick={() => void addToBriefing(lead)} className="focus-ring rounded-lg border border-indigo-400/25 bg-indigo-500/10 px-2.5 py-1 font-semibold text-indigo-200">
                    Add to briefing
                  </button>
                  <button
                    type="button"
                    onClick={() => openLeadDossier(lead.findingId)}
                    aria-label={`Assemble dossier: ${lead.headline}`}
                    className="focus-ring rounded-lg border border-slate-700 px-2.5 py-1 font-semibold text-slate-300 hover:border-cyan-400/40"
                  >
                    Assemble dossier
                  </button>
                  {lead.relatedFindingIds.length > 0 && (
                    <span className="text-slate-400">
                      {lead.relatedFindingIds.length} related finding{lead.relatedFindingIds.length === 1 ? '' : 's'} on the same subject
                    </span>
                  )}
                </div>
                <LeadDraftToggle findingId={lead.findingId} />
              </li>
            ))}
          </ol>
        )}
      </div>
      <LeadDossier />
    </main>
  );
}

/**
 * Keeps the desk compact: the drafts panel mounts only once the reader opens it, so the desk does
 * not ask the backend which drafts apply to every ranked lead on load.
 */
function LeadDraftToggle({ findingId }: { findingId: string }) {
  const [open, setOpen] = useState(false);
  return (
    <details open={open} className="mt-3 text-[10px]">
      <summary
        onClick={(event) => {
          event.preventDefault();
          setOpen((current) => !current);
        }}
        className="focus-ring cursor-pointer select-none text-slate-400 hover:text-slate-200"
      >
        Draft a records request or appeal
      </summary>
      {open && <ArtifactDrafts findingId={findingId} />}
    </details>
  );
}

function kindLabel(kind: FindingKind): string {
  return kind.toLowerCase().replace(/_/g, ' ');
}

function severityLabel(severity: FindingSeverity): string {
  return severity.charAt(0) + severity.slice(1).toLowerCase();
}
