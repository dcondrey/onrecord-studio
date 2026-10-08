import { useEffect, useState } from 'react';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import { useAppStore } from '../../store/useAppStore';
import type { DossierGap, EvidenceLink, LeadDossier as LeadDossierData, LeadFigure as LeadFigureData } from '../../types';
import LeadFigure from '../charts/figures/LeadFigure';
import { useDiscoveryController } from './AutomaticDiscoveryProvider';
import { leadProse, tableLeadSources } from '../../utils/tableLeadSources';
import { findingToBriefingEvidence } from './BriefingWorkspace';
import ArtifactDrafts from './ArtifactDrafts';
import EvidenceChain from '../objects/EvidenceChain';
import ObjectLink from '../objects/ObjectLink';
import { downloadText } from './textDownload';
import Badge from '../ui/Badge';
import Button from '../ui/Button';
import InlineError from '../ui/InlineError';

/** A stored lead's figure, none (not a lead, cut short, or stored before figures), or why it could not be read. */
export type FigureState = { figure: LeadFigureData | null } | { error: string };

export type DossierState =
  | { status: 'pending' }
  | { status: 'ready'; dossier: LeadDossierData; figure: FigureState }
  | { status: 'failed'; message: string };


const PENDING: DossierState = { status: 'pending' };

/**
 * The dossier and stored figure of one finding, read together under one request gate: a read for
 * a finding the caller has since left is discarded, never shown as this one's. The answer is held
 * with the id it answers, so the render that first sees a new id shows pending, not the last one.
 */
export function useLeadDossier(findingId: string): DossierState {
  const [held, setHeld] = useState<{ findingId: string; state: DossierState } | null>(null);
  const gate = useLatestRequestGate();

  useEffect(() => {
    const request = gate.begin();
    const setState = (state: DossierState) => setHeld({ findingId, state });
    setHeld(null);
    // The figure is read under the same request as the dossier, so both describe the same finding.
    const figure: Promise<FigureState> = invokeCommand('list_findings', { filter: { id: findingId, includeDismissed: true }, limit: 1 })
      .then((found) => ({ figure: found[0]?.figure ?? null }), (reason: unknown) => ({ error: formatNativeError(reason) }));
    Promise.all([invokeCommand('build_lead_dossier', { findingId }), figure])
      .then(([dossier, read]) => {
        if (!gate.isCurrent(request)) return;
        setState({ status: 'ready', dossier, figure: read });
      })
      .catch((reason) => {
        if (!gate.isCurrent(request)) return;
        setState({ status: 'failed', message: formatNativeError(reason) });
      });
    return gate.invalidate;
  }, [findingId, gate]);

  return held !== null && held.findingId === findingId ? held.state : PENDING;
}

/**
 * Assembles one finding into a dossier: the records it cites, the findings that touch the same
 * request or documents, what would disconfirm it, and which figures in the cited documents could
 * not be checked. Every fetch is gated on the finding it was requested for.
 */
export default function LeadDossier({ findingId }: { findingId: string }) {
  const addBriefingEvidence = useAppStore((state) => state.addBriefingEvidence);
  const state = useLeadDossier(findingId);

  const addToBriefing = (dossier: LeadDossierData) => {
    const item = findingToBriefingEvidence(dossier.finding);
    if (item) addBriefingEvidence(item);
  };

  const exportMarkdown = (dossier: LeadDossierData) =>
    downloadText(`lead-dossier-${dossier.finding.id}.md`, dossierMarkdown(dossier), 'text/markdown;charset=utf-8');

  return (
    <section aria-label="Lead dossier" className="text-body text-fg-secondary">
      {state.status === 'pending' && <DossierSkeleton />}
      {state.status === 'failed' && <InlineError className="mt-2" summary="The dossier couldn’t be assembled." detail={state.message} />}
      {state.status === 'ready' && (
        <div className="space-y-4">
          <LeadFigureSection dossier={state.dossier} figure={state.figure} />
          <Section title="Draft from this finding">
            <ArtifactDrafts findingId={state.dossier.finding.id} />
          </Section>
          <EvidenceChain dossier={state.dossier} />
          <MethodDetails dossier={state.dossier} />
          <Section title={`Related findings (${state.dossier.related.length}${state.dossier.relatedTruncated ? ', more not shown' : ''})`}>
            {state.dossier.related.length === 0 ? (
              <p className="text-muted">No other undismissed finding touches this request or these documents.</p>
            ) : (
              <ul className="space-y-1.5">
                {state.dossier.related.map((related) => (
                  <li key={related.id} className="flex flex-wrap items-center gap-x-2 gap-y-1">
                    <ObjectLink target={{ kind: 'finding', id: related.id, label: related.headline }}>{related.headline}</ObjectLink>
                    <Badge>{related.basis === 'SAME_REQUEST' ? 'Same request' : 'Same document'}</Badge>
                  </li>
                ))}
              </ul>
            )}
          </Section>
          <Section title="What would disconfirm this">
            <p>{state.dossier.falsification.statement}</p>
            <p className="mt-1 text-muted">Records to seek:</p>
            <ul className="list-disc pl-4 text-muted">
              {state.dossier.falsification.recordsToSeek.map((record) => <li key={record}>{record}</li>)}
            </ul>
          </Section>
          <Section title={`Gaps (${state.dossier.gaps.length}${state.dossier.gapsTruncated ? ', more not shown' : ''})`}>
            {state.dossier.gaps.length === 0 ? (
              <p className="text-muted">Every figure found in the cited documents could be checked against the imported tables, or none was found.</p>
            ) : (
              <ul className="space-y-2">
                {state.dossier.gaps.map((gap, index) => (
                  <li key={index}>
                    <p>{gapLine(gap, state.dossier.evidence)}</p>
                    <GapRequest gap={gap} evidence={state.dossier.evidence} />
                  </li>
                ))}
              </ul>
            )}
          </Section>
          <div className="flex flex-wrap gap-2">
            <Button size="sm" onClick={() => addToBriefing(state.dossier)}>Add to briefing</Button>
            <Button size="sm" variant="ghost" onClick={() => exportMarkdown(state.dossier)}>Export as Markdown</Button>
          </div>
        </div>
      )}
    </section>
  );
}

/**
 * The inspector leads with what can be seen: a stored figure, with the lead's prose under 'Figures
 * as text' so the wording stays citable. A table lead stored without one says so rather than
 * leaving a blank; a document finding has no figure to lack.
 */
export function LeadFigureSection({ dossier, figure }: { dossier: LeadDossierData; figure: FigureState }) {
  const controller = useDiscoveryController();
  const sources = tableLeadSources(dossier.finding.explanation);
  const drawn = 'figure' in figure ? figure.figure : null;
  if (!drawn) {
    if ('error' in figure) return <InlineError summary="The lead's figure couldn’t be read." detail={figure.error} />;
    return sources ? <p className="text-caption text-muted">No stored figure for this lead.</p> : null;
  }
  const prose = leadProse(dossier.finding.explanation);
  const tables = sources?.tables.map((table) => table.parts?.name ?? table.line) ?? [];
  // A mark opens the lead's own query in Analysis, as its card does.
  const sql = sources?.sql;
  const onActivate = controller && sql ? () => { controller.inspect(sql); useAppStore.getState().setWorkspaceView('analysis'); } : undefined;
  return (
    <>
      <LeadFigure
        figure={drawn}
        headline={dossier.finding.headline}
        size="full"
        sources={{ tables: tables.length ? tables : ['tables not recorded with this finding'], retrievedAt: sources?.tables[0]?.parts?.retrievedOn }}
        onActivate={onActivate}
      />
      <details className="text-body">
        <summary className="focus-ring w-fit cursor-pointer rounded text-caption text-muted hover:text-fg">Figures as text</summary>
        <p className="mt-1 whitespace-pre-line">{prose}</p>
      </details>
    </>
  );
}

/**
 * How the finding was produced, folded: the detector, and for a table lead the tables it read
 * (source, retrieval date and full digest, broken anywhere so nothing runs past the panel) and the
 * query. A document finding has no tables and gets no table or query section.
 */
function MethodDetails({ dossier }: { dossier: LeadDossierData }) {
  const sources = tableLeadSources(dossier.finding.explanation);
  return (
    <div className="divide-y divide-line border-y border-line">
      <Fold title="Method">
        <p>Detector <span className="break-all font-mono text-fg">{dossier.finding.sourceDetector}</span></p>
      </Fold>
      {sources && (
        <>
          <Fold title={`Tables read (${sources.tables.length})`}>
            <ul className="space-y-2">
              {sources.tables.map((table) => (
                <li key={table.line} className="break-all">
                  {table.parts === null ? table.line : (
                    <>
                      <span className="font-medium text-fg">{table.parts.name}</span>
                      <span className="block break-all">Source: {table.parts.source}</span>
                      <span className="block">Retrieved: {table.parts.retrievedOn}</span>
                      <span className="block break-all font-mono text-code">SHA-256: {table.parts.sha256}</span>
                    </>
                  )}
                </li>
              ))}
            </ul>
          </Fold>
          <Fold title="SQL">
            <pre aria-label="Query the figures were computed with" className="max-h-80 overflow-auto whitespace-pre-wrap break-all rounded-md bg-raised/60 p-3 font-mono text-code">{sources.sql}</pre>
          </Fold>
        </>
      )}
    </div>
  );
}

function Fold({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <details className="group py-2">
      <summary className="focus-ring flex w-full cursor-pointer list-none items-center gap-1.5 rounded text-body font-medium text-fg [&::-webkit-details-marker]:hidden">
        {title}
      </summary>
      <div className="mt-2 pl-4">{children}</div>
    </details>
  );
}

export function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div>
      <h3 className="text-body font-medium text-fg">{title}</h3>
      <div className="mt-1.5">{children}</div>
    </div>
  );
}

function evidenceLine(link: EvidenceLink): string {
  const name = link.fileName ?? `document ${link.documentId} (not resolved)`;
  const span = link.characterStart === null ? '' : ` · characters ${link.characterStart}–${link.characterEnd ?? link.characterStart}`;
  const agency = link.agencyLabel ? ` · ${link.agencyLabel}` : '';
  const digest = link.rawFileSha256 ? ` · sha256 ${link.rawFileSha256.slice(0, 12)}…` : '';
  const quote = link.quote ? ` — “${link.quote}”` : '';
  return `${link.portalKey} / ${name}${agency}${span}${digest}${quote}`;
}

function DossierSkeleton() {
  return (
    <div role="status" aria-label="Assembling the dossier" className="space-y-2">
      <div className="skeleton h-4 w-3/4 rounded" />
      <div className="skeleton h-3 w-full rounded" />
      <div className="skeleton h-3 w-5/6 rounded" />
      <div className="skeleton mt-4 h-20 w-full rounded-lg" />
    </div>
  );
}

type CopyState = { kind: 'idle' } | { kind: 'copied' } | { kind: 'fallback'; text: string };

/**
 * Offers the records request that would close one gap. The text asks for the records that would
 * confirm or refute the figure; it never states that the claim is wrong. Composed locally from the
 * gap's own fields, so nothing in it goes beyond what the dossier recorded.
 */
export function GapRequest({ gap, evidence }: { gap: DossierGap; evidence: readonly EvidenceLink[] }) {
  const [copy, setCopy] = useState<CopyState>({ kind: 'idle' });

  const copyRequest = async () => {
    const text = gapRequestText(gap, evidence);
    try {
      await navigator.clipboard.writeText(text);
      setCopy({ kind: 'copied' });
    } catch {
      setCopy({ kind: 'fallback', text });
    }
  };

  return (
    <div className="mt-1">
      <div className="flex flex-wrap items-center gap-2">
        <button
          type="button"
          onClick={() => void copyRequest()}
          className="focus-ring rounded-md border border-control px-2 py-0.5 text-caption font-medium text-muted hover:text-fg"
        >
          Copy request text
        </button>
        {copy.kind === 'copied' && <span role="status" className="text-caption text-success">Request text copied.</span>}
      </div>
      {copy.kind === 'fallback' && (
        <>
          <p role="status" className="mt-1 text-caption text-warning">Copying was blocked by the system. Select the text below to copy it.</p>
          <pre className="mt-1 whitespace-pre-wrap rounded-md border border-line bg-canvas/70 p-2 text-caption text-fg-secondary">{copy.text}</pre>
        </>
      )}
    </div>
  );
}

/** A neutral public-records request paragraph for the records that would close `gap`. */
function gapRequestText(gap: DossierGap, evidence: readonly EvidenceLink[]): string {
  const fileName = gapFileName(gap, evidence);
  const record = `${fileName ? `“${fileName}” (` : ''}document ${gap.documentId} held under ${gap.portalKey}${fileName ? ')' : ''}`;
  if (gap.gap === 'notComputed') {
    return (
      `I request a complete, readable electronic copy of ${record}, together with its metadata ` +
      `(creation and revision dates, author or originating office, and the source of any figures it contains). ` +
      `The copy available to me could not be examined (${gap.reason}), so the figures in it could not be checked ` +
      `against the imported records. Please provide the record in a form that allows those figures to be confirmed or refuted.`
    );
  }
  return (
    `I request the dataset, tables, or working figures that support the following statement in ${record}: ` +
    `"${gap.claimText}". The statement was compared against the imported records and could not be reconciled ` +
    `(${gap.reason}: ${gap.detail}). Please provide the underlying records, including the source data, its date, ` +
    `and any revision it reflects, in an electronic format if available, so that the stated figure can be confirmed or refuted.`
  );
}

/** The file name the dossier's evidence resolved for a gap's document; null when it cites none. */
function gapFileName(gap: DossierGap, evidence: readonly EvidenceLink[]): string | null {
  return evidence.find((link) => link.portalKey === gap.portalKey && link.documentId === gap.documentId && link.fileName)?.fileName ?? null;
}

export function gapLine(gap: DossierGap, evidence: readonly EvidenceLink[]): string {
  const record = gapFileName(gap, evidence) ?? `${gap.portalKey} / document ${gap.documentId}`;
  if (gap.gap === 'notComputed') return `${record}: figures not examined — ${gap.reason}`;
  return `${record}: “${gap.claimText}” — ${gap.reason}: ${gap.detail}`;
}

export function dossierMarkdown(dossier: LeadDossierData): string {
  const lines = [
    `# Lead dossier: ${dossier.finding.headline}`,
    '',
    dossier.finding.explanation,
    '',
    '## Evidence chain',
    ...(dossier.evidence.length ? dossier.evidence.map((link) => `- ${evidenceLine(link)}`) : ['- This finding cites no document span.']),
    '',
    '## Related findings',
    ...(dossier.related.length ? dossier.related.map((related) => `- ${related.headline}`) : ['- None.']),
    '',
    '## What would disconfirm this',
    dossier.falsification.statement,
    ...dossier.falsification.recordsToSeek.map((record) => `- ${record}`),
    '',
    '## Gaps',
    ...(dossier.gaps.length ? dossier.gaps.map((gap) => `- ${gapLine(gap, dossier.evidence)}`) : ['- Every figure found could be checked, or none was found.']),
    '',
    `Assembled ${dossier.assembledAt}. Observations only; a pattern signal is not a legal claim.`,
  ];
  return lines.join('\n');
}
