import { useState, type ReactNode } from 'react';
import { invokeCommand, formatNativeError } from '../../ipc';
import { useGatedLoad, type LoadState } from '../../hooks/useGatedLoad';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import { useAppStore, type InspectorTarget } from '../../store/useAppStore';
import type { PortalRequestDeadline, DocumentProvenanceVerification, Finding, LegalComplianceFinding, StatutoryExtensionRecord } from '../../types';
import type { CaseworkCitation } from '../../types/casework';
import ClockStrip, { portalClock, recordFromEngine } from '../charts/figures/ClockStrip';
import { MiniClocks } from '../charts/figures/ClockStack';
import DocumentMinimap, { type MinimapMark } from '../charts/figures/DocumentMinimap';
import NeighbourBars from '../charts/figures/NeighbourBars';
import PeerStrip, { peerStripModel } from '../charts/figures/PeerStrip';
import RequestAccountingBar from '../charts/figures/RequestAccountingBar';
import { Meter } from '../charts/marks/Meter';
import { cn } from '../../utils/cn';
import LeadDossier from '../dashboard/LeadDossier';
import Icon from '../ui/icons';
import ObjectLink from './ObjectLink';
import { enumLabel, formatShortDate, provenanceVerdict, severityLabel, severityTone } from './labels';

const DOCUMENT_CONTEXT = 600;
const RELATED_FINDINGS_LIMIT = 8;

type Target<K extends InspectorTarget['kind']> = Extract<InspectorTarget, { kind: K }>;

export function LayerBody({ target }: { target: InspectorTarget }) {
  switch (target.kind) {
    case 'finding': return <FindingLayer target={target} />;
    case 'entity': return <EntityLayer target={target} />;
    case 'document': return <DocumentLayer target={target} />;
    case 'agency': return <AgencyLayer target={target} />;
    case 'request': return <RequestLayer target={target} />;
    case 'case': return <CaseLayer target={target} />;
  }
}

function FindingLayer({ target }: { target: Target<'finding'> }) {
  const setWorkspaceView = useAppStore((state) => state.setWorkspaceView);
  return (
    <div className="space-y-4">
      <LeadDossier findingId={target.id} />
      <Actions>
        <ActionButton icon="flag" onClick={() => setWorkspaceView('story')}>Open in Story desk</ActionButton>
      </Actions>
    </div>
  );
}

function EntityLayer({ target }: { target: Target<'entity'> }) {
  const openSubjectDossier = useAppStore((state) => state.openSubjectDossier);
  const setWorkspaceView = useAppStore((state) => state.setWorkspaceView);
  const closeInspector = useAppStore((state) => state.closeInspector);
  const inspect = useAppStore((state) => state.inspect);
  const state = useGatedLoad(target.entityId, () => invokeCommand('get_corpus_entity_neighborhood', { entityId: target.entityId, limit: 12 }));
  return (
    <Loaded state={state}>
      {(neighborhood) => neighborhood === null ? (
        <p className="text-muted">This entity is not in the current corpus index. The index may have been rebuilt since the link was made.</p>
      ) : (
        <div className="space-y-4">
          <Facts items={[
            ['Kind', enumLabel(neighborhood.center.kind)],
            ['Weakest validation', enumLabel(neighborhood.center.validationGrade)],
          ]} />
          <NeighbourBars
            neighborhood={neighborhood}
            total={null}
            compact
            storageKey="inspector-entity-neighbours"
            onOpen={(edge) => inspect({ kind: 'entity', entityId: edge.neighbor.entityId, label: edge.neighbor.displayValue }, 'layer')}
          />
          <Actions>
            <ActionButton icon="tag" onClick={() => { openSubjectDossier({ kind: 'entity', value: target.entityId }); setWorkspaceView('overview'); closeInspector(); }}>Open full subject dossier</ActionButton>
          </Actions>
        </div>
      )}
    </Loaded>
  );
}

function DocumentLayer({ target }: { target: Target<'document'> }) {
  const openEvidenceLink = useAppStore((state) => state.openEvidenceLink);
  const setWorkspaceView = useAppStore((state) => state.setWorkspaceView);
  const closeInspector = useAppStore((state) => state.closeInspector);
  const key = `${target.portalKey}:${target.documentId}:${target.characterStart ?? ''}`;
  const start = target.characterStart ?? 0;
  const offset = Math.max(0, start - DOCUMENT_CONTEXT / 2);
  const chunk = useGatedLoad(key, () => invokeCommand('get_document_text_chunk', { documentId: target.documentId, portalKey: target.portalKey, characterOffset: offset, characterLimit: DOCUMENT_CONTEXT * 2 }));
  const history = useGatedLoad(key, () => invokeCommand('list_document_provenance_verifications', { documentId: target.documentId, portalKey: target.portalKey, limit: 1 }));
  const citing = useGatedLoad(key, () => invokeCommand('list_findings', { filter: { portalKey: target.portalKey, documentId: target.documentId }, limit: RELATED_FINDINGS_LIMIT }));
  const [verification, setVerification] = useState<LoadState<DocumentProvenanceVerification> | null>(null);
  const verifyGate = useLatestRequestGate();

  const verify = () => {
    const token = verifyGate.begin();
    setVerification({ status: 'pending' });
    invokeCommand('verify_document_provenance', { documentId: target.documentId, portalKey: target.portalKey })
      .then((data) => { if (verifyGate.isCurrent(token)) setVerification({ status: 'ready', data }); })
      .catch((reason) => { if (verifyGate.isCurrent(token)) setVerification({ status: 'failed', message: formatNativeError(reason) }); });
  };

  const openInViewer = () => {
    if (target.requestId === null) return;
    openEvidenceLink({ portalKey: target.portalKey, requestId: target.requestId, documentId: target.documentId, characterOffset: start });
    setWorkspaceView('evidence');
    closeInspector();
  };

  const total = chunk.status === 'ready' ? chunk.data.totalCharacterCount : null;
  const marks: MinimapMark[] = citing.status === 'ready'
    ? citing.data.flatMap((finding) => finding.citations
      .filter((citation) => citation.documentId === target.documentId && citation.portalKey === target.portalKey)
      .map((citation, index) => ({ key: `${finding.id}:${index}`, kind: 'finding' as const, start: citation.characterStart, end: citation.characterEnd, label: `Finding: ${finding.headline}`, quote: citation.quote })))
    : [];

  return (
    <div className="space-y-4">
      {total !== null && chunk.status === 'ready' && (
        <DocumentMinimap
          marks={marks}
          total={total}
          loaded={{ start: chunk.data.characterOffset, end: chunk.data.characterOffset + chunk.data.returnedCharacterCount }}
          focus={target.characterStart === null ? null : { start: target.characterStart, end: Math.max(target.characterStart, target.characterEnd ?? target.characterStart) }}
          storageKey="inspector-document-minimap"
        />
      )}
      <Section title={target.characterStart === null ? 'Opening text' : 'Cited passage in context'}>
        <Loaded state={chunk}>
          {(data) => {
            const markStart = target.characterStart === null ? -1 : start - data.characterOffset;
            const markEnd = markStart + Math.max(0, (target.characterEnd ?? start) - start);
            return (
              <p tabIndex={0} role="region" aria-label="Document text" className="focus-ring max-h-72 overflow-y-auto whitespace-pre-wrap rounded-lg border border-line bg-canvas/50 p-3 text-caption leading-relaxed text-fg-secondary">
                {markStart < 0 ? data.maskedText : <>
                  {data.maskedText.slice(0, markStart)}
                  <mark className="rounded-sm bg-warning/25 px-0.5 text-fg">{data.maskedText.slice(markStart, markEnd)}</mark>
                  {data.maskedText.slice(markEnd)}
                </>}
              </p>
            );
          }}
        </Loaded>
      </Section>
      <Section title="Provenance">
        <Loaded state={history}>
          {(records) => {
            const latest = verification?.status === 'ready' ? null : records[0];
            return (
              <div className="space-y-1.5">
                {verification?.status === 'ready' && <VerificationResult result={verification.data} />}
                {verification?.status === 'failed' && <p className="text-danger">Verification failed: {verification.message}</p>}
                {verification?.status !== 'ready' && (latest
                  ? <p className={verdictClass(provenanceVerdict(latest).tone)}>{provenanceVerdict(latest).text}{latest.isCryptographicallySigned ? '; the manifest claims a signature' : ''}.</p>
                  : <p className="text-muted">This file has never been verified.</p>)}
                <ActionButton icon="shield" onClick={verify} disabled={verification?.status === 'pending'}>
                  {verification?.status === 'pending' ? 'Verifying…' : 'Verify now'}
                </ActionButton>
              </div>
            );
          }}
        </Loaded>
      </Section>
      <Section title="Findings citing this document">
        <Loaded state={citing}>{(findings) => <FindingList findings={findings} empty="No stored finding cites this document." />}</Loaded>
      </Section>
      <Actions>
        <ActionButton icon="evidence" onClick={openInViewer} disabled={target.requestId === null}>Open in document viewer</ActionButton>
        {target.requestId === null && <span className="text-caption text-muted">The citation does not record which request holds this document.</span>}
      </Actions>
    </div>
  );
}

function VerificationResult({ result }: { result: DocumentProvenanceVerification }) {
  const verdict = provenanceVerdict(result);
  return <p className={verdictClass(verdict.tone)}>Checked just now: {verdict.text.charAt(0).toLowerCase() + verdict.text.slice(1)}.</p>;
}

function verdictClass(tone: 'success' | 'danger' | 'muted') {
  return tone === 'success' ? 'text-success' : tone === 'danger' ? 'text-danger' : 'text-muted';
}

function AgencyLayer({ target }: { target: Target<'agency'> }) {
  const profiles = useAppStore((state) => state.agencyProfiles);
  const profile = profiles.find((entry) => entry.agencyId === target.agencyId && entry.portalSubdomain === target.portalKey) ?? null;
  const inspect = useAppStore((state) => state.inspect);
  const setActivePortal = useAppStore((state) => state.setActivePortal);
  const setSearchFilters = useAppStore((state) => state.setSearchFilters);
  const setWorkspaceView = useAppStore((state) => state.setWorkspaceView);
  const closeInspector = useAppStore((state) => state.closeInspector);
  const findings = useGatedLoad(`${target.portalKey}:${target.agencyId}`, () => invokeCommand('list_findings', { filter: { portalKey: target.portalKey, agencyId: target.agencyId }, limit: RELATED_FINDINGS_LIMIT }));

  const searchRecords = () => {
    setActivePortal(target.portalKey);
    setSearchFilters('', target.agencyId, null);
    setWorkspaceView('evidence');
    closeInspector();
  };

  return (
    <div className="space-y-4">
      {profile ? (
        <>
          <RequestAccountingBar profile={profile} onSearchRecords={searchRecords} />
          <PeerStrip
            model={peerStripModel(profile, profiles)}
            onOpenPeer={(peer) => inspect({ kind: 'agency', portalKey: peer.portalSubdomain, agencyId: peer.agencyId, label: peer.agencyName }, 'layer')}
          />
        </>
      ) : (
        <p className="text-muted">No analytics profile has been computed for this agency.</p>
      )}
      <Section title="Findings about this agency">
        <Loaded state={findings}>{(items) => (
          <>
            {items.length > 0 && <p className="mb-1.5 text-caption tabular-nums text-muted">{severityCounts(items)}</p>}
            <FindingList findings={items} empty="No stored finding names this agency." />
          </>
        )}</Loaded>
      </Section>
      <Actions>
        <ActionButton icon="search" onClick={searchRecords}>Search its records</ActionButton>
        <ActionButton icon="bars" onClick={() => { setWorkspaceView('compare'); closeInspector(); }}>Compare with peers</ActionButton>
      </Actions>
    </div>
  );
}

function RequestLayer({ target }: { target: Target<'request'> }) {
  const setActivePortal = useAppStore((state) => state.setActivePortal);
  const setSelectedRequest = useAppStore((state) => state.setSelectedRequest);
  const setWorkspaceView = useAppStore((state) => state.setWorkspaceView);
  const closeInspector = useAppStore((state) => state.closeInspector);
  const key = `${target.portalKey}:${target.requestId}`;
  const documents = useGatedLoad(key, () => invokeCommand('list_request_documents', { requestId: target.requestId, portalKey: target.portalKey }));
  const findings = useGatedLoad(key, () => invokeCommand('list_findings', { filter: { portalKey: target.portalKey, requestId: target.requestId }, limit: RELATED_FINDINGS_LIMIT }));
  const record = useAppStore((state) => state.filteredResults.find((result) => result.requestId === target.requestId && result.portalKey === target.portalKey) ?? null);
  const clock = useGatedLoad(key, () => loadClockInputs(target.portalKey, target.requestId));

  const openInEvidence = () => {
    setActivePortal(target.portalKey);
    setSelectedRequest(target.requestId, target.portalKey);
    setWorkspaceView('evidence');
    closeInspector();
  };

  return (
    <div className="space-y-4">
      <ClockStrip
        data={portalClock({
          // The list row when this request is in the current results; otherwise the dates the engine read evaluated.
          record: record ?? (clock.status === 'ready' ? recordFromEngine(clock.data.engine) : null),
          extension: clock.status === 'ready' ? clock.data.extension : null,
          engine: clock.status === 'ready' ? clock.data.engine : null,
          findings: clock.status === 'ready' ? clock.data.findings : [],
          observedAt: new Date().toISOString(),
        })}
        status={clock.status === 'pending' ? 'loading' : 'ready'}
        storageKey="inspector-request-clock"
      />
      <Section title="Documents">
        <Loaded state={documents}>
          {(items) => items.length === 0 ? <p className="text-muted">No document is stored for this request.</p> : (
            <ul className="motion-stagger space-y-1.5">
              {items.map((item) => (
                <li key={item.documentId} className="flex items-baseline justify-between gap-3">
                  <ObjectLink target={{ kind: 'document', portalKey: item.portalKey, documentId: item.documentId, requestId: target.requestId, characterStart: null, characterEnd: null, label: item.fileName }}>{item.fileName}</ObjectLink>
                  <span className="shrink-0 text-caption text-muted">{enumLabel(item.extractionStatus)}</span>
                </li>
              ))}
            </ul>
          )}
        </Loaded>
      </Section>
      <Section title="Findings on this request">
        <Loaded state={findings}>{(items) => <FindingList findings={items} empty="No stored finding names this request." />}</Loaded>
      </Section>
      <Actions>
        <ActionButton icon="evidence" onClick={openInEvidence}>Open in Evidence</ActionButton>
      </Actions>
    </div>
  );
}

function CaseLayer({ target }: { target: Target<'case'> }) {
  const activeCaseId = useAppStore((state) => state.activeCaseId);
  const setActiveCase = useAppStore((state) => state.setActiveCase);
  const openCasework = useAppStore((state) => state.openCasework);
  const state = useGatedLoad(target.caseId, () => invokeCommand('get_casework_state', {}));
  return (
    <Loaded state={state}>
      {(casework) => {
        const found = casework.cases.find((entry) => entry.id === target.caseId);
        if (!found) return <p className="text-muted">This case is no longer stored. It may have been removed or replaced by an import.</p>;
        const grades = gradeSegments(found.evidence);
        return (
          <div className="space-y-4">
            <Facts items={[
              ['Topic', found.topic || 'Not set'],
              ['Disposition', enumLabel(found.disposition)],
              ['Questions', found.questions.length.toLocaleString()],
              ['Revision', `${found.revision} · ${formatShortDate(found.updatedAt)}`],
            ]} />
            <MiniClocks found={found} onOpen={(requestId) => { openCasework(found.reportId); revealCaseworkRequest(requestId); }} />
            <Section title={`Cited sources (${found.evidence.length.toLocaleString()})`}>
              {found.evidence.length > 0 && (
                <Meter
                  className="mb-3"
                  label="Cited sources by verification grade"
                  max={found.evidence.length}
                  segments={grades}
                  valueText={`${found.evidence.length} sources cited: ${grades.map((segment) => segment.text).join(', ')}`}
                />
              )}
              {found.evidence.length === 0 ? <p className="text-muted">No source is cited in this case yet.</p> : (
                <ul className="motion-stagger space-y-2">
                  {found.evidence.map((citation) => (
                    <li key={citation.id}>
                      {citation.portalKey && citation.documentId !== null
                        ? <ObjectLink target={{ kind: 'document', portalKey: citation.portalKey, documentId: citation.documentId, requestId: null, characterStart: citation.characterOffset, characterEnd: null, label: citation.label }}>{citation.label}</ObjectLink>
                        : <span className="text-fg-secondary">{citation.label}</span>}
                      {citation.quote && <p className="mt-0.5 line-clamp-2 text-caption text-muted">“{citation.quote}”</p>}
                    </li>
                  ))}
                </ul>
              )}
            </Section>
            <Actions>
              <ActionButton icon="briefcase" onClick={() => openCasework(found.reportId)}>Open casework</ActionButton>
              {activeCaseId === found.id
                ? <ActionButton icon="x" onClick={() => setActiveCase(null)}>Stop working this case</ActionButton>
                : <ActionButton icon="briefcase" onClick={() => setActiveCase(found.id)}>Work this case</ActionButton>}
            </Actions>
          </div>
        );
      }}
    </Loaded>
  );
}

interface ClockInputs {
  extension: StatutoryExtensionRecord | null;
  findings: LegalComplianceFinding[] | null;
  engine: PortalRequestDeadline | null;
}

/** A portal request's clock inputs. A failed or missing engine read leaves the recorded dates only. */
async function loadClockInputs(portalKey: string, requestId: number): Promise<ClockInputs> {
  const args = { portalKey, requestId };
  const [extension, findings, engine] = await Promise.allSettled([
    invokeCommand('get_request_statutory_extension', args),
    invokeCommand('list_request_legal_findings', args),
    invokeCommand('get_portal_request_deadline', args),
  ]);
  const analysis = engine.status === 'fulfilled' ? engine.value : null;
  return {
    extension: extension.status === 'fulfilled' ? extension.value : null,
    findings: findings.status === 'fulfilled' && Array.isArray(findings.value) ? findings.value : null,
    engine: analysis && typeof analysis === 'object' && typeof analysis.effectiveDeadlineAt === 'string' ? analysis : null,
  };
}

/** Frames to wait for the casework panel to mount the request before giving up. */
const REVEAL_FRAMES = 60;

/** Once casework has rendered, opens the requests section and scrolls the request into view. */
function revealCaseworkRequest(requestId: string, frames = REVEAL_FRAMES) {
  const article = document.getElementById(`casework-request-${requestId}`);
  if (!article) {
    if (frames > 0) requestAnimationFrame(() => revealCaseworkRequest(requestId, frames - 1));
    return;
  }
  const section = article.closest('details');
  if (section) section.open = true;
  article.scrollIntoView?.({ block: 'start' });
}

/** Sources by verification grade, strongest first, on the ordinal ramp. */
function gradeSegments(evidence: readonly CaseworkCitation[]) {
  const current = evidence.filter((cite) => cite.verification === 'SAVED_REPORT' || cite.verification === 'VERIFIED').length;
  const historical = evidence.filter((cite) => cite.verification === 'HISTORICAL_REPORT').length;
  const unverified = evidence.length - current - historical;
  return [
    { key: 'current', value: current, step: 4 as const, text: current === 1 ? '1 matches its current source' : `${current} match their current sources` },
    { key: 'historical', value: historical, step: 2 as const, text: historical === 1 ? '1 source has a newer version' : `${historical} sources have a newer version` },
    { key: 'unverified', value: unverified, step: 1 as const, text: `${unverified} unverified` },
  ].filter((segment) => segment.value > 0);
}

function severityCounts(findings: readonly Finding[]): string {
  const order: Finding['severity'][] = ['CRITICAL', 'SERIOUS', 'NOTABLE', 'INFO'];
  return order
    .map((severity) => [severity, findings.filter((finding) => finding.severity === severity).length] as const)
    .filter(([, n]) => n > 0)
    .map(([severity, n]) => `${n} ${severityLabel(severity).toLowerCase()}`)
    .join(' · ');
}

function FindingList({ findings, empty }: { findings: Finding[]; empty: string }) {
  if (findings.length === 0) return <p className="text-muted">{empty}</p>;
  return (
    <ul className="motion-stagger space-y-2">
      {findings.map((finding) => {
        const tone = severityTone(finding.severity);
        return (
          <li key={finding.id} className="flex items-start gap-2">
            <span className={cn('mt-0.5 shrink-0 rounded-full px-1.5 text-caption font-semibold', tone === 'danger' && 'bg-danger/10 text-danger', tone === 'warning' && 'bg-warning/10 text-warning', tone === 'neutral' && 'bg-raised text-fg-secondary', tone === 'muted' && 'bg-surface text-muted')}>
              {severityLabel(finding.severity)}
            </span>
            <ObjectLink target={{ kind: 'finding', id: finding.id, label: finding.headline }}>{finding.headline}</ObjectLink>
          </li>
        );
      })}
    </ul>
  );
}

function Loaded<T>({ state, children }: { state: LoadState<T>; children: (data: T) => ReactNode }) {
  if (state.status === 'pending') {
    return (
      <div role="status" aria-label="Loading" className="space-y-2">
        <div className="skeleton h-3 w-2/3 rounded" />
        <div className="skeleton h-3 w-full rounded" />
        <div className="skeleton h-3 w-1/2 rounded" />
      </div>
    );
  }
  if (state.status === 'failed') return <p role="alert" className="text-danger">Could not be loaded: {state.message}</p>;
  return <>{children(state.data)}</>;
}

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="motion-reveal">
      <h3 className="text-caption font-semibold uppercase tracking-wider text-accent">{title}</h3>
      <div className="mt-1.5 text-label">{children}</div>
    </section>
  );
}

function Facts({ items }: { items: [string, string][] }) {
  return (
    <dl className="grid grid-cols-2 gap-2">
      {items.map(([label, value]) => (
        <div key={label} className="rounded-lg border border-line bg-surface/50 px-3 py-2">
          <dt className="text-caption text-muted">{label}</dt>
          <dd className="numeric-value mt-0.5 text-label font-semibold text-fg">{value}</dd>
        </div>
      ))}
    </dl>
  );
}

function Actions({ children }: { children: ReactNode }) {
  return <div className="flex flex-wrap items-center gap-2 border-t border-line pt-3">{children}</div>;
}

function ActionButton({ icon, onClick, disabled, children }: { icon: Parameters<typeof Icon>[0]['name']; onClick: () => void; disabled?: boolean; children: ReactNode }) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      className="motion-press focus-ring flex items-center gap-1.5 rounded-lg border border-control bg-surface/70 px-2.5 py-1.5 text-label font-semibold text-fg-secondary transition hover:border-accent/40 hover:text-fg disabled:opacity-50"
    >
      <Icon name={icon} className="h-3.5 w-3.5" />
      {children}
    </button>
  );
}
