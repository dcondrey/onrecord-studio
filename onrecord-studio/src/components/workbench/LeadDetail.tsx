import { useLayoutEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { useAppStore } from '../../store/useAppStore';
import type { EvidenceLink, LeadDossier as LeadDossierData, StoryLead } from '../../types';
import { isLegalClaimKind } from '../../types/findings';
import { leadProse, tableLeadSources, type TableLeadSources } from '../../utils/tableLeadSources';
import { scrollingAncestor, scrollWithin } from '../../utils/scrollWithin';
import AttentionFactors from '../charts/figures/AttentionFactors';
import LeadFigure from '../charts/figures/LeadFigure';
import { useDiscoveryController } from '../dashboard/AutomaticDiscoveryProvider';
import ArtifactDrafts from '../dashboard/ArtifactDrafts';
import { findingToBriefingEvidence } from '../dashboard/BriefingWorkspace';
import { dossierMarkdown, GapRequest, gapLine, useLeadDossier, type FigureState } from '../dashboard/LeadDossier';
import SeverityGlyph from '../dashboard/SeverityGlyph';
import { downloadText } from '../dashboard/textDownload';
import EvidenceChain from '../objects/EvidenceChain';
import ObjectLink from '../objects/ObjectLink';
import Badge from '../ui/Badge';
import Button from '../ui/Button';
import Icon from '../ui/icons';
import InlineError from '../ui/InlineError';
import { keepIdentifiers, LeadByline, LeadKicker } from './LeadMeta';
import { Pane, PaneSection } from './Workbench';

const DRAFT_SECTION_ID = 'lead-draft';

export interface LeadPosition {
  /** 1-based rank, which is the lead's position in the queue. */
  index: number;
  total: number;
}

/**
 * The stage and context panes for one finding, from one gated dossier read: the stage states the
 * lead and lays out its figure and evidence; the context lists the records, requests and findings
 * around it. Both panes describe the finding the read was made for, or neither renders.
 */
export default function LeadDetail({ findingId, storyLead, position, compare, source, onSelectFinding, actions, notice }: {
  findingId: string;
  /** The ranker's record of this finding: drawn as why it ranks here. */
  storyLead?: StoryLead | null;
  position?: LeadPosition;
  /** The lead the factors are compared with: #2 for #1, otherwise the lead just above. */
  compare?: { lead: StoryLead; rank: number };
  /** The lead's source, named only when the queue spans more than one. */
  source?: string;
  /** Selects a related finding in this workbench; false when it is not in the queue. */
  onSelectFinding?(id: string): boolean;
  /** Replaces the default Add to briefing action (Story desk reports its outcome beside it). */
  actions?: ReactNode;
  notice?: ReactNode;
}) {
  const state = useLeadDossier(findingId);
  const ready = state.status === 'ready' ? state : null;
  // The stage's Draft action opens the context pane's drafts; the two panes share this switch.
  const [draft, setDraft] = useState<{ findingId: string; open: boolean }>({ findingId, open: false });
  const draftOpen = draft.findingId === findingId && draft.open;
  const openDraft = () => {
    setDraft({ findingId, open: true });
    requestAnimationFrame(() => {
      const section = document.getElementById(DRAFT_SECTION_ID);
      if (!section) return;
      const container = scrollingAncestor(section);
      if (container) scrollWithin(container, section, 'start');
      section.querySelector<HTMLElement>('button:not(:disabled), input, select, textarea, a[href]')?.focus({ preventScroll: true });
    });
  };
  const headline = storyLead?.headline ?? ready?.dossier.finding.headline;
  return (
    <>
      <Pane
        label="Selected lead"
        title="Lead"
        role="stage"
        meta={position ? `#${position.index} of ${position.total.toLocaleString()}` : undefined}
        actions={<StageActions position={position} dossier={ready?.dossier ?? null} announcement={position && headline ? `Lead ${position.index} of ${position.total}: ${headline}` : ''} />}
        elevated
      >
        {state.status === 'failed' ? (
          <div className="p-6"><InlineError summary="This lead’s dossier couldn’t be assembled." detail={state.message} /></div>
        ) : !ready ? (
          <StageSkeleton />
        ) : (
          <Stage
            key={ready.dossier.finding.id}
            dossier={ready.dossier}
            figure={ready.figure}
            storyLead={storyLead ?? null}
            rank={position?.index}
            compare={compare}
            source={source}
            onDraft={openDraft}
            actions={actions}
            notice={notice}
          />
        )}
      </Pane>
      <Pane label="Records around this lead" title="Around this lead" role="context">
        {state.status === 'failed' ? (
          <p className="px-4 py-3 text-body text-muted">Nothing to list: the dossier didn’t load.</p>
        ) : !ready ? (
          <ContextSkeleton />
        ) : (
          <Context
            key={ready.dossier.finding.id}
            dossier={ready.dossier}
            onSelectFinding={onSelectFinding}
            draftOpen={draftOpen}
            onOpenDraft={() => setDraft({ findingId, open: true })}
          />
        )}
      </Pane>
    </>
  );
}

function Stage({ dossier, figure, storyLead, rank, compare, source, onDraft, actions, notice }: {
  dossier: LeadDossierData;
  figure: FigureState;
  storyLead: StoryLead | null;
  rank?: number;
  compare?: { lead: StoryLead; rank: number };
  source?: string;
  onDraft(): void;
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
  const factors = storyLead && rank !== undefined ? <AttentionFactors lead={storyLead} rank={rank} compare={compare} /> : null;

  return (
    <article className="motion-fade" aria-labelledby="stage-headline">
      <header className="border-b border-line px-6 pb-5 pt-5">
        <LeadKicker kind={finding.kind} severity={finding.severity} />
        <h2 id="stage-headline" className="stage-headline mt-2 max-w-[52rem] text-balance text-fg">{keepIdentifiers(finding.headline)}</h2>
        <Dek text={leadProse(finding.explanation).trim()} label={counted ? 'What was counted' : 'What the records show'} />
        <LeadByline source={source} firstSeen={finding.discoveredAt} portalKey={finding.subject.portalKey} agencyId={finding.subject.agencyId} className="mt-3" />
        <div className="mt-4 flex flex-wrap items-center gap-2">
          <Button size="sm" variant="primary" onClick={() => openLeadDossier(finding.id)}>Open evidence chain</Button>
          <Button size="sm" onClick={inspectRecords}>Inspect records</Button>
          <Button size="sm" icon="filePlus" onClick={onDraft}>Draft records request</Button>
          {openQuery && <Button size="sm" icon="database" onClick={openQuery}>Open the query</Button>}
          {actions ?? (briefing && <Button size="sm" variant="ghost" onClick={() => addBriefingEvidence(briefing)}>Add to briefing</Button>)}
          {notice}
        </div>
      </header>

      <div className="space-y-7 px-6 py-6">
        {'error' in figure && <InlineError summary="The lead’s figure couldn’t be read." detail={figure.error} />}
        {drawn ? (
          <>
            <LeadFigure
              figure={drawn}
              headline={null}
              size="full"
              sources={{ tables: sources?.tables.map((table) => table.parts?.name ?? table.line) ?? ['tables not recorded with this finding'], retrievedAt: sources?.tables[0]?.parts?.retrievedOn }}
              onActivate={openQuery}
            />
            {factors && (
              <details className="group">
                <summary className="focus-ring inline-flex w-fit cursor-pointer list-none items-center gap-1.5 rounded text-overline uppercase text-muted hover:text-fg [&::-webkit-details-marker]:hidden">
                  <Icon name="chevron" size="xs" className="transition-transform group-open:rotate-90" />
                  Why it ranks here
                </summary>
                <div className="mt-3">{factors}</div>
              </details>
            )}
          </>
        ) : factors}

        {dossier.evidence.length > 0 || !sources ? (
          <section aria-labelledby="stage-chain">
            <h3 id="stage-chain" className="mb-3 text-overline uppercase text-muted">Evidence chain</h3>
            <EvidenceChain dossier={chain} />
          </section>
        ) : (
          <ComputedFrom sources={sources} detector={finding.sourceDetector} />
        )}

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

/**
 * The lead's own account under its headline, clamped to three lines. "Read all of it" appears only
 * when the clamp actually hides text, measured on the rendered block rather than guessed from a
 * character count.
 */
function Dek({ text, label }: { text: string; label: string }) {
  const [open, setOpen] = useState(false);
  const [clamped, setClamped] = useState(false);
  const block = useRef<HTMLParagraphElement>(null);
  useLayoutEffect(() => {
    const element = block.current;
    if (!element || open) return;
    const measure = () => setClamped(element.scrollHeight > element.clientHeight + 1);
    measure();
    if (typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, [open, text]);
  if (!text) return null;
  return (
    <section aria-label={label} className="mt-2.5">
      <p ref={block} id="stage-dek" className={`max-w-prose whitespace-pre-line text-pretty text-dek text-fg-secondary ${open ? '' : 'line-clamp-3'}`}>{text}</p>
      {(clamped || open) && (
        <Button variant="link" size="sm" className="mt-1 h-auto px-0" aria-expanded={open} aria-controls="stage-dek" onClick={() => setOpen((value) => !value)}>
          {open ? 'Show less' : 'Read all of it'}
        </Button>
      )}
    </section>
  );
}

/** What a table lead was computed from: each table with its retrieval date and digest, the query, the method. */
function ComputedFrom({ sources, detector }: { sources: TableLeadSources; detector: string }) {
  return (
    <section aria-labelledby="stage-computed">
      <h3 id="stage-computed" className="mb-3 text-overline uppercase text-muted">Computed from</h3>
      <ul className="space-y-3">
        {sources.tables.map((table) => (
          <li key={table.line} className="flex gap-3">
            <span aria-hidden="true" className="mt-0.5 grid h-7 w-7 shrink-0 place-items-center rounded-md bg-raised text-fg-secondary"><Icon name="database" size="sm" /></span>
            {table.parts === null ? <span className="min-w-0 break-words text-body text-fg-secondary">{table.line}</span> : (
              <span className="min-w-0">
                <span className="block text-body font-medium text-fg">{table.parts.name}</span>
                <span className="mt-0.5 flex flex-wrap items-center gap-x-1.5 text-caption text-muted">
                  <span>Retrieved {table.parts.retrievedOn}</span>
                  <span aria-hidden="true">·</span>
                  <DigestCopy sha256={table.parts.sha256} />
                </span>
              </span>
            )}
          </li>
        ))}
      </ul>
      <details className="group mt-4">
        <summary className="focus-ring inline-flex w-fit cursor-pointer list-none items-center gap-1.5 rounded text-caption text-fg-secondary hover:text-fg [&::-webkit-details-marker]:hidden">
          <Icon name="chevron" size="xs" className="transition-transform group-open:rotate-90" />
          Show the query
        </summary>
        <pre className="mt-2 max-h-64 overflow-auto whitespace-pre-wrap break-all rounded-md bg-raised/60 p-3 font-mono text-code text-fg-secondary">{sources.sql}</pre>
      </details>
      <p className="mt-3 text-caption text-muted">Method <span className="font-mono text-fg-secondary">{detector}</span></p>
    </section>
  );
}

function DigestCopy({ sha256 }: { sha256: string }) {
  const [copied, setCopied] = useState<boolean | null>(null);
  return (
    <span className="inline-flex items-center gap-1">
      <span className="font-mono text-code" title={`SHA-256 ${sha256}`}>SHA-256 {sha256.slice(0, 12)}…</span>
      <Button
        size="sm"
        variant="ghost"
        iconOnly
        icon="clipboard"
        className="h-6 w-6"
        aria-label="Copy the full SHA-256 digest"
        title={sha256}
        onClick={() => {
          void navigator.clipboard?.writeText(sha256).then(() => setCopied(true), () => setCopied(false));
        }}
      />
      {copied !== null && <span role="status" className={copied ? 'text-success' : 'text-warning'}>{copied ? 'Copied' : 'Copying was blocked'}</span>}
    </span>
  );
}

/** Step through the queue (J and K do the same anywhere on the page) and export the dossier. */
function StageActions({ position, dossier, announcement }: { position?: LeadPosition; dossier: LeadDossierData | null; announcement: string }) {
  const step = useAppStore((state) => state.stepWorkbenchLead);
  return (
    <>
      <span className="sr-only" aria-live="polite">{announcement}</span>
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

function Context({ dossier, onSelectFinding, draftOpen, onOpenDraft }: {
  dossier: LeadDossierData;
  onSelectFinding?(id: string): boolean;
  draftOpen: boolean;
  onOpenDraft(): void;
}) {
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
    <div className="context-sections">
      <PaneSection title="Cited records" count={documents.length || undefined} area="cited">
        {documents.length === 0 ? (
          <p className="text-body text-muted">{sources ? 'No document passage: this lead was computed from the tables on the stage.' : 'This finding cites no document passage.'}</p>
        ) : (
          <ul className="space-y-2.5">
            {documents.map(({ link, passages }) => (
              <li key={`${link.portalKey}:${link.documentId}`} className="flex gap-2.5">
                <span aria-hidden="true" className="mt-0.5 grid h-6 w-6 shrink-0 place-items-center rounded-md bg-raised text-fg-secondary"><Icon name="evidence" size="sm" /></span>
                <span className="min-w-0">
                  {link.fileName ? (
                    <ObjectLink
                      target={{ kind: 'document', portalKey: link.portalKey, documentId: link.documentId, requestId: link.requestId, characterStart: link.characterStart, characterEnd: link.characterEnd, label: link.fileName }}
                      className="block max-w-full text-body font-medium"
                    >
                      <span className="line-clamp-2 [overflow-wrap:anywhere]" title={link.fileName}>{link.fileName}</span>
                    </ObjectLink>
                  ) : (
                    <span className="block text-body text-warning">Document {link.documentId} not resolved to a stored file</span>
                  )}
                  <span className="block text-caption tabular-nums text-muted">{passages} passage{passages === 1 ? '' : 's'} cited</span>
                  {link.agencyLabel && <span className="block text-caption text-muted">{link.agencyLabel}</span>}
                </span>
              </li>
            ))}
          </ul>
        )}
      </PaneSection>

      <PaneSection title="Requests" count={requests.length || undefined} area="requests">
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

      <PaneSection title="Related findings" count={dossier.related.length ? `${dossier.related.length}${dossier.relatedTruncated ? '+' : ''}` : undefined} area="related">
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
                    <span className="line-clamp-2 block text-body text-fg">{keepIdentifiers(related.headline)}</span>
                    <span className="block text-caption text-muted">{related.basis === 'SAME_REQUEST' ? 'Same request' : 'Cites the same document'}</span>
                  </span>
                </button>
              </li>
            ))}
          </ul>
        )}
      </PaneSection>

      {/* With no passage the tables are the evidence, and the stage lists them. */}
      {sources && documents.length > 0 && <TablesSection sources={sources} />}

      <PaneSection title="Gaps" count={dossier.gaps.length ? `${dossier.gaps.length}${dossier.gapsTruncated ? '+' : ''}` : undefined} area="gaps">
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

      <PaneSection title="Draft from this lead" area="draft" id={DRAFT_SECTION_ID}>
        {draftOpen ? <ArtifactDrafts key={finding.id} findingId={finding.id} /> : (
          <Button size="sm" icon="filePlus" className="w-full justify-center" onClick={onOpenDraft}>Draft a request or appeal</Button>
        )}
      </PaneSection>
    </div>
  );
}

function TablesSection({ sources }: { sources: TableLeadSources }) {
  return (
    <PaneSection title="Tables read" count={sources.tables.length} area="tables">
      <ul className="space-y-2.5">
        {sources.tables.map((table) => (
          <li key={table.line} className="flex gap-2.5">
            <span aria-hidden="true" className="mt-0.5 grid h-6 w-6 shrink-0 place-items-center rounded-md bg-raised text-fg-secondary"><Icon name="database" size="sm" /></span>
            {table.parts === null ? <span className="min-w-0 break-words text-body text-fg-secondary">{table.line}</span> : (
              <span className="min-w-0">
                <span className="block text-body font-medium text-fg [overflow-wrap:anywhere]">{table.parts.name}</span>
                <span className="block text-caption text-muted">Retrieved {table.parts.retrievedOn}</span>
                <span className="block break-all font-mono text-code text-muted">SHA-256 {table.parts.sha256}</span>
              </span>
            )}
          </li>
        ))}
      </ul>
      <details className="mt-3">
        <summary className="focus-ring w-fit cursor-pointer rounded text-caption text-muted hover:text-fg">Show the query</summary>
        <pre className="mt-2 max-h-64 overflow-auto whitespace-pre-wrap break-all rounded-md bg-raised/60 p-3 font-mono text-code text-fg-secondary">{sources.sql}</pre>
      </details>
    </PaneSection>
  );
}

export function StageSkeleton() {
  return (
    <div role="status" aria-label="Assembling the lead" className="px-6 py-5">
      <div className="flex gap-2"><div className="skeleton h-3 w-24 rounded" /><div className="skeleton h-3 w-28 rounded" /></div>
      <div className="skeleton mt-3 h-6 w-4/5 rounded" />
      <div className="skeleton mt-2 h-6 w-3/5 rounded" />
      <div className="skeleton mt-4 h-3.5 w-full max-w-prose rounded" />
      <div className="skeleton mt-2 h-3.5 w-4/5 max-w-prose rounded" />
      <div className="mt-5 flex gap-2"><div className="skeleton h-7 w-36 rounded-md" /><div className="skeleton h-7 w-28 rounded-md" /><div className="skeleton h-7 w-40 rounded-md" /></div>
      <div className="skeleton mt-8 h-44 w-full rounded-lg" />
    </div>
  );
}

export function ContextSkeleton() {
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
