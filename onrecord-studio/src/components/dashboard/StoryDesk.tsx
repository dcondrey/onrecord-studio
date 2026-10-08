import { useCallback, useEffect, useMemo, useState } from 'react';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import { useAppStore } from '../../store/useAppStore';
import { useFirstRunDataStore } from '../../store/useFirstRunDataStore';
import { useAutomaticDiscovery } from './AutomaticDiscoveryProvider';
import type { StoryLead } from '../../types';
import { severityLabel } from '../objects/labels';
import { findingFamily } from '../objects/findingFamily';
import FamilyBadge, { ClaimBasisBadge } from '../ui/FamilyBadge';
import SegmentedControl from '../ui/SegmentedControl';
import Button from '../ui/Button';
import InlineError from '../ui/InlineError';
import SeverityGlyph from './SeverityGlyph';
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
  // Keyed by lead, so the outcome shows beside the button that caused it.
  const [notices, setNotices] = useState<Record<string, { text: string; failed: boolean }>>({});
  const [refreshing, setRefreshing] = useState(false);
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
    // A refresh keeps the ranked list on screen, so the page neither collapses nor loses its place.
    setState((current) => (current.status === 'READY' ? current : { status: 'PENDING' }));
    setRefreshing(true);
    invokeCommand('list_story_leads', { limit: 100 })
      .then((leads) => {
        if (!gate.isCurrent(token)) return;
        setState({ status: 'READY', leads });
      })
      .catch((reason) => {
        if (!gate.isCurrent(token)) return;
        setState({ status: 'FAILED', message: formatNativeError(reason) });
      })
      .finally(() => {
        if (gate.isCurrent(token)) setRefreshing(false);
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
          setNotices((current) => ({ ...current, [lead.findingId]: { text: 'This lead has no request anchor, so it can’t be added to a briefing.', failed: true } }));
          return;
        }
        addBriefingEvidence(evidence);
        setNotices((current) => ({ ...current, [lead.findingId]: { text: `Added to the briefing: ${lead.headline}`, failed: false } }));
      } catch (reason) {
        if (!briefingGate.isCurrent(token)) return;
        setNotices((current) => ({ ...current, [lead.findingId]: { text: `Couldn’t add to the briefing: ${formatNativeError(reason)}`, failed: true } }));
      }
    },
    [addBriefingEvidence, briefingGate],
  );

  const hasLeads = state.status === 'READY' && state.leads.length > 0;
  const showRefresh = mode === 'all' || state.status !== 'READY' || state.leads.length > 0;

  return (
    <main className="min-h-0 flex-1 overflow-y-auto">
      <div className="mx-auto w-full max-w-content space-y-6 px-4 pb-10 pt-6 md:px-6">
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
            <SegmentedControl
              label="Story desk view"
              mode="tabs"
              value={mode}
              onChange={setMode}
              panelId={() => 'story-desk-panel'}
              options={[{ value: 'ranked', label: 'Ranked leads' }, { value: 'all', label: 'All findings' }]}
            />
            {mode === 'ranked' && hasLeads && <SinceControl />}
            {showRefresh && (
              <Button
                disabled={mode === 'ranked' && (refreshing || state.status === 'PENDING')}
                onClick={mode === 'ranked' ? load : requestFindingsRefresh}
              >
                {mode === 'ranked' && refreshing ? 'Refreshing…' : 'Refresh'}
              </Button>
            )}
          </>}
        />

        <CaseScopeBar state={caseScope} noun={mode === 'all' ? 'findings' : 'leads'} />

        <div id="story-desk-panel" role="tabpanel" aria-label={mode === 'ranked' ? 'Ranked leads' : 'All findings'} className="space-y-4">
        {mode === 'all' ? <FindingsInbox /> : <>
        {state.status === 'PENDING' && (
          <div role="status" aria-busy="true" className="space-y-3">
            <span className="sr-only">Ranking leads…</span>
            {[0, 1, 2].map((card) => (
              <div key={card} aria-hidden="true" className="flex h-[168px] flex-col gap-3 rounded-xl border border-line bg-panel p-4">
                <div className="flex gap-2"><div className="skeleton h-5 w-24 rounded-full" /><div className="skeleton h-5 w-28 rounded-full" /></div>
                <div className="skeleton h-5 w-3/4 rounded" />
                <div className="skeleton h-3 w-1/3 rounded" />
                <div className="mt-auto flex gap-2"><div className="skeleton h-7 w-28 rounded-md" /><div className="skeleton h-7 w-28 rounded-md" /></div>
              </div>
            ))}
          </div>
        )}
        {state.status === 'FAILED' && (
          <InlineError
            summary="Leads couldn’t be loaded. The local evidence store is busy; try again in a moment."
            detail={state.message}
            onRetry={load}
          />
        )}
        {state.status === 'READY' && state.leads.length > 0 && visibleLeads.length < state.leads.length && (
          <p role="status" className="text-body text-muted">
            Showing <span className="tabular-nums">{visibleLeads.length.toLocaleString()}</span> of <span className="tabular-nums">{state.leads.length.toLocaleString()}</span> leads{since ? ' first seen in this window' : ''}{caseScope.status === 'ready' ? ' in the active case' : ''}.{' '}
            {since && <Button variant="link" onClick={() => setSincePreset('all')}>Any time</Button>}
          </p>
        )}
        {state.status === 'READY' && state.leads.length === 0 && (
          savedSourceCount === 0 && noTables ? (
            <EmptyState
              icon="flag"
              title="No leads yet"
              body="Leads come from the records you collect or the tables you import. San Diego’s budget, Get It Done and SDPD tables can be fetched in one step."
              actions={<>
                <Button
                  variant="primary"
                  disabled={fetchingCoreData}
                  onClick={() => {
                    void useFirstRunDataStore.getState().run(controller);
                    setWorkspaceView('overview');
                  }}
                >
                  Get San Diego’s core public data
                </Button>
                <Button onClick={openSourceConnector}>Add a source</Button>
              </>}
            />
          ) : savedSourceCount === 0 ? (
            <EmptyState
              icon="flag"
              title="No leads yet"
              body="Leads come from findings on the records you collect. Add a records portal, website or file to start."
              actions={<Button variant="primary" onClick={openSourceConnector}>Add a source</Button>}
            />
          ) : (
            <EmptyState
              icon="flag"
              title="No leads yet"
              body="Findings appear when the detectors finish a sweep of what has been ingested. Run the sweep again, or check whether a sync is still running."
              actions={<>
                <Button variant="primary" onClick={requestFindingsRefresh}>Refresh findings</Button>
                <Button onClick={() => setWorkspaceView('activity')}>See sync activity</Button>
              </>}
            />
          )
        )}
        {state.status === 'READY' && visibleLeads.length > 0 && (
          <ol className="motion-stagger space-y-3" aria-label="Ranked story leads" aria-busy={refreshing || undefined}>
            {visibleLeads.map((lead, index) => (
              <li key={lead.findingId}>
                <article aria-labelledby={`lead-${lead.findingId}`} className="flex flex-col gap-3 rounded-xl border border-line bg-panel p-4 shadow-panel">
                  <div className="flex flex-wrap items-start justify-between gap-x-4 gap-y-2">
                    <div className="min-w-0 flex-1">
                      <div className="flex flex-wrap items-center gap-2">
                        <span className="text-caption tabular-nums text-muted">#{index + 1}</span>
                        <FamilyBadge family={findingFamily(lead.kind)} />
                        <ClaimBasisBadge kind={lead.kind} />
                        <span className="inline-flex items-center gap-1 text-caption text-fg-secondary">
                          <SeverityGlyph severity={lead.severity} />
                          {severityLabel(lead.severity)}
                        </span>
                      </div>
                      <h3 id={`lead-${lead.findingId}`} className="mt-2 text-heading">
                        <ObjectLink target={{ kind: 'finding', id: lead.findingId, label: lead.headline }} className="text-fg no-underline hover:underline">{lead.headline}</ObjectLink>
                      </h3>
                      <AgencyReference lead={lead} />
                    </div>
                    <div className="shrink-0 text-right" title="Ranks attention; not a measure of newsworthiness or a legal finding">
                      <p className="text-metric tabular-nums text-fg" aria-label="Attention score">{Math.round(lead.publicInterest.totalBasisPoints / 100)}</p>
                      <p className="text-caption text-muted">Attention score</p>
                    </div>
                  </div>
                  <ul className="flex flex-wrap gap-1.5" aria-label="Ranking factors">
                    {lead.publicInterest.factors.map((factor) => (
                      <li key={factor.name} title={factor.evidence} className="inline-flex min-h-5 max-w-full items-center rounded-full border border-line bg-raised px-2 py-0.5 text-caption text-fg-secondary">
                        <span className="min-w-0 truncate">
                          <span className="font-medium">{factorLabel(factor.name)}</span>{' '}
                          <span className="tabular-nums">+{(factor.basisPoints / 100).toFixed(0)}</span>
                          <span className="text-muted"> · {factor.evidence}</span>
                        </span>
                      </li>
                    ))}
                  </ul>
                  <div className="flex flex-wrap items-center gap-2">
                    <Button size="sm" onClick={() => openLeadDossier(lead.findingId)} aria-label={`Open evidence chain: ${lead.headline}`}>
                      Open evidence chain
                    </Button>
                    <Button size="sm" onClick={() => inspect(lead)}>Inspect records</Button>
                    <Button size="sm" variant="ghost" onClick={() => void addToBriefing(lead)}>Add to briefing</Button>
                    {lead.relatedFindingIds.length > 0 && (
                      <span className="text-caption tabular-nums text-muted">
                        {lead.relatedFindingIds.length} related finding{lead.relatedFindingIds.length === 1 ? '' : 's'} on the same subject
                      </span>
                    )}
                    {notices[lead.findingId] && (
                      <span role={notices[lead.findingId].failed ? 'alert' : 'status'} className={cn('motion-fade text-caption', notices[lead.findingId].failed ? 'text-danger' : 'text-success')}>
                        {notices[lead.findingId].text}
                      </span>
                    )}
                  </div>
                  <LeadDraftToggle findingId={lead.findingId} />
                </article>
              </li>
            ))}
          </ol>
        )}
        </>}
        </div>
      </div>
    </main>
  );
}

// The backend names factors in lowercase prose; the card shows one consistent label per factor.
const FACTOR_LABELS: Record<string, string> = {
  'money at stake': 'Money',
  scale: 'Scale',
  recurrence: 'Recurrence',
  'record contradicts statement': 'Contradiction',
  'counted statutory or policy violation': 'Counted violation',
  'evidence strength': 'Evidence strength',
  freshness: 'Freshness',
};

function factorLabel(name: string) {
  return FACTOR_LABELS[name] ?? name.charAt(0).toUpperCase() + name.slice(1);
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
    <details open={open} className="group border-t border-line pt-3">
      <summary
        onClick={(event) => {
          event.preventDefault();
          setOpen((current) => !current);
        }}
        className="focus-ring inline-flex cursor-pointer list-none select-none items-center gap-1.5 rounded text-body text-fg-secondary hover:text-fg [&::-webkit-details-marker]:hidden"
      >
        Draft a records request or appeal
      </summary>
      {open && <div className="mt-3"><ArtifactDrafts findingId={findingId} /></div>}
    </details>
  );
}
