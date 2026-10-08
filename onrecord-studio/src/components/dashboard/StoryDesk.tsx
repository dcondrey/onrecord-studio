import { useCallback, useEffect, useMemo, useState, type ReactNode } from 'react';
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
import BarTable, { type BarTableColumn, type BarTableRow } from '../charts/figures/BarTable';
import InsightCard from './InsightCard';
import { absent, measured } from '../charts/core/nulls';

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
  // While the load has failed, the error's own Try again is the one retry on screen.
  const showRefresh = mode === 'all' || state.status === 'PENDING' || hasLeads;

  return (
    <main className="min-h-0 flex-1 overflow-y-auto">
      <div className="mx-auto w-full max-w-6xl space-y-6 px-4 pb-10 pt-6 md:px-6">
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
            {mode === 'ranked' && (hasLeads || state.status === 'PENDING') && <SinceControl disabled={state.status === 'PENDING'} />}
            {showRefresh && (
              <Button
                className="min-w-[7rem]"
                disabled={mode === 'ranked' && (refreshing || state.status === 'PENDING')}
                onClick={mode === 'ranked' ? load : requestFindingsRefresh}
              >
                {mode === 'ranked' && state.status === 'PENDING' ? 'Loading…' : mode === 'ranked' && refreshing ? 'Refreshing…' : 'Refresh'}
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
                <Button onClick={openSourceConnector}>Add source</Button>
              </>}
            />
          ) : savedSourceCount === 0 ? (
            <EmptyState
              icon="flag"
              title="No leads yet"
              body="Leads come from findings on the records you collect. Add a records portal, website or file to start."
              actions={<Button variant="primary" onClick={openSourceConnector}>Add source</Button>}
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
          <TopLeadCards leads={visibleLeads} onOpenChain={openLeadDossier} />
        )}
        {state.status === 'READY' && visibleLeads.length > 0 && (
          <RankedLeads
            leads={visibleLeads}
            allLeads={state.leads}
            busy={refreshing}
            renderDetail={(lead) => (
              <div className="flex flex-col gap-3">
                <FamilyBadge family={findingFamily(lead.kind)} />
                <ul className="grid gap-1 text-caption text-fg-secondary" aria-label="Ranking factors">
                  {lead.publicInterest.factors.map((factor) => (
                    <li key={factor.name}>
                      <span className="font-medium text-fg">{factorLabel(factor.name)}</span>{' '}
                      <span className="tabular-nums">+{(factor.basisPoints / 100).toFixed(0)}</span>
                      <span className="text-muted"> · {factor.evidence}</span>
                    </li>
                  ))}
                </ul>
                <div className="flex flex-wrap items-center gap-2">
                  <Button size="sm" onClick={() => openLeadDossier(lead.findingId)} aria-label={`Open evidence chain: ${lead.headline}`}>
                    Open evidence chain
                  </Button>
                  <Button size="sm" variant="ghost" onClick={() => inspect(lead)}>Inspect records</Button>
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
              </div>
            )}
          />
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

/** The seven factors the ranker can emit (analysis/story_desk.rs), in the figure's fixed order. */
const FACTOR_ORDER = [
  'money at stake',
  'scale',
  'recurrence',
  'record contradicts statement',
  'counted statutory or policy violation',
  'evidence strength',
  'freshness',
] as const;
const isKnownFactor = (name: string) => (FACTOR_ORDER as readonly string[]).includes(name);
/** Header labels for a frame too narrow for the full ones; the full wording stays in the title. */
const FACTOR_SHORT: Record<string, string> = {
  'money at stake': 'Money',
  scale: 'Scale',
  recurrence: 'Recur.',
  'record contradicts statement': 'Contra.',
  'counted statutory or policy violation': 'Counted',
  'evidence strength': 'Evid.',
  freshness: 'Fresh',
};
const COUNTED_FACTOR = 'counted statutory or policy violation';
const EVIDENCE_FACTOR = 'evidence strength';
/** The largest single factor (money at stake), so every factor column shares one 0–2500 bp scale. */
const FACTOR_SCALE_MAX_BP = 2500;
const SCORE_KEY = 'score';

const points = (basisPoints: number) => Math.round(basisPoints / 100);
const factorPoints = (lead: StoryLead, name: string) => lead.publicInterest.factors.find((factor) => factor.name === name)?.basisPoints ?? 0;

/**
 * The deterministic headline: what lifts #1, and on which factor it is ahead of #2. Factor names
 * are the ranker's own lowercase prose.
 */
export function storyHeadline(leads: readonly StoryLead[]): string {
  const [first, second] = leads;
  if (!first) return '';
  const top = [...first.publicInterest.factors].filter((factor) => factor.basisPoints > 0).sort((a, b) => b.basisPoints - a.basisPoints).slice(0, 2);
  const lead = `#1 scores ${points(first.publicInterest.totalBasisPoints)}`;
  const from = top.length === 0 ? '' : `: most from ${top.map((factor) => `${factor.name} (+${points(factor.basisPoints)})`).join(' and ')}`;
  if (!second) return `${lead}${from}.`;
  const delta = points(first.publicInterest.totalBasisPoints) - points(second.publicInterest.totalBasisPoints);
  if (delta === 0) return `${lead}${from}. It ties #2.`;
  const names = [...new Set([...first.publicInterest.factors, ...second.publicInterest.factors].map((factor) => factor.name))];
  const lead1Ahead = (name: string) => factorPoints(first, name) - factorPoints(second, name);
  const gap = names.reduce((best, name) => (lead1Ahead(name) > lead1Ahead(best) ? name : best), names[0]);
  return `${lead}${from}. It is ahead of #2 by ${delta}, mostly on ${gap}.`;
}

/**
 * The ranked leads as a bar-table: the attention score, then the seven factors on one shared
 * basis-point scale. An absent factor is a muted dot; an unassessed evidence strength is the
 * dashed not-recorded slot, never a zero bar. A row expands in place to the lead's actions.
 */
function RankedLeads({ leads, allLeads, busy, renderDetail }: { leads: StoryLead[]; allLeads: readonly StoryLead[]; busy: boolean; renderDetail(lead: StoryLead): ReactNode }) {
  const [sortKey, setSortKey] = useState<string>(SCORE_KEY);
  // The seven factors always have their columns; scales come from every loaded lead, so filtering
  // never rescales one. A factor name outside the ranker's closed set gets its own column after
  // them: it is never dropped.
  const emitted = new Set(allLeads.flatMap((lead) => lead.publicInterest.factors.map((factor) => factor.name)));
  const factorNames = [...FACTOR_ORDER, ...[...emitted].filter((name) => !isKnownFactor(name))];
  const scoreMax = Math.max(1, ...allLeads.map((lead) => lead.publicInterest.totalBasisPoints));
  const factorMax = Math.max(FACTOR_SCALE_MAX_BP, ...allLeads.flatMap((lead) => lead.publicInterest.factors.map((factor) => factor.basisPoints)));
  const rank = new Map(leads.map((lead, index) => [lead.findingId, index + 1]));
  const ordered = sortKey === SCORE_KEY ? leads : [...leads].sort((a, b) => factorPoints(b, sortKey) - factorPoints(a, sortKey) || (rank.get(a.findingId) ?? 0) - (rank.get(b.findingId) ?? 0));
  const byId = new Map(leads.map((lead) => [lead.findingId, lead]));

  const columns: BarTableColumn[] = [
    { key: SCORE_KEY, label: 'Score', title: 'Attention score: ranks reading order; not newsworthiness and not a claim about any record', scale: 'own-max', max: scoreMax, sortable: true, barWidth: 44 },
    ...factorNames.map((name): BarTableColumn => ({
      key: name,
      label: name === COUNTED_FACTOR ? 'Counted violation (legal claim)' : factorLabel(name),
      short: FACTOR_SHORT[name],
      // A name outside the closed set has an unknown claim type: no note asserts one for it.
      note: name === COUNTED_FACTOR || !isKnownFactor(name) ? undefined : 'pattern signal',
      title: isKnownFactor(name) ? name : `${name} (factor not recognised by this version)`,
      scale: 'shared',
      claim: name === COUNTED_FACTOR ? 'counted' : undefined,
      sortable: true,
      barWidth: 32,
      missing: { text: '·', title: 'not in this finding’s record' },
    })),
  ];

  const rows: BarTableRow[] = ordered.map((lead) => {
    const cells: BarTableRow['cells'] = {
      [SCORE_KEY]: {
        value: measured(lead.publicInterest.totalBasisPoints),
        text: String(points(lead.publicInterest.totalBasisPoints)),
        tooltip: { rows: [{ key: 'score', value: String(points(lead.publicInterest.totalBasisPoints)), label: 'attention score' }], note: 'Ranks reading order; not a legal finding.' },
      },
    };
    for (const factor of lead.publicInterest.factors) {
      const unassessed = factor.name === EVIDENCE_FACTOR && lead.confidence.kind === 'UNASSESSED';
      cells[factor.name] = unassessed
        ? { value: absent('not-recorded', 'The finding’s confidence was not assessed'), text: 'not assessed', tooltip: { rows: [], note: 'Evidence strength: not assessed.' } }
        : {
          value: measured(factor.basisPoints),
          text: `+${points(factor.basisPoints)}`,
          tooltip: { rows: [{ key: 'points', value: `+${points(factor.basisPoints)}`, label: factor.name }], note: factor.evidence },
        };
    }
    const present = factorNames.filter((name) => cells[name]).map((name) => `${name} ${cells[name]?.text}`);
    return {
      key: lead.findingId,
      name: lead.headline,
      label: `#${rank.get(lead.findingId)}, ${lead.headline}, score ${points(lead.publicInterest.totalBasisPoints)}: ${present.join(', ')}`,
      cells,
      header: (
        <div className="grid min-w-[14rem] max-w-[22rem] gap-1">
          <div className="flex flex-wrap items-center gap-2">
            <span className="text-caption tabular-nums text-muted">#{rank.get(lead.findingId)}</span>
            <span className="inline-flex items-center gap-1 text-caption text-fg-secondary">
              <SeverityGlyph severity={lead.severity} />
              {severityLabel(lead.severity)}
            </span>
            <ClaimBasisBadge kind={lead.kind} />
          </div>
          <span id={`lead-${lead.findingId}`} className="line-clamp-2 text-body font-medium" title={lead.headline}>
            <ObjectLink target={{ kind: 'finding', id: lead.findingId, label: lead.headline }} className="text-fg no-underline hover:underline">{lead.headline}</ObjectLink>
          </span>
          <AgencyReference lead={lead} />
        </div>
      ),
    };
  });

  return (
    <div aria-busy={busy || undefined} className="rounded-xl border border-line bg-panel p-4 shadow-panel">
      <BarTable
        title="Ranked leads"
        headline={storyHeadline(leads)}
        claim="descriptive"
        sources={{ tables: ['findings'], note: 'Attention score ranks reading order; it is not newsworthiness and not a claim about any record' }}
        summary={`${leads.length} leads. Factor columns share one scale of 0 to ${points(factorMax)} points. A dot means the factor did not apply; not assessed means the finding's confidence was never measured.`}
        status="ready"
        layout="stacked"
        rowHeader="Lead"
        columns={columns}
        rows={rows}
        sharedMax={factorMax}
        sort={{ key: sortKey, direction: 'descending' }}
        onSort={setSortKey}
        drill={{ mode: 'row', render: (key) => { const lead = byId.get(key); return lead ? renderDetail(lead) : null; } }}
        twin={{
          caption: 'Ranked leads with each factor’s points',
          columns: [
            { key: 'rank', label: 'Rank', align: 'end' },
            { key: 'headline', label: 'Lead' },
            { key: SCORE_KEY, label: 'Score', align: 'end' },
            ...factorNames.map((name) => ({ key: name, label: name, align: 'end' as const })),
          ],
          rows: ordered.map((lead) => ({
            rank: rank.get(lead.findingId) ?? null,
            headline: lead.headline,
            [SCORE_KEY]: points(lead.publicInterest.totalBasisPoints),
            ...Object.fromEntries(factorNames.map((name) => {
              const factor = lead.publicInterest.factors.find((candidate) => candidate.name === name);
              if (!factor) return [name, 'not in this finding’s record'];
              if (name === EVIDENCE_FACTOR && lead.confidence.kind === 'UNASSESSED') return [name, absent('not-recorded', 'confidence not assessed')];
              return [name, `+${points(factor.basisPoints)} · ${factor.evidence}`];
            })),
          })),
        }}
        controls={sortKey !== SCORE_KEY && (
          <p role="status" className="flex flex-wrap items-center gap-2 text-caption text-fg-secondary">
            Sorted by {sortKey}.
            <Button size="sm" variant="ghost" onClick={() => setSortKey(SCORE_KEY)}>Sort by attention score</Button>
          </p>
        )}
      />
    </div>
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

const TOP_CARDS = 3;

/**
 * The top of the desk as lead cards: rank, family and claim basis, the headline, and the attention
 * score drawn as its factors (one violet segment per factor that contributed, on a scale shared by
 * the three cards). The ranked figure below carries every lead and factor; these are its first rows,
 * stated as cards. An unassessed evidence strength contributes no segment and says so.
 */
function TopLeadCards({ leads, onOpenChain }: { leads: readonly StoryLead[]; onOpenChain(findingId: string): void }) {
  const top = leads.slice(0, TOP_CARDS);
  const scoreMax = Math.max(1, ...top.map((lead) => lead.publicInterest.totalBasisPoints));
  return (
    <section aria-labelledby="top-leads-heading" className="grid gap-3">
      <h3 id="top-leads-heading" className="text-overline uppercase text-muted">Top of the desk</h3>
      <ol className="grid grid-cols-1 gap-4 md:grid-cols-2 xl:grid-cols-3">
        {top.map((lead, index) => (
          <li key={lead.findingId} className={cn('min-w-0', index === 2 && 'md:hidden xl:block')}>
            <InsightCard
              badges={<>
                <span className="text-caption font-medium tabular-nums text-muted">#{index + 1}</span>
                <FamilyBadge family={findingFamily(lead.kind)} />
                <ClaimBasisBadge kind={lead.kind} />
              </>}
              headline={lead.headline}
              open={{ target: { kind: 'finding', id: lead.findingId, label: lead.headline } }}
              body={<ScoreFactors lead={lead} scoreMax={scoreMax} />}
              severity={lead.severity}
              time={lead.firstSeenAt}
              actions={(
                <Button size="sm" onClick={() => onOpenChain(lead.findingId)} aria-label={`Open evidence chain for lead #${index + 1}`}>
                  Open evidence chain
                </Button>
              )}
            />
          </li>
        ))}
      </ol>
    </section>
  );
}

function ScoreFactors({ lead, scoreMax }: { lead: StoryLead; scoreMax: number }) {
  const unassessed = lead.confidence.kind === 'UNASSESSED';
  const order = (name: string) => {
    const at = (FACTOR_ORDER as readonly string[]).indexOf(name);
    return at < 0 ? FACTOR_ORDER.length : at;
  };
  const factors = lead.publicInterest.factors
    .filter((factor) => factor.basisPoints > 0 && !(factor.name === EVIDENCE_FACTOR && unassessed))
    .sort((a, b) => order(a.name) - order(b.name));
  const leading = [...factors].sort((a, b) => b.basisPoints - a.basisPoints).slice(0, 2);
  const total = points(lead.publicInterest.totalBasisPoints);
  return (
    <div className="mt-1 grid gap-1.5">
      <div className="flex items-baseline justify-between gap-3 text-caption">
        <span className="text-muted">Attention score</span>
        <span className="font-medium tabular-nums text-fg">{total}</span>
      </div>
      <div
        role="img"
        aria-label={`Attention score ${total}: ${factors.map((factor) => `${factor.name} +${points(factor.basisPoints)}`).join(', ') || 'no contributing factor'}`}
        className="flex h-2 gap-0.5"
        style={{ width: `${(lead.publicInterest.totalBasisPoints / scoreMax) * 100}%` }}
      >
        {factors.map((factor, index) => (
          <span
            key={factor.name}
            className="h-full min-w-[3px] first:rounded-l-full last:rounded-r-full"
            style={{ flexGrow: factor.basisPoints, flexBasis: 0, background: 'var(--viz-lead)', opacity: index % 2 === 0 ? 1 : 0.55 }}
          />
        ))}
      </div>
      <p className="truncate text-caption tabular-nums text-fg-secondary">
        {leading.map((factor) => `${factorLabel(factor.name)} +${points(factor.basisPoints)}`).join(' · ')}
        {unassessed && <span className="text-muted">{leading.length ? ' · ' : ''}evidence not assessed</span>}
      </p>
    </div>
  );
}
