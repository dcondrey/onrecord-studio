import { useEffect, useState, type ReactNode } from 'react';
import { invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import type { EvidenceLink, LeadDossier, ProvenanceVerificationRecord } from '../../types';
import { cn } from '../../utils/cn';
import { leadProse } from '../../utils/tableLeadSources';
import Icon, { type IconName } from '../ui/icons';
import ObjectLink from './ObjectLink';
import { claimBasisLabel, confidenceLabel, findingKindLabel, formatShortDate, provenanceVerdict, severityLabel, severityTone } from './labels';

const PROVENANCE_LOOKUP_LIMIT = 12;

type ProvenanceState = { status: 'pending' } | { status: 'ready'; record: ProvenanceVerificationRecord | null } | { status: 'failed' };

/**
 * One finding drawn as the chain a reader must be able to walk: the claim, what it was measured
 * on, each cited passage, the file that holds it, and the last provenance check of that file.
 * A link the record does not have is drawn as missing, never filled in.
 */
export default function EvidenceChain({ dossier }: { dossier: LeadDossier }) {
  const { finding, evidence, evidenceTruncated } = dossier;
  const provenance = useProvenance(evidence);
  const tone = severityTone(finding.severity);
  const prose = leadProse(finding.explanation).trim();
  return (
    <ol aria-label="Evidence chain" className="relative space-y-0">
      <ChainNode icon="flag" tone={tone} title={`${severityLabel(finding.severity)} · ${findingKindLabel(finding.kind)}`} caption={claimBasisLabel(finding.kind)}>
        <p className="font-semibold text-fg">{finding.headline}</p>
        {/* A table lead's tables and query follow its prose; they are listed apart, in full. */}
        {prose && <p className="mt-1 whitespace-pre-line text-muted [overflow-wrap:anywhere]">{prose}</p>}
      </ChainNode>
      <ChainNode icon="sliders" tone={finding.confidence.kind === 'MEASURED' ? 'accent' : 'muted'} title="Basis" caption={confidenceLabel(finding.confidence)}>
        <p className="text-muted">
          Detector <span className="font-mono text-fg-secondary">{finding.sourceDetector}</span> · first seen {formatShortDate(finding.discoveredAt)}
        </p>
      </ChainNode>
      {evidence.length === 0 ? (
        <ChainNode icon="evidence" tone="warning" title="Sources" caption="None cited" last>
          <p className="text-warning">This finding cites no document passage, so nothing here can be checked against a record.</p>
        </ChainNode>
      ) : (
        evidence.map((link, index) => (
          <ChainNode
            key={`${link.portalKey}:${link.documentId}:${index}`}
            icon="evidence"
            tone={link.fileName ? 'accent' : 'warning'}
            title={`Source ${index + 1}${link.agencyLabel ? ` · ${link.agencyLabel}` : ''}`}
            caption={spanLabel(link)}
            last={index === evidence.length - 1 && !evidenceTruncated}
          >
            <SourceLink link={link} />
            {link.quote
              ? <blockquote className="mt-1.5 border-l-2 border-accent/40 pl-2 text-fg-secondary">“{link.quote}”</blockquote>
              : <p className="mt-1.5 text-muted">No quoted passage was recorded for this citation.</p>}
            <DigestLine link={link} provenance={provenance.get(provenanceKey(link))} />
          </ChainNode>
        ))
      )}
      {evidenceTruncated && (
        <ChainNode icon="evidence" tone="muted" title="More sources" caption="Not shown" last>
          <p className="text-muted">The dossier stopped at its source limit; more passages are cited than listed.</p>
        </ChainNode>
      )}
    </ol>
  );
}

function ChainNode({ icon, tone, title, caption, last, children }: { icon: IconName; tone: 'danger' | 'warning' | 'accent' | 'neutral' | 'muted'; title: string; caption: string; last?: boolean; children: ReactNode }) {
  return (
    <li className="motion-reveal relative pb-4 pl-9">
      {!last && <span aria-hidden="true" className="absolute bottom-0 left-[13px] top-7 w-px bg-line-strong" />}
      <span
        aria-hidden="true"
        className={cn(
          'absolute left-0 top-0 grid h-7 w-7 place-items-center rounded-full border',
          tone === 'danger' && 'border-danger/30 bg-danger/10 text-danger',
          tone === 'warning' && 'border-warning/30 bg-warning/10 text-warning',
          tone === 'accent' && 'border-accent/30 bg-accent/10 text-accent',
          tone === 'neutral' && 'border-line-strong bg-raised text-fg-secondary',
          tone === 'muted' && 'border-line bg-surface text-muted',
        )}
      >
        <Icon name={icon} className="h-3.5 w-3.5" />
      </span>
      <div className="flex flex-wrap items-baseline justify-between gap-x-3">
        <h3 className="text-caption font-semibold uppercase tracking-wider text-fg-secondary">{title}</h3>
        <span className="text-caption text-muted">{caption}</span>
      </div>
      <div className="mt-1 text-label">{children}</div>
    </li>
  );
}

function SourceLink({ link }: { link: EvidenceLink }) {
  if (!link.fileName) {
    return <p className="text-warning">Document {link.documentId} under {link.portalKey} could not be resolved to a stored file.</p>;
  }
  return (
    <ObjectLink
      target={{ kind: 'document', portalKey: link.portalKey, documentId: link.documentId, requestId: link.requestId, characterStart: link.characterStart, characterEnd: link.characterEnd, label: link.fileName }}
      className="font-medium"
    >
      {link.fileName}
    </ObjectLink>
  );
}

function DigestLine({ link, provenance }: { link: EvidenceLink; provenance: ProvenanceState | undefined }) {
  const digest = link.rawFileSha256
    ? <span>SHA-256 <span className="font-mono">{link.rawFileSha256.slice(0, 12)}…</span> recorded at capture</span>
    : <span className="text-warning">No capture digest recorded</span>;
  return (
    <p className="mt-1.5 flex flex-wrap items-center gap-x-2 gap-y-0.5 text-caption text-muted">
      <Icon name="shield" className="h-3 w-3" />
      {digest}
      <span aria-hidden="true">·</span>
      <ProvenanceLabel state={provenance} />
    </p>
  );
}

function ProvenanceLabel({ state }: { state: ProvenanceState | undefined }) {
  if (!state) return <span>Provenance not looked up</span>;
  if (state.status === 'pending') return <span className="skeleton inline-block h-3 w-28 rounded" aria-label="Checking provenance" />;
  if (state.status === 'failed') return <span className="text-warning">Provenance history unavailable</span>;
  if (!state.record) return <span>Never verified</span>;
  const verdict = provenanceVerdict(state.record);
  return <span className={verdict.tone === 'success' ? 'text-success' : verdict.tone === 'danger' ? 'text-danger' : undefined}>{verdict.text}</span>;
}

function spanLabel(link: EvidenceLink): string {
  if (link.characterStart === null) return 'Whole document';
  return `Characters ${link.characterStart.toLocaleString()}–${(link.characterEnd ?? link.characterStart).toLocaleString()}`;
}

function provenanceKey(link: EvidenceLink) {
  return `${link.portalKey}:${link.documentId}`;
}

/** The latest provenance verification of each resolved cited file, gated on the evidence list it was asked for. */
function useProvenance(evidence: EvidenceLink[]) {
  const [states, setStates] = useState<Map<string, ProvenanceState>>(new Map());
  const gate = useLatestRequestGate();

  useEffect(() => {
    const token = gate.begin();
    const unique = new Map<string, EvidenceLink>();
    for (const link of evidence) if (link.fileName && unique.size < PROVENANCE_LOOKUP_LIMIT) unique.set(provenanceKey(link), link);
    setStates(new Map([...unique.keys()].map((key) => [key, { status: 'pending' } as ProvenanceState])));
    for (const [key, link] of unique) {
      invokeCommand('list_document_provenance_verifications', { documentId: link.documentId, portalKey: link.portalKey, limit: 1 })
        .then((records) => {
          if (!gate.isCurrent(token)) return;
          setStates((current) => new Map(current).set(key, { status: 'ready', record: records[0] ?? null }));
        })
        .catch(() => {
          if (!gate.isCurrent(token)) return;
          setStates((current) => new Map(current).set(key, { status: 'failed' }));
        });
    }
    return gate.invalidate;
  }, [evidence, gate]);

  return states;
}
