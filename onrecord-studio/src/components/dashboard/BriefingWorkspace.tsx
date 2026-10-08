import { useEffect, useMemo, useRef, useState } from 'react';
import { useAppStore } from '../../store/useAppStore';
import {
  buildBriefingMarkdown,
  type BriefingEvidence,
  type BriefingEvidenceReadiness,
} from '../../utils/briefing';
import { cn } from '../../utils/cn';
import { focusSourceComposer } from '../../utils/sanDiego';
import Icon from '../ui/icons';
import Badge from '../ui/Badge';
import Button from '../ui/Button';
import PageHeader from '../ui/PageHeader';
import { INVESTIGATION_TOPICS } from '../../utils/investigations';
import { formatNativeError, invokeCommand } from '../../ipc';
import type { Finding } from '../../types';
import { downloadFile } from './downloadFile';

const AUTO_DRAFT_LABEL_PREFIX = '[Auto-drafted from finding]';
const AUTO_DRAFT_ID_PREFIX = 'auto-draft:';

interface ResearchLane {
  title: string;
  agendaItem: string;
  purpose: string;
  questions: string[];
  searches: string[];
}

interface StarterSource {
  label: string;
  url: string;
  purpose: string;
  tier: 'Meeting record' | 'Local policy' | 'Independent research' | 'Governing law';
}

const STARTER_SOURCES: StarterSource[] = [
  { label: 'Current Council agendas and packets', url: 'https://www.sandiego.gov/city-clerk/officialdocs/legisdocs/brown-act', purpose: 'Confirm the meeting date, item, staff recommendation, and participation instructions in the posted agenda.', tier: 'Meeting record' },
  { label: 'Homelessness funding and program reports', url: 'https://www.sandiego.gov/homelessness-strategies-and-solutions/data-reports', purpose: 'Connect program commitments with reported services and housing outcomes for the same period.', tier: 'Local policy' },
  { label: 'Independent Budget Analyst reports', url: 'https://www.sandiego.gov/iba/reports', purpose: 'Check funding assumptions, recurring costs, alternatives, and the fiscal implications of Council decisions.', tier: 'Independent research' },
  { label: 'City Auditor reports', url: 'https://www.sandiego.gov/auditor/reports', purpose: 'Follow a specific audit recommendation through management responses and evidence of implementation.', tier: 'Independent research' },
];

const SEARCHES_PER_LANE = 4;

const RESEARCH_LANES: ResearchLane[] = INVESTIGATION_TOPICS.map((topic) => ({
  title: topic.title,
  agendaItem: 'Match to a verified agenda item',
  purpose: topic.method,
  questions: [topic.question, ...topic.records.map((record) => `What do the available records establish about: ${record}`), topic.limitation],
  searches: topic.searches,
}));

/** Each lane's first few searches, with a term already offered by an earlier lane left out. */
function distinctLaneSearches(lanes: ResearchLane[]): Map<string, string[]> {
  const offered = new Set<string>();
  const result = new Map<string, string[]>();
  for (const lane of lanes) {
    const shown: string[] = [];
    for (const search of lane.searches) {
      const key = search.trim().toLocaleLowerCase();
      if (shown.length === SEARCHES_PER_LANE || offered.has(key)) continue;
      offered.add(key);
      shown.push(search);
    }
    result.set(lane.title, shown);
  }
  return result;
}

const laneSearches = distinctLaneSearches(RESEARCH_LANES);

export default function BriefingWorkspace() {
  const project = useAppStore((state) => state.briefingProject);
  const portals = useAppStore((state) => state.portals);
  const updateProject = useAppStore((state) => state.updateBriefingProject);
  const updateEvidence = useAppStore((state) => state.updateBriefingEvidence);
  const removeEvidence = useAppStore((state) => state.removeBriefingEvidence);
  const setWorkspaceView = useAppStore((state) => state.setWorkspaceView);
  const setSearchFilters = useAppStore((state) => state.setSearchFilters);
  const setActivePortal = useAppStore((state) => state.setActivePortal);
  const setSelectedRequest = useAppStore((state) => state.setSelectedRequest);
  const setPortalUrlDraft = useAppStore((state) => state.setPortalUrlDraft);
  const addBriefingEvidence = useAppStore((state) => state.addBriefingEvidence);
  const [exported, setExported] = useState(false);
  const autoDraftedRef = useRef(false);
  const [autoDraftError, setAutoDraftError] = useState<string | null>(null);
  // Seeded from the same condition the effect below uses to decide whether it will fetch, so
  // the first paint already reflects reality instead of flashing "no findings" for one frame
  // before the effect flips it to "drafting".
  const [isAutoDrafting, setIsAutoDrafting] = useState(
    () => !project.autoDrafted && project.evidence.length === 0,
  );

  // When a user opens the briefing workspace and nothing is saved, populate a draft from the top
  // findings the detectors have surfaced. The promotion checklist still requires an operator's
  // judgment to publish.
  //
  // `project.autoDrafted` is the source of truth for "has this run before"; it is persisted on
  // the project itself, not component state, because deleting every drafted item drives
  // `project.evidence.length` back to 0 and a remount must not read that as "never drafted" and
  // resurrect what the user removed. `autoDraftedRef` only dedupes re-runs within one mount.
  // The flag is only set on a definitive outcome — the already-had-evidence skip, or a completed
  // fetch — never in the `.catch()`: a transient IPC failure must still get a retry on the next
  // mount rather than permanently burning the one chance to auto-draft.
  useEffect(() => {
    if (autoDraftedRef.current) return;
    autoDraftedRef.current = true;
    if (project.autoDrafted) return;
    if (project.evidence.length > 0) {
      updateProject({ autoDrafted: true });
      return;
    }
    draftFromFindings();
    // draftFromFindings closes over the store setters only, all stable.
  }, [addBriefingEvidence, project.autoDrafted, project.evidence.length, updateProject]);

  function draftFromFindings() {
    setIsAutoDrafting(true);
    setAutoDraftError(null);
    invokeCommand('list_findings', { filter: null, limit: 20 })
      .then((rows) => {
        const drafted = rows
          .map((row) => findingToBriefingEvidence(row))
          .filter((item): item is BriefingEvidence => item !== null);
        for (const item of drafted) addBriefingEvidence(item);
        updateProject({ autoDrafted: true });
      })
      .catch((reason) => {
        setAutoDraftError(formatNativeError(reason));
      })
      .finally(() => setIsAutoDrafting(false));
  }

  const hasAutoDraftedItems = project.evidence.some((item) => item.id.startsWith(AUTO_DRAFT_ID_PREFIX));

  const checks = useMemo(() => promotionChecks(project), [project]);
  const completedChecks = checks.filter((check) => check.done).length;

  const useAccountabilityStarter = () => updateProject({
    title: 'San Diego spending and services briefing',
    audience: 'San Diego City Council',
    durationMinutes: 3,
    objective: 'Present a documented connection between spending, delivered services, and an unresolved commitment; request a specific, verifiable follow-up.',
  });

  const runSearch = (search: string) => {
    setActivePortal(null);
    setSearchFilters(search, null, null, null);
    setWorkspaceView('evidence');
  };

  const stageSource = (url: string) => {
    setPortalUrlDraft(url);
    window.requestAnimationFrame(focusSourceComposer);
  };

  const reviewEvidence = (item: BriefingEvidence) => {
    setActivePortal(item.portalKey);
    setSearchFilters('', null, null, null);
    setSelectedRequest(item.requestId, item.portalKey);
    setWorkspaceView('evidence');
  };

  const exportOutline = () => {
    downloadFile(`${fileStem(project.title)}-${project.meetingDate || 'briefing'}.md`, buildBriefingMarkdown(project), 'text/markdown;charset=utf-8');
    setExported(true);
  };

  return (
    <main className="min-h-0 flex-1 overflow-y-auto">
      <div className="mx-auto w-full max-w-content space-y-6 px-4 pb-10 pt-6 md:px-6">
        <PageHeader
          title="Prepare a public comment"
          summary="Start with the agenda, save traceable records or masked excerpts, record the limitation beside each claim, and end with one action the body can verify."
          actions={<>
            <Button
              variant="link"
              className="tabular-nums text-muted hover:text-fg"
              onClick={() => document.getElementById('briefing-checks')?.scrollIntoView({ block: 'start' })}
            >
              Checks {completedChecks} of {checks.length}
            </Button>
            <Button onClick={useAccountabilityStarter}>Use template</Button>
            <Button variant="primary" onClick={exportOutline}>Export outline</Button>
          </>}
        />
        {autoDraftError && (
          <div role="alert" className="flex flex-wrap items-center justify-between gap-x-4 gap-y-2 rounded-lg border border-warning/30 bg-warning/10 px-4 py-3">
            <p className="min-w-0 flex-1 text-body text-warning">Findings could not be drafted into this briefing.</p>
            <Button size="sm" disabled={isAutoDrafting} onClick={draftFromFindings}>Retry</Button>
          </div>
        )}
        {isAutoDrafting && <p role="status" className="text-caption text-muted">Drafting from your findings…</p>}

        {exported && <div role="status" className="flex items-start justify-between gap-3 rounded-lg border border-success/30 bg-success/10 px-4 py-2 text-caption text-success"><span>Markdown outline exported. It includes local evidence locators, source URLs, preserved-file hashes when available, and explicit operator-assessment labels.</span><Button size="sm" variant="ghost" iconOnly icon="x" aria-label="Dismiss" className="-my-1 -mr-2 shrink-0" onClick={() => setExported(false)} /></div>}

        <div className="grid grid-cols-1 gap-4 xl:grid-cols-[minmax(0,1.4fr)_minmax(340px,0.7fr)]">
          <div className="space-y-4">
            <section className="app-panel overflow-hidden">
              <SectionHeader step="1" title="Frame the presentation" detail="Keep the claim narrower than the evidence and the requested action concrete." />
              <div className="grid gap-3 p-4 md:grid-cols-2">
                <Field label="Presentation title">
                  <input value={project.title} maxLength={240} onChange={(event) => updateProject({ title: event.target.value })} className="analysis-input w-full" />
                </Field>
                <Field label="Audience">
                  <input value={project.audience} maxLength={240} onChange={(event) => updateProject({ audience: event.target.value })} className="analysis-input w-full" />
                </Field>
                <Field label="Meeting date">
                  <input type="date" value={project.meetingDate} onChange={(event) => updateProject({ meetingDate: event.target.value })} className="analysis-input w-full" />
                </Field>
                <Field label="Speaking time (minutes)">
                  <input type="number" min={1} max={120} value={project.durationMinutes} onChange={(event) => updateProject({ durationMinutes: clamp(Number(event.target.value), 1, 120) })} className="analysis-input w-full" />
                </Field>
                <Field label="Objective" wide>
                  <textarea value={project.objective} maxLength={4_000} rows={2} onChange={(event) => updateProject({ objective: event.target.value })} className="analysis-input h-auto min-h-20 w-full resize-y py-2" />
                </Field>
                <Field label="Opening" help="Name the agenda item and its consequence." wide>
                  <textarea value={project.opening} maxLength={8_000} rows={3} placeholder="Example: Before SDPD deploys a roadside screening tool, the public should be able to see what it measures, how errors are handled, and how its use will be audited." onChange={(event) => updateProject({ opening: event.target.value })} className="analysis-input h-auto min-h-20 w-full resize-y py-2" />
                </Field>
                <Field label="Requested action" help="A decision, who owns it, and by when." wide>
                  <textarea value={project.requestedAction} maxLength={8_000} rows={3} placeholder="Example: Place the written operating policy and validation protocol on the next Policy Committee agenda, with public reporting requirements and a named follow-up date." onChange={(event) => updateProject({ requestedAction: event.target.value })} className="analysis-input h-auto min-h-20 w-full resize-y py-2" />
                </Field>
              </div>
            </section>

            <section className="app-panel overflow-hidden">
              <SectionHeader step="2" title="Interrogate the record" detail="Use these research prompts when relevant to your topic. Confirm the date, agenda items, location, and speaking time in the posted meeting record." />
              <div className="grid gap-3 p-4 lg:grid-cols-3">
                {RESEARCH_LANES.map((lane) => (
                  <article key={lane.title} className="rounded-xl border border-line bg-canvas/40 p-3.5">
                    <div className="text-caption text-muted">{lane.agendaItem}</div>
                    <h3 className="mt-1 text-heading text-fg">{lane.title}</h3>
                    <p className="mt-1.5 text-caption leading-relaxed text-muted">{lane.purpose}</p>
                    <ul className="mt-3 space-y-2 text-caption leading-relaxed text-muted">
                      {lane.questions.map((question) => <li key={question} className="flex gap-2"><span className="mt-1 h-1 w-1 shrink-0 rounded-full bg-accent" /> <span>{question}</span></li>)}
                    </ul>
                    <div role="group" aria-label="Search evidence for" className="mt-3 flex flex-wrap gap-1.5">
                      {laneSearches.get(lane.title)?.map((search) => <button key={search} type="button" onClick={() => runSearch(search)} className="focus-ring h-6 rounded-full border border-line px-2.5 text-caption text-fg-secondary transition-colors hover:bg-raised hover:text-fg">{search}</button>)}
                    </div>
                  </article>
                ))}
              </div>
            </section>

            <section className="app-panel overflow-hidden">
              <SectionHeader step="3" title="Build the evidence board" detail="Save leads from Evidence, open the underlying document, then promote only what you reviewed." />
              {project.evidence.length === 0 ? (
                <div className="m-4 grid min-h-48 place-items-center rounded-xl border border-dashed border-line bg-canvas/40 p-6 text-center">
                  <div className="max-w-md">
                    <Icon name="evidence" className="mx-auto h-6 w-6 text-muted" />
                    <h3 className="mt-3 text-label font-semibold text-fg-secondary">
                      {autoDraftError
                        ? 'Auto-draft unavailable — start manually.'
                        : isAutoDrafting
                          ? 'Drafting from detector findings…'
                          : 'No findings drafted — start manually.'}
                    </h3>
                    <p className="mt-1.5 text-caption leading-relaxed text-muted">
                      {autoDraftError
                        ? `We couldn't populate a starting draft (${autoDraftError}). Run a focused search, select a request, and choose "Save lead to briefing" — or open Evidence and add manually.`
                        : isAutoDrafting
                          ? 'The workspace populates with the most notable findings from your data. You review and promote what belongs in the briefing.'
                          : 'The workspace found no detector findings to draft, or the drafted items were removed. Run a focused search, select a request, and choose "Save lead to briefing" — or open Evidence and add manually.'}
                    </p>
                    <Button size="sm" className="mt-3" onClick={() => setWorkspaceView('evidence')}>Open Evidence</Button>
                  </div>
                </div>
              ) : (
                <div className="space-y-3 p-4">
                  {hasAutoDraftedItems && (
                    <div role="status" className="rounded-lg border border-accent/30 bg-accent/5 px-3 py-2 text-caption text-accent">
                      Auto-drafted from your top findings. Edit, remove, or promote each item before you present.
                    </div>
                  )}
                  {project.evidenceTruncated && (
                    <div role="status" className="rounded-lg border border-warning/30 bg-warning/5 px-3 py-2 text-caption text-warning">
                      This briefing exceeded the 200-item evidence cap; the oldest items were dropped to make room for new ones.
                    </div>
                  )}
                  {project.evidence.map((item, index) => (
                    <EvidenceCard
                      key={item.id}
                      item={item}
                      number={index + 1}
                      onChange={(patch) => updateEvidence(item.id, patch)}
                      onReview={() => reviewEvidence(item)}
                      onRemove={() => removeEvidence(item.id)}
                    />
                  ))}
                </div>
              )}
            </section>
          </div>

          <aside aria-label="Briefing sources" className="space-y-4">
            <section className="app-panel overflow-hidden">
              <SectionHeader title="Primary-source queue" detail="Add a source to preserve and index it locally." />
              <div className="space-y-2 p-3">
                {STARTER_SOURCES.map((source) => {
                  const connected = portals.some(({ registration }) => sameHost(registration.canonicalUrl, source.url));
                  return (
                    <article key={source.url} className="rounded-xl border border-line bg-canvas/40 p-3">
                      <div className="flex items-start justify-between gap-3">
                        <div className="min-w-0">
                          <div className="text-caption text-muted">{source.tier}</div>
                          <h3 className="mt-0.5 text-control text-fg">{source.label}</h3>
                        </div>
                        <Badge tone={connected ? 'success' : 'neutral'} className="shrink-0">{connected ? 'Added' : 'Not added'}</Badge>
                      </div>
                      <p className="mt-2 text-caption leading-relaxed text-muted">{source.purpose}</p>
                      {!connected && <Button size="sm" className="mt-2" onClick={() => stageSource(source.url)}>Add source</Button>}
                    </article>
                  );
                })}
              </div>
            </section>

            <section className="app-panel overflow-hidden">
              <SectionHeader title={`${Math.max(1, Math.round(project.durationMinutes))}-minute run of show`} detail={`Aim for about ${Math.max(1, Math.round(project.durationMinutes)) * 130} spoken words; clarity beats coverage.`} />
              <ol className="space-y-2 p-4 text-caption text-muted">
                <RunStep share={15} label="Opening" detail="Name the agenda item and the concrete consequence." />
                <RunStep share={50} label="Two strongest facts" detail="State the source before the inference; avoid a pile of examples." />
                <RunStep share={15} label="Limitation or counterpoint" detail="Say what the evidence does not establish and answer the strongest objection." />
                <RunStep share={15} label="Requested action" detail="Ask for a decision, responsible owner, public artifact, and date." />
                <RunStep share={5} label="Buffer" detail="Stop cleanly before the chair ends your time." />
              </ol>
            </section>

            <section id="briefing-checks" className="app-panel scroll-mt-6 overflow-hidden">
              <SectionHeader title="Promotion standard" detail="The app auto-drafts a starting briefing; you still decide what gets promoted to Ready." />
              <ul className="space-y-2 p-4 text-caption leading-relaxed text-muted">
                {checks.map((check) => <Check key={check.label} done={check.done}>{check.label}</Check>)}
              </ul>
            </section>
          </aside>
        </div>
      </div>
    </main>
  );
}

function EvidenceCard({ item, number, onChange, onReview, onRemove }: {
  item: BriefingEvidence;
  number: number;
  onChange: (patch: Pick<Partial<BriefingEvidence>, 'claim' | 'note' | 'readiness'>) => void;
  onReview: () => void;
  onRemove: () => void;
}) {
  return (
    <article className="rounded-xl border border-line bg-canvas/40 p-3.5">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0 flex-1">
          <div className="text-caption text-muted">Point {number} · {item.kind === 'DOCUMENT_EXCERPT' ? 'Document excerpt' : 'Request lead'}</div>
          <h3 className="mt-0.5 truncate text-control text-fg" title={item.label}>{item.label}</h3>
          <div className="mt-1 font-mono text-caption text-muted">{item.portalKey} / request {item.requestId}{item.documentId === null ? '' : ` / document ${item.documentId}`}{item.documentId !== null && item.characterOffset !== null ? ` / character ${item.characterOffset}` : ''}</div>
        </div>
        <div className="flex items-center gap-2">
          <select value={item.readiness} onChange={(event) => onChange({ readiness: event.target.value as BriefingEvidenceReadiness })} aria-label={`Evidence status for ${item.label}`} className="focus-ring h-8 rounded-lg border border-control bg-canvas px-2 text-caption font-semibold text-fg-secondary">
            <option value="LEAD">Saved lead</option>
            <option value="REVIEWED">Source reviewed</option>
            <option value="READY">Presentation ready</option>
          </select>
          <Button size="sm" onClick={onReview}>Review</Button>
          <button type="button" onClick={onRemove} aria-label={`Remove ${item.label} from briefing`} className="focus-ring grid h-8 w-8 place-items-center rounded-lg border border-control text-muted hover:border-danger/30 hover:text-danger"><Icon name="trash" className="h-3 w-3" /></button>
        </div>
      </div>
      {item.excerpt && <blockquote className="mt-3 max-h-28 overflow-y-auto rounded-lg border border-line bg-canvas/70 p-3 text-caption leading-relaxed text-muted">{item.excerpt}</blockquote>}
      {item.rawFileSha256 && <div className="mt-2 break-all font-mono text-caption text-muted">Preserved SHA-256 {item.rawFileSha256}</div>}
      <div className="mt-3 grid gap-3 md:grid-cols-2">
        <Field label="Claim this supports">
          <textarea value={item.claim} maxLength={4_000} rows={3} placeholder="State only what this record establishes." onChange={(event) => onChange({ claim: event.target.value })} className="analysis-input h-auto min-h-20 w-full resize-y py-2" />
        </Field>
        <Field label="Context, limitation, or counterpoint">
          <textarea value={item.note} maxLength={4_000} rows={3} placeholder="What does it not establish? What source would corroborate it?" onChange={(event) => onChange({ note: event.target.value })} className="analysis-input h-auto min-h-20 w-full resize-y py-2" />
        </Field>
      </div>
    </article>
  );
}

function SectionHeader({ step, title, detail }: { step?: string; title: string; detail: string }) {
  return (
    <div className="flex items-start gap-3 border-b border-line px-5 py-4">
      {step && <span aria-hidden="true" className="grid h-7 w-7 shrink-0 place-items-center rounded-lg bg-raised text-caption font-medium tabular-nums text-fg-secondary">{step}</span>}
      <div className="min-w-0">
        <h2 className="text-heading text-fg">{step && <span className="sr-only">Step {step}: </span>}{title}</h2>
        <p className="mt-1 text-caption text-muted">{detail}</p>
      </div>
    </div>
  );
}

function Field({ label, help, wide = false, children }: { label: string; help?: string; wide?: boolean; children: React.ReactNode }) {
  return (
    <label className={cn('block text-control text-fg-secondary', wide && 'md:col-span-2')}>
      {label}
      {help && <span className="mt-0.5 block text-caption font-normal text-muted">{help}</span>}
      <span className="mt-1.5 block">{children}</span>
    </label>
  );
}

function RunStep({ share, label, detail }: { share: number; label: string; detail: string }) {
  return <li className="grid grid-cols-[38px_minmax(0,1fr)] gap-2"><span className="font-medium tabular-nums text-fg-secondary">{share}%</span><span><strong className="font-semibold text-fg-secondary">{label}.</strong> {detail}</span></li>;
}

function Check({ done, children }: { done: boolean; children: React.ReactNode }) {
  return <li className="flex gap-2"><span aria-hidden="true" className={cn('mt-0.5 grid h-3.5 w-3.5 shrink-0 place-items-center rounded-full border text-caption', done ? 'border-success/30 bg-success/10 text-success' : 'border-line-strong text-muted')}>{done ? '✓' : ''}</span><span>{children}<span className="sr-only">{done ? ' (done)' : ' (not done)'}</span></span></li>;
}

/** The single checklist behind both the header count and the promotion standard panel. */
function promotionChecks(project: { objective: string; requestedAction: string; evidence: BriefingEvidence[] }) {
  const { evidence } = project;
  return [
    { label: 'The objective states what the presentation is for.', done: Boolean(project.objective.trim()) },
    { label: 'At least one traceable request or excerpt is saved.', done: evidence.length > 0 },
    { label: 'At least one claim reaches the underlying document, not only request metadata.', done: evidence.some((item) => item.kind === 'DOCUMENT_EXCERPT') },
    { label: 'A reviewed item is explicitly marked presentation ready.', done: evidence.some((item) => item.readiness === 'READY') },
    { label: 'A ready item records its context, limitation, or counterpoint.', done: evidence.some((item) => item.readiness === 'READY' && Boolean(item.note.trim())) },
    { label: 'The close asks for a measurable action.', done: Boolean(project.requestedAction.trim()) },
  ];
}

function sameHost(left: string, right: string) {
  try {
    return new URL(left).hostname.toLocaleLowerCase() === new URL(right).hostname.toLocaleLowerCase();
  } catch {
    return false;
  }
}

function fileStem(value: string) {
  return value.trim().toLocaleLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '').slice(0, 80) || 'onrecord-briefing';
}

function clamp(value: number, minimum: number, maximum: number) {
  return Number.isFinite(value) ? Math.min(maximum, Math.max(minimum, value)) : minimum;
}

/**
 * Turns a detector finding into a starter briefing entry. Only findings that resolve to a
 * concrete request (portalKey + requestId) become drafted evidence, because the briefing's
 * schema requires those identifiers to render local locators. A finding that has neither in
 * its subject or citations is dropped rather than silently attached to a placeholder request.
 */
export function findingToBriefingEvidence(finding: Finding): BriefingEvidence | null {
  const cite = finding.citations[0] ?? null;
  const portalKey = finding.subject.portalKey ?? cite?.portalKey ?? null;
  const requestId = finding.subject.requestId;
  // Requests are the anchor a briefing renders. Citations only carry documentIds, not requestIds,
  // so if the finding's subject has no requestId we keep it out of the auto-draft rather than
  // fabricating an anchor.
  if (!portalKey || requestId === null) return null;
  const documentId = finding.subject.documentId ?? cite?.documentId ?? null;
  const label = `${AUTO_DRAFT_LABEL_PREFIX} ${finding.headline}`.slice(0, 240);
  const excerpt = cite?.quote ?? finding.explanation;
  return {
    id: `${AUTO_DRAFT_ID_PREFIX}${finding.id}`,
    kind: documentId === null ? 'REQUEST' : 'DOCUMENT_EXCERPT',
    portalKey,
    requestId,
    documentId,
    fileName: null,
    characterOffset: cite?.characterStart ?? null,
    sourceUrl: null,
    agencyName: '',
    label,
    recordedAt: finding.discoveredAt,
    excerpt: excerpt.slice(0, 4_000),
    rawFileSha256: null,
    claim: finding.explanation.slice(0, 4_000),
    note: '',
    readiness: 'LEAD',
    addedAt: finding.discoveredAt,
  };
}
