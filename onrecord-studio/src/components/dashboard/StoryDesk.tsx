import { useCallback, useEffect, useMemo, useState } from 'react';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import { useAppStore } from '../../store/useAppStore';
import { useFirstRunDataStore } from '../../store/useFirstRunDataStore';
import { useAutomaticDiscovery } from './AutomaticDiscoveryProvider';
import type { StoryLead } from '../../types';
import { claimBasisLabel, findingKindLabel, severityLabel } from '../objects/labels';
import { findingToBriefingEvidence } from './BriefingWorkspace';
import FindingsInbox from './FindingsInbox';
import PageHeader from '../ui/PageHeader';
import EmptyState from '../ui/EmptyState';
import ObjectLink from '../objects/ObjectLink';
import { openSourceConnector } from './sourceEvents';
import SinceControl, { useSinceInstant } from './SinceControl';
import CaseScopeBar from './CaseScopeBar';
import { useCaseScope } from '../../hooks/useCaseScope';
import { cn } from '../../utils/cn';
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
  const [notice, setNotice] = useState<{ text: string; failed: boolean } | null>(null);
  const gate = useLatestRequestGate();
  const briefingGate = useLatestRequestGate();
  const findingsRefreshRequests = useAppStore((s) => s.findingsRefreshRequests);
  const setActivePortal = useAppStore((s) => s.setActivePortal);
  const setSearchFilters = useAppStore((s) => s.setSearchFilters);
  const setWorkspaceView = useAppStore((s) => s.setWorkspaceView);
  const addBriefingEvidence = useAppStore((s) => s.addBriefingEvidence);
  const openLeadDossier = useAppStore((s) => s.openLeadDossier);
  const requestFindingsRefresh = useAppStore((s) => s.requestFindingsRefresh);
  const savedSourceCount = useAppStore((s) => s.portals.length);
  const { controller, state: discovery } = useAutomaticDiscovery();
  const fetchingCoreData = useFirstRunDataStore((s) => s.running);
  // A schema that has not loaded, or failed to load, is not an empty library.
  const noTables = discovery?.schema?.datasets.length === 0;
  const [mode, setMode] = useState<'ranked' | 'all'>('ranked');
  const since = useSinceInstant();
  const setSincePreset = useAppStore((s) => s.setSincePreset);
  const caseScope = useCaseScope();
  const visibleLeads = useMemo(
    () => state.status === 'READY'
      ? state.leads.filter((lead) => (!since || lead.firstSeenAt > since) && (caseScope.status !== 'ready' || caseScope.value.findingIds.has(lead.findingId)))
      : [],
    [caseScope, since, state],
  );

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
      const token = briefingGate.begin();
      try {
        const findings = await invokeCommand('list_findings', {
          filter: { id: lead.findingId },
          limit: 1,
        });
        if (!briefingGate.isCurrent(token)) return;
        const finding = findings.find((row) => row.id === lead.findingId);
        const evidence = finding ? findingToBriefingEvidence(finding) : null;
        if (!evidence) {
          setNotice({ text: 'This lead has no request anchor, so it cannot be added to a briefing.', failed: true });
          return;
        }
        addBriefingEvidence(evidence);
        setNotice({ text: `Added to the briefing: ${lead.headline}`, failed: false });
      } catch (reason) {
        if (!briefingGate.isCurrent(token)) return;
        setNotice({ text: formatNativeError(reason), failed: true });
      }
    },
    [addBriefingEvidence, briefingGate],
  );

  return (
    <main className="min-h-0 flex-1 overflow-y-auto">
      <div className="mx-auto w-full max-w-[1200px] space-y-4 p-4 lg:p-6">
        <PageHeader
          title="What the records point to"
          summary="Findings ranked by what the records show, with every claim one click from its source."
          method={<>
            The ranking weighs money at stake, how many documents, requests and agencies repeat the
            pattern, a record contradicting a statement, a counted violation, evidence strength and
            freshness. The score ranks attention; it is not a measure of newsworthiness and not a claim
            about any record. Only a counted violation is a legal claim; everything else is a pattern signal.
          </>}
          actions={<>
            <div role="tablist" aria-label="Story desk view" className="flex rounded-lg border border-control p-0.5">
              {(['ranked', 'all'] as const).map((value) => (
                <button
                  key={value}
                  type="button"
                  role="tab"
                  aria-selected={mode === value}
                  onClick={() => setMode(value)}
                  className={cn('motion-press focus-ring rounded-md px-3 py-1 text-label font-semibold transition', mode === value ? 'bg-accent/10 text-accent' : 'text-muted hover:text-fg')}
                >
                  {value === 'ranked' ? 'Ranked leads' : 'All findings'}
                </button>
              ))}
            </div>
            {mode === 'ranked' && <SinceControl />}
            {mode === 'ranked' && (
              <button type="button" onClick={load} className="motion-press focus-ring rounded-lg border border-control px-3 py-1.5 text-label font-semibold text-fg-secondary hover:border-accent/30">
                Refresh
              </button>
            )}
          </>}
        />
        {notice && <p role={notice.failed ? 'alert' : 'status'} className={cn('motion-reveal text-label', notice.failed ? 'text-warning' : 'text-success')}>{notice.text}</p>}

        <CaseScopeBar state={caseScope} noun={mode === 'all' ? 'findings' : 'leads'} />

        {mode === 'all' ? <FindingsInbox /> : <>
        {state.status === 'PENDING' && <p role="status" className="text-label text-muted">Ranking leads…</p>}
        {state.status === 'FAILED' && (
          <p role="alert" className="rounded-lg border border-danger/30 bg-danger/10 px-3 py-2 text-label text-danger">
            Leads could not be loaded: {state.message}
          </p>
        )}
        {state.status === 'READY' && state.leads.length > 0 && visibleLeads.length < state.leads.length && (
          <p role="status" className="text-label text-muted">
            Showing {visibleLeads.length.toLocaleString()} of {state.leads.length.toLocaleString()} leads{since ? ' first seen in this window' : ''}{caseScope.status === 'ready' ? ' in the active case' : ''}.{' '}
            {since && <button type="button" onClick={() => setSincePreset('all')} className="focus-ring rounded text-accent hover:underline">Any time</button>}
          </p>
        )}
        {state.status === 'READY' && state.leads.length === 0 && (
          savedSourceCount === 0 && noTables ? (
            <EmptyState
              icon="flag"
              title="No leads yet"
              body="Leads come from the records you collect or the tables you import. San Diego’s budget, Get It Done and SDPD tables can be fetched in one step."
              actions={<>
                <button
                  type="button"
                  disabled={fetchingCoreData}
                  onClick={() => {
                    void useFirstRunDataStore.getState().run(controller);
                    setWorkspaceView('overview');
                  }}
                  className="motion-press focus-ring rounded-lg border border-transparent primary-action px-3 py-1.5 text-label font-semibold text-white"
                >
                  Get San Diego’s core public data
                </button>
                <button type="button" onClick={openSourceConnector} className="motion-press focus-ring rounded-lg border border-control px-3 py-1.5 text-label font-semibold text-fg-secondary">Add a source</button>
              </>}
            />
          ) : savedSourceCount === 0 ? (
            <EmptyState
              icon="flag"
              title="No leads yet"
              body="Leads come from findings on the records you collect. Add a records portal, website or file to start."
              actions={<button type="button" onClick={openSourceConnector} className="motion-press focus-ring rounded-lg border border-transparent primary-action px-3 py-1.5 text-label font-semibold text-white">Add a source</button>}
            />
          ) : (
            <EmptyState
              icon="flag"
              title="No leads yet"
              body="Findings appear when the detectors finish a sweep of what has been ingested. Run the sweep again, or check whether a sync is still running."
              actions={<>
                <button type="button" onClick={requestFindingsRefresh} className="motion-press focus-ring rounded-lg border border-transparent primary-action px-3 py-1.5 text-label font-semibold text-white">Refresh findings</button>
                <button type="button" onClick={() => setWorkspaceView('activity')} className="motion-press focus-ring rounded-lg border border-control px-3 py-1.5 text-label font-semibold text-fg-secondary">See sync activity</button>
              </>}
            />
          )
        )}
        {state.status === 'READY' && visibleLeads.length > 0 && (
          <ol className="motion-stagger space-y-3" aria-label="Ranked story leads">
            {visibleLeads.map((lead, index) => (
              <li key={lead.findingId} className="motion-lift rounded-xl border border-line bg-canvas/40 p-4 hover:border-line-strong">
                <div className="flex flex-wrap items-start justify-between gap-3">
                  <div className="min-w-0 flex-1">
                    <div className="text-caption font-semibold uppercase tracking-wider text-accent">
                      #{index + 1} · {findingKindLabel(lead.kind)} · {severityLabel(lead.severity)} · {claimBasisLabel(lead.kind)}
                    </div>
                    <h3 className="mt-1 text-sm font-semibold leading-snug">
                      <ObjectLink target={{ kind: 'finding', id: lead.findingId, label: lead.headline }} className="text-fg decoration-line-strong hover:text-accent">{lead.headline}</ObjectLink>
                    </h3>
                    <AgencyReference lead={lead} />
                  </div>
                  <div className="text-right">
                    <div className="text-caption uppercase tracking-wider text-muted">Attention score</div>
                    <div className="font-mono text-sm text-fg" aria-label="Attention score">
                      {(lead.publicInterest.totalBasisPoints / 100).toFixed(1)}
                    </div>
                  </div>
                </div>
                <ul className="mt-3 flex flex-wrap gap-1.5" aria-label="Ranking factors">
                  {lead.publicInterest.factors.map((factor) => (
                    <li
                      key={factor.name}
                      title={factor.evidence}
                      className="rounded-full border border-line-strong px-2 py-0.5 text-caption text-fg-secondary"
                    >
                      <span className="font-semibold">{factor.name}</span> +{(factor.basisPoints / 100).toFixed(0)}
                      <span className="text-muted"> · {factor.evidence}</span>
                    </li>
                  ))}
                </ul>
                <div className="mt-3 flex flex-wrap items-center gap-2 text-label">
                  <button type="button" onClick={() => inspect(lead)} className="focus-ring rounded-lg border border-control px-2.5 py-1 font-semibold text-fg-secondary hover:border-accent/30">
                    Inspect records
                  </button>
                  <button type="button" onClick={() => void addToBriefing(lead)} className="focus-ring rounded-lg border border-highlight/30 bg-highlight/10 px-2.5 py-1 font-semibold text-highlight">
                    Add to briefing
                  </button>
                  <button
                    type="button"
                    onClick={() => openLeadDossier(lead.findingId)}
                    aria-label={`Assemble dossier: ${lead.headline}`}
                    className="motion-press focus-ring rounded-lg border border-control px-2.5 py-1 font-semibold text-fg-secondary hover:border-accent/30"
                  >
                    Open evidence chain
                  </button>
                  {lead.relatedFindingIds.length > 0 && (
                    <span className="text-muted">
                      {lead.relatedFindingIds.length} related finding{lead.relatedFindingIds.length === 1 ? '' : 's'} on the same subject
                    </span>
                  )}
                </div>
                <LeadDraftToggle findingId={lead.findingId} />
              </li>
            ))}
          </ol>
        )}
        </>}
      </div>
    </main>
  );
}

/** The agency a lead is about, as a link, when the analytics profile names it. */
function AgencyReference({ lead }: { lead: StoryLead }) {
  const { agencyId, portalKey } = lead.subject;
  const profile = useAppStore((state) => agencyId === null ? null : state.agencyProfiles.find((entry) => entry.agencyId === agencyId && entry.portalSubdomain === portalKey) ?? null);
  if (agencyId === null || !portalKey || !profile) return null;
  return (
    <p className="mt-1 text-caption text-muted">
      About <ObjectLink target={{ kind: 'agency', portalKey, agencyId, label: profile.agencyName }}>{profile.agencyName}</ObjectLink>
    </p>
  );
}

/** Mounts the drafts panel only once opened, so loading the desk does not ask which drafts apply to every lead. */
function LeadDraftToggle({ findingId }: { findingId: string }) {
  const [open, setOpen] = useState(false);
  return (
    <details open={open} className="mt-3 text-label">
      <summary
        onClick={(event) => {
          event.preventDefault();
          setOpen((current) => !current);
        }}
        className="focus-ring select-none text-muted hover:text-fg"
      >
        Draft a records request or appeal
      </summary>
      {open && <ArtifactDrafts findingId={findingId} />}
    </details>
  );
}
