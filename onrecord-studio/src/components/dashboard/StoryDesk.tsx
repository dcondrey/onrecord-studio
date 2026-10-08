import { useCallback, useMemo, useRef, useState, type ReactNode } from 'react';
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
import MethodNote from '../ui/MethodNote';
import { comparisonHeadline, comparisonOf, COUNTED_FACTOR, EVIDENCE_FACTOR, FACTOR_ORDER, FACTOR_SCALE_MAX_BP, FACTOR_SHORT, factorLabel, factorPoints, isKnownFactor, points } from '../../utils/attentionFactors';
import { useRankedLeads } from '../../hooks/useRankedLeads';
import Masthead from '../workbench/Masthead';
import LeadCard from '../workbench/LeadCard';
import LeadDetail from '../workbench/LeadDetail';
import Workbench, { LeadQueue, Pane, QueueSkeleton, useWorkbenchQueue } from '../workbench/Workbench';
import { PendingDetail } from '../workbench/OverviewWorkbench';
import EmptyState from '../ui/EmptyState';
import ObjectLink from '../objects/ObjectLink';
import { openSourceConnector } from './sourceEvents';
import SinceControl, { useSinceInstant } from './SinceControl';
import CaseScopeBar from './CaseScopeBar';
import { useCaseScope } from '../../hooks/useCaseScope';
import { cn } from '../../utils/cn';
import ArtifactDrafts from './ArtifactDrafts';
import StoredLeadFigure from './StoredLeadFigure';
import BarTable, { type BarTableColumn, type BarTableRow } from '../charts/figures/BarTable';
import { absent, measured } from '../charts/core/nulls';

/**
 * Findings ordered by observable public-interest factors. The score is a ranking heuristic over
 * facts the detectors recorded; every factor shows the fact behind it, and none of it is a claim
 * about the record. Pattern signals stay labelled as such; only a counted violation may say so.
 * The ranking is the one the overview queues, read once through useRankedLeads.
 */
export default function StoryDesk() {
  // Keyed by lead, so the outcome shows beside the button that caused it.
  const [notices, setNotices] = useState<Record<string, { text: string; failed: boolean }>>({});
  const briefingGate = useLatestRequestGate();
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
  const [mode, setMode] = useState<'ranked' | 'table' | 'all'>('ranked');
  const since = useSinceInstant();
  const setSincePreset = useAppStore((s) => s.setSincePreset);
  const caseScope = useCaseScope();
  const ranked = useRankedLeads();
  const visibleLeads = ranked.leads;
  const refreshing = ranked.refreshing;

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

  const hasLeads = ranked.status === 'ready' && ranked.total > 0;
  // While the load has failed, the error's own Try again is the one retry on screen.
  const showRefresh = mode === 'all' || ranked.status === 'pending' || hasLeads;
  const isRanked = mode !== 'all';
  const workbench = mode === 'ranked' && (ranked.status === 'pending' || visibleLeads.length > 0);

  const renderDetail = (lead: StoryLead) => (
    <div className="flex flex-col gap-3">
      <FamilyBadge family={findingFamily(lead.kind)} />
      <StoredLeadFigure findingId={lead.findingId} fallback={null} />
      <LeadDraftButton findingId={lead.findingId} headline={lead.headline} />
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
        <Notice notice={notices[lead.findingId]} />
      </div>
    </div>
  );

  return (
    <main className="min-h-0 flex-1 overflow-y-auto">
      <div className={cn('mx-auto flex w-full flex-col gap-3 px-3 pt-4 md:px-4', workbench ? 'h-full min-h-[36rem] max-w-content pb-3' : 'max-w-content pb-10')}>
        <Masthead
          overline="Story desk"
          title="What the records point to"
          aside={(
            <MethodNote label="How to read this">
              The ranking weighs money at stake, how many documents, requests and agencies repeat the
              pattern, a record contradicting a statement, a counted violation, evidence strength and
              freshness. The score ranks attention; it is not a measure of newsworthiness and not a claim
              about any record. Only a counted violation is a legal claim; everything else is a pattern signal.
            </MethodNote>
          )}
          tabs={(
            <SegmentedControl
              label="Story desk view"
              mode="tabs"
              size="sm"
              value={mode}
              onChange={setMode}
              panelId={() => 'story-desk-panel'}
              options={[{ value: 'ranked', label: 'Ranked leads' }, { value: 'table', label: 'Factor table' }, { value: 'all', label: 'All findings' }]}
            />
          )}
          actions={<>
            {isRanked && (hasLeads || ranked.status === 'pending') && <SinceControl disabled={ranked.status === 'pending'} />}
            {showRefresh && (
              <Button
                className="min-w-[6.5rem]"
                disabled={isRanked && (refreshing || ranked.status === 'pending')}
                onClick={requestFindingsRefresh}
              >
                {isRanked && ranked.status === 'pending' ? 'Loading…' : isRanked && refreshing ? 'Refreshing…' : 'Refresh'}
              </Button>
            )}
          </>}
        />

        <CaseScopeBar state={caseScope} noun={mode === 'all' ? 'findings' : 'leads'} />

        <div
          id="story-desk-panel"
          role="tabpanel"
          aria-label={mode === 'ranked' ? 'Ranked leads' : mode === 'table' ? 'Factor table' : 'All findings'}
          className={cn(workbench ? 'flex min-h-0 flex-1 flex-col' : 'space-y-4')}
        >
        {mode === 'all' ? <FindingsInbox /> : <>
        {mode === 'table' && ranked.status === 'pending' && (
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
        {ranked.status === 'failed' && (
          <InlineError
            summary="Leads couldn’t be loaded. The local evidence store is busy; try again in a moment."
            detail={ranked.message}
            onRetry={requestFindingsRefresh}
          />
        )}
        {isRanked && visibleLeads.length > 0 && visibleLeads.length < ranked.total && (
          <p role="status" className="text-body text-muted">
            Showing <span className="tabular-nums">{visibleLeads.length.toLocaleString()}</span> of <span className="tabular-nums">{ranked.total.toLocaleString()}</span> leads{since ? ' first seen in this window' : ''}{caseScope.status === 'ready' ? ' in the active case' : ''}.{' '}
            {since && <Button variant="link" onClick={() => setSincePreset('all')}>Any time</Button>}
          </p>
        )}
        {ranked.status === 'ready' && ranked.total === 0 && (
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
        {mode === 'ranked' && hasLeads && visibleLeads.length === 0 && (
          <EmptyState
            icon="flag"
            title={since ? 'Nothing new in this window' : 'No lead in the active case'}
            body={`All ${ranked.total.toLocaleString()} leads were first seen ${since ? 'before this window opened' : 'outside the active case'}.`}
            actions={since ? <Button onClick={() => setSincePreset('all')}>Any time</Button> : undefined}
          />
        )}
        {workbench && (
          <StoryWorkbench
            leads={ranked.status === 'ready' ? visibleLeads : null}
            total={ranked.status === 'ready' ? ranked.total : null}
            busy={refreshing}
            onAddToBriefing={(lead) => void addToBriefing(lead)}
            notice={(lead) => <Notice notice={notices[lead.findingId]} />}
          />
        )}
        {mode === 'table' && visibleLeads.length > 0 && (
          <RankedLeads leads={visibleLeads} allLeads={ranked.all} busy={refreshing} renderDetail={renderDetail} />
        )}
        </>}
        </div>
      </div>
    </main>
  );
}

function Notice({ notice }: { notice: { text: string; failed: boolean } | undefined }) {
  if (!notice) return null;
  return (
    <span role={notice.failed ? 'alert' : 'status'} className={cn('motion-fade text-caption', notice.failed ? 'text-danger' : 'text-success')}>
      {notice.text}
    </span>
  );
}

/**
 * The ranked leads as a workbench: the queue in rank order with each lead's score on one shared
 * scale, the selected lead with why it ranks there and its evidence, and the records around it.
 */
function StoryWorkbench({ leads, total, busy, onAddToBriefing, notice }: {
  leads: StoryLead[] | null;
  total: number | null;
  busy: boolean;
  onAddToBriefing(lead: StoryLead): void;
  notice(lead: StoryLead): ReactNode;
}) {
  const selected = useAppStore((s) => s.workbenchSelection.story ?? null);
  const selectWorkbenchLead = useAppStore((s) => s.selectWorkbenchLead);
  const portals = useAppStore((s) => s.portals);
  const queue = useMemo(() => leads?.map((lead, index) => ({ id: lead.findingId, headline: lead.headline, kind: findingFamily(lead.kind), rank: index + 1, lead })) ?? null, [leads]);
  const index = Math.max(0, leads?.findIndex((lead) => lead.findingId === selected) ?? 0);
  const current = leads?.[index] ?? null;
  useWorkbenchQueue('story', queue, current?.findingId ?? null);
  const select = useCallback((id: string) => selectWorkbenchLead('story', id), [selectWorkbenchLead]);
  const scaleMax = leads?.[0]?.publicInterest.totalBasisPoints ?? 1;
  const spansSources = new Set((leads ?? []).map((lead) => lead.portalKey).filter(Boolean)).size > 1;
  const sourceName = (portalKey: string | null) => portals.find((saved) => saved.registration.portalKey === portalKey)?.registration.displayName;
  const items = (leads ?? []).map((lead, at) => ({
    id: lead.findingId,
    content: <LeadCard lead={lead} rank={at + 1} scaleMax={scaleMax} source={spansSources ? sourceName(lead.portalKey) : undefined} />,
  }));
  const selectIfQueued = useCallback((id: string) => {
    if (!leads?.some((lead) => lead.findingId === id)) return false;
    select(id);
    return true;
  }, [leads, select]);

  return (
    <Workbench
      navigator={(
        <div className="workbench-nav">
          <Pane
            label="Ranked lead queue"
            title="Ranked"
            meta={leads && total !== null ? (leads.length < total ? `${leads.length.toLocaleString()} of ${total.toLocaleString()}` : leads.length.toLocaleString()) : undefined}
            actions={<span className="text-caption text-muted">by attention score</span>}
            className="workbench-queue"
            bodyClassName="overflow-y-auto"
          >
            <div aria-busy={busy || undefined}>
              {leads === null ? (
                <QueueSkeleton />
              ) : (
                <LeadQueue label="Leads by attention score" items={items} selectedId={current?.findingId ?? null} onSelect={select} />
              )}
            </div>
          </Pane>
        </div>
      )}
    >
      {current && leads ? (
        <LeadDetail
          findingId={current.findingId}
          storyLead={current}
          position={{ index: index + 1, total: leads.length }}
          compare={comparisonOf(leads, index)}
          source={spansSources ? sourceName(current.portalKey) : undefined}
          onSelectFinding={selectIfQueued}
          actions={<Button size="sm" variant="ghost" onClick={() => onAddToBriefing(current)}>Add to briefing</Button>}
          notice={notice(current)}
        />
      ) : <PendingDetail />}
    </Workbench>
  );
}

const SCORE_KEY = 'score';

/**
 * The ranked leads as a bar-table: the attention score, then the seven factors on one shared
 * basis-point scale, violet as every lead figure is and crimson only for the counted factor. An
 * absent factor's cell is empty and says so to a screen reader; an unassessed evidence strength is
 * the dashed not-recorded slot, never a zero bar. A row expands in place to the lead's actions.
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
    { key: SCORE_KEY, label: 'Score', title: 'Attention score: ranks reading order; not newsworthiness and not a claim about any record', scale: 'own-max', max: scoreMax, paint: 'lead', sortable: true, barWidth: 44 },
    ...factorNames.map((name): BarTableColumn => ({
      key: name,
      label: name === COUNTED_FACTOR ? 'Counted violation (legal claim)' : factorLabel(name),
      short: FACTOR_SHORT[name],
      title: isKnownFactor(name) ? name : `${name} (factor not recognised by this version)`,
      scale: 'shared',
      claim: name === COUNTED_FACTOR ? 'counted' : undefined,
      paint: 'lead',
      sortable: true,
      barWidth: 32,
      missing: { text: '', title: 'not in this finding’s record' },
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
        title="Factor table"
        headline={leads[0] ? comparisonHeadline(leads[0], 1, comparisonOf(leads, 0)) : ''}
        claim="descriptive"
        sources={{ tables: ['findings'], note: 'attention score ranks reading order; it is not newsworthiness and not a claim about any record. Every factor but a counted violation is a pattern signal' }}
        summary={`${leads.length} leads. Factor columns share one scale of 0 to ${points(factorMax)} points. An empty cell means the factor is not in that finding's record; not assessed means the finding's confidence was never measured.`}
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

/**
 * Mounts the drafts panel only once pressed, so loading the desk does not ask which drafts apply to
 * every lead. The panel replaces the button: its own buttons name the drafts that apply.
 */
function LeadDraftButton({ findingId, headline }: { findingId: string; headline: string }) {
  const [open, setOpen] = useState(false);
  const panel = useRef<HTMLDivElement>(null);
  // The pressed button unmounts; focus moves to the first draft the panel offers, or the panel.
  const focusPanel = useCallback(() => (panel.current?.querySelector('button') ?? panel.current)?.focus(), []);
  if (open) return <div ref={panel} tabIndex={-1} className="outline-none"><ArtifactDrafts findingId={findingId} onSettled={focusPanel} /></div>;
  return (
    <div>
      <Button size="sm" icon="filePlus" aria-label={`Draft a request or appeal: ${headline}`} onClick={() => setOpen(true)}>
        Draft a request or appeal
      </Button>
    </div>
  );
}
