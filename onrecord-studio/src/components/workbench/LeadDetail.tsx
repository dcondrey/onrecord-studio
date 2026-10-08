import { useMemo, useState, type ReactNode } from 'react';
import { useAppStore } from '../../store/useAppStore';
import type { EvidenceLink, LeadDossier as LeadDossierData, StoryLead } from '../../types';
import { isLegalClaimKind } from '../../types/findings';
import { tableLeadSources, type TableLeadSources } from '../../utils/tableLeadSources';
import AttentionFactors from '../charts/figures/AttentionFactors';
import LeadFigure from '../charts/figures/LeadFigure';
import { useDiscoveryController } from '../dashboard/AutomaticDiscoveryProvider';
import ArtifactDrafts from '../dashboard/ArtifactDrafts';
import { findingToBriefingEvidence } from '../dashboard/BriefingWorkspace';
import { dossierMarkdown, GapRequest, gapLine, useLeadDossier, type FigureState } from '../dashboard/LeadDossier';
import SeverityGlyph from '../dashboard/SeverityGlyph';
import { downloadText } from '../dashboard/textDownload';
import EvidenceChain from '../objects/EvidenceChain';
import { findingFamily } from '../objects/findingFamily';
import { formatShortDate, severityLabel } from '../objects/labels';
import ObjectLink from '../objects/ObjectLink';
import Badge from '../ui/Badge';
import Button from '../ui/Button';
import FamilyBadge, { ClaimBasisBadge } from '../ui/FamilyBadge';
import Icon from '../ui/icons';
import InlineError from '../ui/InlineError';
import { Pane, PaneSection } from './Workbench';

const TABLES_READ = '\n\nTables read:\n';

/** The explanation without the tables-read and SQL trailer a table lead stores after its prose. */
function prose(explanation: string): string {
  const at = explanation.lastIndexOf(TABLES_READ);
  return (at < 0 ? explanation : explanation.slice(0, at)).trim();
}

export interface LeadPosition {
  index: number;
  total: number;
}

/**
 * The stage and context panes for one finding, from one gated dossier read: the stage states the
 * lead and lays out its figure and evidence chain; the context lists the records, requests and
 * findings around it. Both panes describe the finding the read was made for, or neither renders.
 */
export default function LeadDetail({ findingId, storyLead, rank, position, onSelectFinding, actions, notice }: {
  findingId: string;
  /** The ranker's record of this finding, when it has one: drawn as why it ranks here. */
  storyLead?: StoryLead | null;
  rank?: number;
  position?: LeadPosition;
  /** Selects a related finding in this workbench; false when it is not in the queue. */
  onSelectFinding?(id: string): boolean;
  /** Replaces the default Add to briefing action (Story desk reports its outcome beside it). */
  actions?: ReactNode;
  notice?: ReactNode;
}) {
  const state = useLeadDossier(findingId);
  const ready = state.status === 'ready' ? state : null;
  return (
    <>
      <Pane label="Selected lead" title="Lead" meta={position ? `${position.index} of ${position.total}` : undefined} actions={<StageActions position={position} dossier={ready?.dossier ?? null} />} elevated>
        {state.status === 'failed' ? (
          <div className="p-6"><InlineError summary="This lead’s dossier couldn’t be assembled." detail={state.message} /></div>
        ) : !ready ? (
          <StageSkeleton />
        ) : (
          <Stage key={ready.dossier.finding.id} dossier={ready.dossier} figure={ready.figure} storyLead={storyLead ?? null} rank={rank} actions={actions} notice={notice} />
        )}
      </Pane>
      <Pane label="Records around this lead" title="Around this lead">
        {state.status === 'failed' ? (
          <p className="px-4 py-3 text-body text-muted">Nothing to list: the dossier didn’t load.</p>
        ) : !ready ? (
          <ContextSkeleton />
        ) : (
          <Context key={ready.dossier.finding.id} dossier={ready.dossier} onSelectFinding={onSelectFinding} />
        )}
      </Pane>
    </>
  );
}

function Stage({ dossier, figure, storyLead, rank, actions, notice }: {
  dossier: LeadDossierData;
  figure: FigureState;
  storyLead: StoryLead | null;
  rank?: number;
  actions?: ReactNode;
  notice?: ReactNode;
}) {
  const { finding } = dossier;
  const openLeadDossier = useAppStore((state) => state.openLeadDossier);
  const addBriefingEvidence = useAppStore((state) => state.addBriefingEvidence);
  const setActivePortal = useAppStore((state) => state.setActivePortal);
  const setSearchFilters = useAppStore((state) => state.setSearchFilters);
  const setWorkspaceView = useAppStore((state) => state.setWorkspaceView);
  const controller = useDiscoveryController();
  const sources = useMemo(() => tableLeadSources(finding.explanation), [finding.explanation]);
  const drawn = 'figure' in figure ? figure.figure : null;
  const text = prose(finding.explanation);
  const counted = isLegalClaimKind(finding.kind);
  const briefing = findingToBriefingEvidence(finding);
  // The evidence chain restates the claim; the stage already leads with it.
  const chain = useMemo(() => ({ ...dossier, finding: { ...finding, explanation: '' } }), [dossier, finding]);

  const inspectRecords = () => {
    if (finding.subject.portalKey) setActivePortal(finding.subject.portalKey);
    setSearchFilters('', finding.subject.agencyId ?? null, null, null);
    setWorkspaceView('evidence');
  };
  const sql = sources?.sql;
  const openQuery = controller && sql ? () => { controller.inspect(sql); setWorkspaceView('analysis'); } : undefined;

  return (
    <article className="motion-fade" aria-labelledby="stage-headline">
      <header className="border-b border-line px-5 pb-5 pt-4">
        <div className="flex flex-wrap items-center gap-2">
          <FamilyBadge family={findingFamily(finding.kind)} />
          <ClaimBasisBadge kind={finding.kind} />
          <span className="inline-flex items-center gap-1 text-caption text-fg-secondary">
            <SeverityGlyph severity={finding.severity} />
            {severityLabel(finding.severity)}
          </span>
        </div>
        <h2 id="stage-headline" className="mt-2.5 max-w-[52rem] text-balance text-title text-fg">{finding.headline}</h2>
        <p className="mt-1.5 flex flex-wrap items-center gap-x-1.5 text-caption text-muted">
          <span>First seen <time dateTime={finding.discoveredAt} className="tabular-nums">{formatShortDate(finding.discoveredAt)}</time></span>
          <span aria-hidden="true">·</span>
          <span>Detector <span className="font-mono text-fg-secondary">{finding.sourceDetector}</span></span>
          <AgencyMeta portalKey={finding.subject.portalKey} agencyId={finding.subject.agencyId} />
        </p>
        <div className="mt-4 flex flex-wrap items-center gap-2">
          <Button size="sm" variant="primary" onClick={() => openLeadDossier(finding.id)}>Open in inspector</Button>
          <Button size="sm" onClick={inspectRecords}>Inspect records</Button>
          {openQuery && <Button size="sm" icon="database" onClick={openQuery}>Open the query</Button>}
          {actions ?? (briefing && <Button size="sm" variant="ghost" onClick={() => addBriefingEvidence(briefing)}>Add to briefing</Button>)}
          {notice}
        </div>
      </header>

      <div className="space-y-7 px-5 py-6">
        {'error' in figure && <InlineError summary="The lead’s figure couldn’t be read." detail={figure.error} />}
        {drawn ? (
          <LeadFigure
            figure={drawn}
            headline={null}
            size="full"
            sources={{ tables: sources?.tables.map((table) => table.parts?.name ?? table.line) ?? ['tables not recorded with this finding'], retrievedAt: sources?.tables[0]?.parts?.retrievedOn }}
            onActivate={openQuery}
          />
        ) : storyLead ? (
          <AttentionFactors lead={storyLead} rank={rank} />
        ) : null}

        {text && <Statement text={text} counted={counted} />}

        <section aria-labelledby="stage-chain">
          <h3 id="stage-chain" className="mb-3 text-overline uppercase text-muted">Evidence chain</h3>
          <EvidenceChain dossier={chain} />
        </section>

        <section aria-labelledby="stage-disconfirm" className="rounded-lg border border-dashed border-line-strong px-4 py-3.5">
          <h3 id="stage-disconfirm" className="text-overline uppercase text-muted">What would disconfirm this</h3>
          <p className="mt-1.5 text-body text-fg-secondary">{dossier.falsification.statement}</p>
          {dossier.falsification.recordsToSeek.length > 0 && (
            <ul className="mt-2 flex flex-wrap gap-1.5">
              {dossier.falsification.recordsToSeek.map((record) => <li key={record}><Badge>{record}</Badge></li>)}
            </ul>
          )}
        </section>
      </div>
    </article>
  );
}

/** Step through the queue (J and K do the same anywhere on the page) and export the dossier. */
function StageActions({ position, dossier }: { position?: LeadPosition; dossier: LeadDossierData | null }) {
  const step = useAppStore((state) => state.stepWorkbenchLead);
  return (
    <>
      {dossier && (
        <Button
          size="sm"
          variant="ghost"
          className="px-2"
          title="Export this lead’s dossier as Markdown"
          onClick={() => downloadText(`lead-dossier-${dossier.finding.id}.md`, dossierMarkdown(dossier), 'text/markdown;charset=utf-8')}
        >
          Export
        </Button>
      )}
      {position && (
        <>
          <span aria-hidden="true" className="mx-1 h-4 w-px bg-line" />
          <Button size="sm" variant="ghost" iconOnly icon="back" aria-label="Previous lead" aria-keyshortcuts="K" title="Previous lead (K)" disabled={position.index <= 1} onClick={() => step(-1)} />
          <Button size="sm" variant="ghost" iconOnly icon="forward" aria-label="Next lead" aria-keyshortcuts="J" title="Next lead (J)" disabled={position.index >= position.total} onClick={() => step(1)} />
        </>
      )}
    </>
  );
}

/** The lead's own account of what the records show, clamped until the reader asks for all of it. */
function Statement({ text, counted }: { text: string; counted: boolean }) {
  const [open, setOpen] = useState(false);
  const long = text.length > 420;
  return (
    <section aria-labelledby="stage-statement">
      <h3 id="stage-statement" className="text-overline uppercase text-muted">{counted ? 'What was counted' : 'What the records show'}</h3>
      <p id="stage-statement-text" className={`mt-1.5 max-w-prose whitespace-pre-line text-body text-fg-secondary ${long && !open ? 'line-clamp-4' : ''}`}>{text}</p>
      {long && (
        <Button variant="link" size="sm" className="mt-1 h-auto px-0" aria-expanded={open} aria-controls="stage-statement-text" onClick={() => setOpen((value) => !value)}>
          {open ? 'Show less' : 'Read all of it'}
        </Button>
      )}
    </section>
  );
}

function AgencyMeta({ portalKey, agencyId }: { portalKey: string | null; agencyId: number | null }) {
  const profile = useAppStore((state) => agencyId === null ? null : state.agencyProfiles.find((entry) => entry.agencyId === agencyId && entry.portalSubdomain === portalKey) ?? null);
  if (agencyId === null || !portalKey || !profile) return null;
  return (
    <>
      <span aria-hidden="true">·</span>
      <span>About <ObjectLink target={{ kind: 'agency', portalKey, agencyId, label: profile.agencyName }}>{profile.agencyName}</ObjectLink></span>
    </>
  );
}

function uniqueDocuments(evidence: readonly EvidenceLink[]) {
  const seen = new Map<string, { link: EvidenceLink; passages: number }>();
  for (const link of evidence) {
    const key = `${link.portalKey}:${link.documentId}`;
    const entry = seen.get(key);
    if (entry) entry.passages += 1;
    else seen.set(key, { link, passages: 1 });
  }
  return [...seen.values()];
}

function Context({ dossier, onSelectFinding }: { dossier: LeadDossierData; onSelectFinding?(id: string): boolean }) {
  const { finding } = dossier;
  const sources = useMemo(() => tableLeadSources(finding.explanation), [finding.explanation]);
  const documents = uniqueDocuments(dossier.evidence);
  const requests = [...new Map(
    [
      ...(finding.subject.requestId !== null && finding.subject.portalKey ? [{ portalKey: finding.subject.portalKey, requestId: finding.subject.requestId }] : []),
      ...dossier.evidence.filter((link) => link.requestId !== null).map((link) => ({ portalKey: link.portalKey, requestId: link.requestId as number })),
    ].map((request) => [`${request.portalKey}:${request.requestId}`, request]),
  ).values()];
  const inspect = useAppStore((state) => state.inspect);
  return (
    <div className="divide-y divide-line">
      <PaneSection title="Cited records" count={documents.length || undefined}>
        {documents.length === 0 ? (
          <p className="text-body text-muted">{sources ? 'No document passage: this lead was computed from the tables below.' : 'This finding cites no document passage.'}</p>
        ) : (
          <ul className="space-y-2.5">
            {documents.map(({ link, passages }) => (
              <li key={`${link.portalKey}:${link.documentId}`} className="flex gap-2.5">
                <span aria-hidden="true" className="mt-0.5 grid h-6 w-6 shrink-0 place-items-center rounded-md bg-raised text-fg-secondary"><Icon name="evidence" size="sm" /></span>
                <span className="min-w-0">
                  {link.fileName ? (
                    <ObjectLink
                      target={{ kind: 'document', portalKey: link.portalKey, documentId: link.documentId, requestId: link.requestId, characterStart: link.characterStart, characterEnd: link.characterEnd, label: link.fileName }}
                      className="block max-w-full truncate text-body font-medium"
                    >
                      {link.fileName}
                    </ObjectLink>
                  ) : (
                    <span className="block text-body text-warning">Document {link.documentId} not resolved to a stored file</span>
                  )}
                  <span className="block truncate text-caption text-muted">
                    {passages} passage{passages === 1 ? '' : 's'} cited{link.agencyLabel ? ` · ${link.agencyLabel}` : ''}
                  </span>
                </span>
              </li>
            ))}
          </ul>
        )}
      </PaneSection>

      <PaneSection title="Requests" count={requests.length || undefined}>
        {requests.length === 0 ? (
          <p className="text-body text-muted">No records request is linked to this lead.</p>
        ) : (
          <ul className="flex flex-wrap gap-1.5">
            {requests.map((request) => (
              <li key={`${request.portalKey}:${request.requestId}`}>
                <button
                  type="button"
                  onClick={() => inspect({ kind: 'request', portalKey: request.portalKey, requestId: request.requestId })}
                  className="focus-ring inline-flex h-7 items-center gap-1.5 rounded-md border border-line bg-surface px-2 text-caption font-medium tabular-nums text-fg-secondary hover:border-line-strong hover:text-fg"
                >
                  <Icon name="clipboard" size="xs" />
                  Request {request.requestId}
                </button>
              </li>
            ))}
          </ul>
        )}
      </PaneSection>

      <PaneSection title="Related findings" count={dossier.related.length ? `${dossier.related.length}${dossier.relatedTruncated ? '+' : ''}` : undefined}>
        {dossier.related.length === 0 ? (
          <p className="text-body text-muted">No other open finding touches this request or these documents.</p>
        ) : (
          <ul className="space-y-1">
            {dossier.related.map((related) => (
              <li key={related.id}>
                <button
                  type="button"
                  onClick={() => { if (!onSelectFinding?.(related.id)) inspect({ kind: 'finding', id: related.id, label: related.headline }); }}
                  className="focus-ring -mx-2 flex w-[calc(100%+1rem)] items-start gap-2 rounded-md px-2 py-1.5 text-left hover:bg-raised/60"
                >
                  <SeverityGlyph severity={related.severity} />
                  <span className="min-w-0">
                    <span className="line-clamp-2 block text-body text-fg">{related.headline}</span>
                    <span className="block text-caption text-muted">{related.basis === 'SAME_REQUEST' ? 'Same request' : 'Cites the same document'}</span>
                  </span>
                </button>
              </li>
            ))}
          </ul>
        )}
      </PaneSection>

      {sources && <TablesSection sources={sources} />}

      <PaneSection title="Gaps" count={dossier.gaps.length ? `${dossier.gaps.length}${dossier.gapsTruncated ? '+' : ''}` : undefined}>
        {dossier.gaps.length === 0 ? (
          <p className="text-body text-muted">Every figure found in the cited documents could be checked, or none was found.</p>
        ) : (
          <ul className="space-y-3">
            {dossier.gaps.map((gap, index) => (
              <li key={index} className="text-body text-fg-secondary">
                <p className="break-words">{gapLine(gap)}</p>
                <GapRequest gap={gap} />
              </li>
            ))}
          </ul>
        )}
      </PaneSection>

      <DraftSection findingId={finding.id} />
    </div>
  );
}

function TablesSection({ sources }: { sources: TableLeadSources }) {
  return (
    <PaneSection title="Tables read" count={sources.tables.length}>
      <ul className="space-y-2.5">
        {sources.tables.map((table) => (
          <li key={table.line} className="flex gap-2.5">
            <span aria-hidden="true" className="mt-0.5 grid h-6 w-6 shrink-0 place-items-center rounded-md bg-raised text-fg-secondary"><Icon name="database" size="sm" /></span>
            {table.parts === null ? <span className="min-w-0 break-words text-body text-fg-secondary">{table.line}</span> : (
              <span className="min-w-0">
                <span className="block text-body font-medium text-fg">{table.parts.name}</span>
                <span className="block text-caption text-muted">Retrieved {table.parts.retrievedOn}</span>
                <span className="block truncate font-mono text-code text-muted" title={table.parts.sha256}>SHA-256 {table.parts.sha256.slice(0, 16)}…</span>
              </span>
            )}
          </li>
        ))}
      </ul>
      <details className="mt-3">
        <summary className="focus-ring w-fit cursor-pointer rounded text-caption text-muted hover:text-fg">The query the figures were computed with</summary>
        <pre className="mt-2 max-h-64 overflow-auto whitespace-pre-wrap break-words rounded-md bg-raised/60 p-3 font-mono text-code text-fg-secondary">{sources.sql}</pre>
      </details>
    </PaneSection>
  );
}

/** The drafts panel mounts only once opened, so selecting a lead does not ask which drafts apply. */
function DraftSection({ findingId }: { findingId: string }) {
  const [open, setOpen] = useState(false);
  return (
    <PaneSection title="Draft from this lead">
      {open ? <ArtifactDrafts key={findingId} findingId={findingId} /> : (
        <Button size="sm" icon="filePlus" className="w-full justify-center" onClick={() => setOpen(true)}>Draft a request or appeal</Button>
      )}
    </PaneSection>
  );
}

function StageSkeleton() {
  return (
    <div role="status" aria-label="Assembling the lead" className="px-6 py-5">
      <div className="flex gap-2"><div className="skeleton h-5 w-24 rounded-full" /><div className="skeleton h-5 w-28 rounded-full" /></div>
      <div className="skeleton mt-4 h-6 w-4/5 rounded" />
      <div className="skeleton mt-2 h-6 w-3/5 rounded" />
      <div className="skeleton mt-3 h-3 w-2/5 rounded" />
      <div className="mt-5 flex gap-2"><div className="skeleton h-7 w-32 rounded-md" /><div className="skeleton h-7 w-28 rounded-md" /></div>
      <div className="skeleton mt-8 h-44 w-full rounded-lg" />
    </div>
  );
}

function ContextSkeleton() {
  return (
    <div aria-hidden="true" className="space-y-5 px-4 py-4">
      {[0, 1, 2].map((block) => (
        <div key={block} className="space-y-2">
          <div className="skeleton h-3 w-24 rounded" />
          <div className="skeleton h-4 w-full rounded" />
          <div className="skeleton h-4 w-4/5 rounded" />
        </div>
      ))}
    </div>
  );
}
