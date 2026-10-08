import Dialog, { DialogHeader } from '../ui/Dialog';
import InlineError from '../ui/InlineError';
import { Select, Textarea } from '../ui/Field';
import { useEffect, useRef, useState, type ChangeEvent, type FormEvent } from 'react';
import { invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import Button, { buttonClass } from '../ui/Button';
import Skeleton from '../ui/Skeleton';
import { useAppStore } from '../../store/useAppStore';
import { parseIsoDateString } from '../../types';
import type {
  LegalAuthorityImport,
  LegalAuthorityKind,
  LegalMandateInput,
  LegalMandateSeverity,
  LegalAuthorityVersionDetail,
  LegalAuthorityVersionSummary,
} from '../../types';
import { cn } from '../../utils/cn';
import { enumLabel, formatShortDate } from '../objects/labels';
import { displayError } from '../../utils/errorMessage';

const MAXIMUM_SOURCE_BYTES = 8 * 1024 * 1024;

interface LegalAuthorityManagerProps {
  isOpen: boolean;
  onClose: () => void;
}

interface MandateEditorState {
  key: string;
  mandateId: string;
  title: string;
  description: string;
  applicabilityText: string;
  evidenceText: string;
  severity: LegalMandateSeverity;
}

interface AuthorityEditorState {
  statuteId: string;
  jurisdiction: string;
  statuteCitation: string;
  title: string;
  authorityKind: LegalAuthorityKind;
  rawText: string;
  mandatedDeadlineDays: string;
  canonicalSourceUri: string;
  effectiveFrom: string;
  effectiveTo: string;
}

const emptyAuthority = (): AuthorityEditorState => ({
  statuteId: '',
  jurisdiction: '',
  statuteCitation: '',
  title: '',
  authorityKind: 'AGENCY_POLICY',
  rawText: '',
  mandatedDeadlineDays: '',
  canonicalSourceUri: '',
  effectiveFrom: '',
  effectiveTo: '',
});

const emptyMandate = (): MandateEditorState => ({
  key: `${Date.now()}-${Math.random()}`,
  mandateId: '',
  title: '',
  description: '',
  applicabilityText: '',
  evidenceText: '',
  severity: 'HIGH',
});

export default function LegalAuthorityManager({ isOpen, onClose }: LegalAuthorityManagerProps) {
  const authorities = useAppStore((state) => state.legalAuthorities);
  const setAuthorities = useAppStore((state) => state.setLegalAuthorities);
  const [editor, setEditor] = useState<AuthorityEditorState>(emptyAuthority);
  const [mandates, setMandates] = useState<MandateEditorState[]>([]);
  const [sourceFileName, setSourceFileName] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [isImporting, setIsImporting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState<string | null>(null);
  const [selectedStatuteId, setSelectedStatuteId] = useState<string | null>(null);
  const [versions, setVersions] = useState<LegalAuthorityVersionSummary[]>([]);
  const [leftVersionId, setLeftVersionId] = useState<string | null>(null);
  const [rightVersionId, setRightVersionId] = useState<string | null>(null);
  const [leftVersion, setLeftVersion] = useState<LegalAuthorityVersionDetail | null>(null);
  const [rightVersion, setRightVersion] = useState<LegalAuthorityVersionDetail | null>(null);
  const [versionError, setVersionError] = useState<string | null>(null);
  const [versionRevision, setVersionRevision] = useState(0);
  const fileInput = useRef<HTMLInputElement>(null);

  // A failed read is its own state: "none indexed" would be a claim the read never made.
  const [listError, setListError] = useState<string | null>(null);
  const [listRevision, setListRevision] = useState(0);
  const listGate = useLatestRequestGate();
  useEffect(() => {
    if (!isOpen) return;
    const token = listGate.begin();
    setIsLoading(true);
    setListError(null);
    void invokeCommand('list_legal_authorities', {})
      .then((value) => {
        if (listGate.isCurrent(token)) setAuthorities(value);
      })
      .catch((reason) => {
        if (listGate.isCurrent(token)) setListError(displayError(reason).message);
      })
      .finally(() => {
        if (listGate.isCurrent(token)) setIsLoading(false);
      });
    return () => listGate.invalidate();
  }, [isOpen, listGate, listRevision, setAuthorities]);

  useEffect(() => {
    if (!isOpen || authorities.length === 0) return;
    setSelectedStatuteId((current) => current && authorities.some((authority) => authority.statuteId === current) ? current : authorities[0].statuteId);
  }, [authorities, isOpen]);

  useEffect(() => {
    if (!isOpen || !selectedStatuteId) return;
    let active = true;
    setVersionError(null);
    setVersions([]);
    setLeftVersionId(null);
    setRightVersionId(null);
    setLeftVersion(null);
    setRightVersion(null);
    void invokeCommand('list_legal_authority_versions', { statuteId: selectedStatuteId }).then((values) => {
      if (!active) return;
      setVersions(values);
      setRightVersionId(values[0]?.versionId ?? null);
      setLeftVersionId(values[1]?.versionId ?? values[0]?.versionId ?? null);
    }).catch((reason) => {
      if (active) setVersionError(displayError(reason).message);
    });
    return () => { active = false; };
  }, [isOpen, selectedStatuteId, versionRevision]);

  useEffect(() => {
    if (!selectedStatuteId || !leftVersionId || !rightVersionId) return;
    let active = true;
    setVersionError(null);
    void Promise.all([
      invokeCommand('get_legal_authority_version', { statuteId: selectedStatuteId, versionId: leftVersionId }),
      invokeCommand('get_legal_authority_version', { statuteId: selectedStatuteId, versionId: rightVersionId }),
    ]).then(([left, right]) => {
      if (!active) return;
      setLeftVersion(left);
      setRightVersion(right);
    }).catch((reason) => {
      if (active) setVersionError(displayError(reason).message);
    });
    return () => { active = false; };
  }, [leftVersionId, rightVersionId, selectedStatuteId]);

  if (!isOpen) return null;

  const updateEditor = <Key extends keyof AuthorityEditorState>(
    key: Key,
    value: AuthorityEditorState[Key],
  ) => setEditor((current) => ({ ...current, [key]: value }));

  const updateMandate = <Key extends keyof MandateEditorState>(
    index: number,
    key: Key,
    value: MandateEditorState[Key],
  ) => setMandates((current) => current.map((mandate, position) =>
    position === index ? { ...mandate, [key]: value } : mandate,
  ));

  const loadFile = async (event: ChangeEvent<HTMLInputElement>) => {
    const file = event.target.files?.[0];
    if (!file) return;
    setError(null);
    setSuccess(null);
    if (file.size > MAXIMUM_SOURCE_BYTES) {
      setError(`Source files must be no larger than ${MAXIMUM_SOURCE_BYTES / (1024 * 1024)} MiB.`);
      event.target.value = '';
      return;
    }
    try {
      const rawText = await file.text();
      updateEditor('rawText', rawText);
      setSourceFileName(file.name);
      if (!editor.title) updateEditor('title', file.name.split('_').join(' ').split('-').join(' '));
    } catch (reason) {
      setError(`Could not read the selected source: ${displayError(reason).message}`);
    }
  };

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    setError(null);
    setSuccess(null);
    setIsImporting(true);
    try {
      const deadline = editor.mandatedDeadlineDays.trim();
      const validation = validateAuthorityEditor(editor, mandates);
      if (!validation.ok) {
        setError(validation.error);
        return;
      }
      const input: LegalAuthorityImport = {
        statuteId: editor.statuteId.trim(),
        jurisdiction: editor.jurisdiction.trim(),
        statuteCitation: editor.statuteCitation.trim(),
        title: editor.title.trim(),
        authorityKind: editor.authorityKind,
        rawText: editor.rawText,
        mandatedDeadlineDays: deadline ? Number(deadline) : null,
        canonicalSourceUri: nullable(editor.canonicalSourceUri),
        effectiveFrom: validation.effectiveFrom,
        effectiveTo: validation.effectiveTo,
        mandates: mandates.map(toMandateInput),
      };
      const result = await invokeCommand('import_legal_authority', { input });
      const refreshed = await invokeCommand('list_legal_authorities', {});
      setAuthorities(refreshed);
      setSelectedStatuteId(result.authority.statuteId);
      setVersionRevision((value) => value + 1);
      setSuccess(
        result.insertedNewVersion
          ? `${result.authority.statuteCitation} was indexed as a new immutable version.`
          : `${result.authority.statuteCitation} metadata and evidence rules were refreshed.`,
      );
      setEditor(emptyAuthority());
      setMandates([]);
      setSourceFileName(null);
      if (fileInput.current) fileInput.current.value = '';
    } catch (reason) {
      setError(displayError(reason).message);
    } finally {
      setIsImporting(false);
    }
  };

  return (
    <Dialog size="xl" onClose={onClose} className="h-[calc(100vh-4rem)] max-w-[min(1380px,96vw)]">
      <DialogHeader title="Legal authorities" description="Statutes, ordinances and policies the compliance checks read. Each import is an immutable, content-addressed version." />
      <div className="grid min-h-0 flex-1 grid-cols-[300px_minmax(0,1fr)] border-t border-line xl:grid-cols-[340px_minmax(0,1fr)]">
        <aside aria-label="Indexed authorities" className="flex min-h-0 flex-col border-r border-line">
          <div className="min-h-0 flex-1 overflow-y-auto p-3">
            {isLoading ? (
              <div className="space-y-2">
                <Skeleton variant="card" label="Loading indexed authorities" />
                <Skeleton variant="card" label="" className="hidden sm:block" />
              </div>
            ) : listError ? (
              <InlineError
                summary={`Authorities could not be read. This does not mean none are indexed. ${listError}`}
                onRetry={() => setListRevision((value) => value + 1)}
              />
            ) : authorities.length === 0 ? (
              <p className="rounded-lg border border-dashed border-line p-4 text-body text-muted">
                No authorities are indexed yet. Index one with the form beside this list.
              </p>
            ) : (
              <div className="space-y-2">
                {authorities.map((authority) => (
                  <button type="button" key={authority.statuteId} aria-current={selectedStatuteId === authority.statuteId ? 'true' : undefined} onClick={() => setSelectedStatuteId(authority.statuteId)} className={cn('focus-ring block w-full rounded-lg border p-3 text-left transition-colors', selectedStatuteId === authority.statuteId ? 'border-accent bg-accent/5' : 'border-line bg-panel hover:border-line-strong')}>
                    <div className="flex items-start justify-between gap-2">
                      <div className="min-w-0">
                        <div className="truncate text-body font-medium text-fg">
                          {authority.statuteCitation}
                        </div>
                        <div className="mt-1 truncate text-caption text-muted">
                          {authority.jurisdiction}
                        </div>
                      </div>
                      <span className="inline-flex h-5 shrink-0 items-center whitespace-nowrap rounded-full border border-line bg-raised px-2 text-caption font-medium text-fg-secondary">
                        {enumLabel(authority.authorityKind)}
                      </span>
                    </div>
                    <p className="mt-2 line-clamp-2 text-caption leading-relaxed text-muted">
                      {authority.title}
                    </p>
                    <div className="mt-2 flex gap-3 text-caption tabular-nums text-muted">
                      <span>{authority.provisionCount} {authority.provisionCount === 1 ? 'provision' : 'provisions'}</span>
                      <span>{authority.mandateCount} {authority.mandateCount === 1 ? 'evidence rule' : 'evidence rules'}</span>
                    </div>
                  </button>
                ))}
              </div>
            )}
          </div>
        </aside>

        <section className="flex min-h-0 flex-col overflow-y-auto">
          {selectedStatuteId && <AuthorityVersionWorkspace citation={authorities.find((authority) => authority.statuteId === selectedStatuteId)?.statuteCitation ?? selectedStatuteId} versions={versions} leftVersionId={leftVersionId} rightVersionId={rightVersionId} leftVersion={leftVersion?.version.versionId === leftVersionId ? leftVersion : null} rightVersion={rightVersion?.version.versionId === rightVersionId ? rightVersion : null} error={versionError} onLeftChange={setLeftVersionId} onRightChange={setRightVersionId} />}

          <form onSubmit={(event) => void submit(event)} className="p-5" aria-labelledby="index-authority-heading">
            <h3 id="index-authority-heading" className="text-heading text-fg">Index an authority</h3>
            <p className="mb-4 mt-1 text-caption text-muted">
              Markdown headings and legal section labels become independently searchable provisions.
            </p>
            <div className="grid grid-cols-2 gap-3">
              <TextField label="Stable authority ID" value={editor.statuteId} onChange={(value) => updateEditor('statuteId', value)} required />
              <TextField label="Jurisdiction" value={editor.jurisdiction} onChange={(value) => updateEditor('jurisdiction', value)} required />
              <TextField label="Citation" value={editor.statuteCitation} onChange={(value) => updateEditor('statuteCitation', value)} required />
              <TextField label="Title" value={editor.title} onChange={(value) => updateEditor('title', value)} required />
              <Select label="Authority type" value={editor.authorityKind} onChange={(event) => updateEditor('authorityKind', event.target.value as LegalAuthorityKind)} className="focus-ring">
                <option value="STATE_STATUTE">State statute</option>
                <option value="MUNICIPAL_ORDINANCE">Municipal ordinance</option>
                <option value="AGENCY_POLICY">Agency policy</option>
                <option value="PROCEDURE_MANUAL">Procedure manual</option>
                <option value="REGULATION">Regulation</option>
                <option value="OTHER">Other authority</option>
              </Select>
              <TextField label="Calendar-day deadline" value={editor.mandatedDeadlineDays} onChange={(value) => updateEditor('mandatedDeadlineDays', value)} type="number" min="0" max="36500" />
              <TextField label="Official HTTPS source" value={editor.canonicalSourceUri} onChange={(value) => updateEditor('canonicalSourceUri', value)} type="url" />
              <div className="grid grid-cols-2 gap-3">
                <TextField label="Effective from" value={editor.effectiveFrom} onChange={(value) => updateEditor('effectiveFrom', value)} type="date" />
                <TextField label="Effective to" value={editor.effectiveTo} onChange={(value) => updateEditor('effectiveTo', value)} type="date" />
              </div>
            </div>

            <div className="mt-5">
              <div className="flex items-end justify-between gap-3">
                <div>
                  <div className="text-control text-fg-secondary">Legislative or policy text<span aria-hidden="true"> *</span></div>
                  <div className="mt-0.5 text-caption text-muted">UTF-8 Markdown or plain text, up to 8 MiB</div>
                </div>
                <label className={buttonClass({ size: 'sm', className: 'cursor-pointer focus-within:ring-2 focus-within:ring-focus' })}>
                  Choose source file
                  <input ref={fileInput} type="file" accept=".md,.markdown,.txt,text/markdown,text/plain" onChange={(event) => void loadFile(event)} className="sr-only" />
                </label>
              </div>
              {sourceFileName && (
                <div className="mt-2 text-caption tabular-nums text-success">
                  <span className="font-mono">{sourceFileName}</span> · {editor.rawText.length.toLocaleString()} characters loaded
                </div>
              )}
              <Textarea
                value={editor.rawText}
                onChange={(event) => updateEditor('rawText', event.target.value)}
                required
                rows={8}
                aria-label="Legal source text"
                className="focus-ring mt-2 min-h-40 font-mono text-code"
              />
            </div>

            <div className="mt-5">
              <div className="flex items-end justify-between gap-3">
                <div>
                  <div className="text-control text-fg-secondary">Policy evidence rules</div>
                  <p className="mt-0.5 text-caption text-muted">Optional. Phrase sequences are tokenized, stemmed, and matched on word boundaries.</p>
                </div>
                <Button size="sm" onClick={() => setMandates((current) => [...current, emptyMandate()])} className="focus-ring">
                  Add evidence rule
                </Button>
              </div>
              <div className="mt-3 space-y-3">
                {mandates.map((mandate, index) => (
                  <div key={mandate.key} className="rounded-xl border border-line bg-surface/40 p-4">
                    <div className="flex items-center justify-between">
                      <span className="text-overline uppercase text-muted">Evidence rule {index + 1}</span>
                      <Button variant="ghost" size="sm" onClick={() => setMandates((current) => current.filter((_, position) => position !== index))} className="focus-ring text-danger hover:bg-danger/10 hover:text-danger">Remove</Button>
                    </div>
                    <div className="mt-3 grid grid-cols-[1fr_1.5fr_140px] gap-3">
                      <TextField label="Rule ID" value={mandate.mandateId} onChange={(value) => updateMandate(index, 'mandateId', value)} required />
                      <TextField label="Rule title" value={mandate.title} onChange={(value) => updateMandate(index, 'title', value)} required />
                      <Select label="Severity" value={mandate.severity} onChange={(event) => updateMandate(index, 'severity', event.target.value as LegalMandateSeverity)} className="focus-ring">
                        <option value="ADVISORY">Advisory</option>
                        <option value="MODERATE">Moderate</option>
                        <option value="HIGH">High</option>
                        <option value="CRITICAL">Critical</option>
                      </Select>
                    </div>
                    <label className="mt-3 block">
                      <span className="text-control text-fg-secondary">Rule rationale</span>
                      <Textarea value={mandate.description} onChange={(event) => updateMandate(index, 'description', event.target.value)} required rows={2} className="focus-ring mt-1.5" />
                    </label>
                    <div className="mt-3 grid grid-cols-2 gap-3">
                      <PhraseField label="Applicability phrases" value={mandate.applicabilityText} onChange={(value) => updateMandate(index, 'applicabilityText', value)} />
                      <PhraseField label="Required evidence phrases" value={mandate.evidenceText} onChange={(value) => updateMandate(index, 'evidenceText', value)} />
                    </div>
                  </div>
                ))}
              </div>
            </div>

            {(error || success) && (
              <div role={error ? 'alert' : 'status'} className={cn('mt-4 rounded-lg border px-3 py-2 text-caption leading-relaxed', error ? 'border-danger/30 bg-danger/5 text-danger' : 'border-success/30 bg-success/5 text-success')}>
                {error ?? success}
              </div>
            )}
            <div className="mt-5 flex items-center justify-between gap-4 border-t border-line pt-4">
              <p className="max-w-lg text-caption text-muted">
                Imports are content-addressed. Reimporting changed text creates a new immutable version while retaining prior findings and review history.
              </p>
              <Button type="submit" variant="primary" disabled={isImporting} className="focus-ring">
                {isImporting ? 'Parsing and indexing…' : 'Index authority'}
              </Button>
            </div>
          </form>
        </section>
      </div>
    </Dialog>
  );
}

function AuthorityVersionWorkspace({ citation, versions, leftVersionId, rightVersionId, leftVersion, rightVersion, error, onLeftChange, onRightChange }: { citation: string; versions: LegalAuthorityVersionSummary[]; leftVersionId: string | null; rightVersionId: string | null; leftVersion: LegalAuthorityVersionDetail | null; rightVersion: LegalAuthorityVersionDetail | null; error: string | null; onLeftChange: (value: string) => void; onRightChange: (value: string) => void }) {
  // A comparison needs two versions; with one there is nothing to count, not zero changes.
  const changes = versions.length > 1 && leftVersion && rightVersion ? compareAuthorityText(leftVersion.fullText, rightVersion.fullText) : null;
  return (
    <section className="border-b border-line bg-raised/40 px-5 py-4" aria-label="Authority version comparison">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <h3 className="truncate text-heading text-fg">{citation}</h3>
          <div className="mt-0.5 text-caption text-fg-secondary">
            Effective-date timeline and immutable text comparison
          </div>
          <div className="mt-0.5 text-caption text-muted">
            {versions.length} content-addressed version{versions.length === 1 ? '' : 's'}, newest first
          </div>
        </div>
        {changes && (
          <span className="text-caption tabular-nums text-fg-secondary">
            {changes.changed.toLocaleString()} changed · {changes.added.toLocaleString()} added ·{' '}
            {changes.removed.toLocaleString()} removed lines
          </span>
        )}
      </div>
      {error ? (
        <p role="alert" className="mt-2 text-caption text-danger">
          {error}
        </p>
      ) : versions.length === 0 ? (
        <p className="mt-2 text-caption text-muted">No immutable versions are indexed for this authority.</p>
      ) : versions.length === 1 ? (
        <p className="mt-2 text-caption text-muted">One version indexed: {versionRange(versions[0])}.</p>
      ) : (
        <>
          <div className="mt-2 flex gap-1 overflow-x-auto pb-1">
            {versions.map((version) => (
              <span
                key={version.versionId}
                className="shrink-0 rounded-md border border-line bg-canvas px-2 py-1 text-caption tabular-nums text-muted"
                title={version.contentSha256}
              >
                {versionRange(version)} · indexed {formatShortDate(version.importedAt)}
              </span>
            ))}
          </div>
          <div className="mt-2 grid grid-cols-2 gap-2">
            {(
              [
                ['Earlier / baseline', leftVersionId, onLeftChange, leftVersion],
                ['Later / comparison', rightVersionId, onRightChange, rightVersion],
              ] as const
            ).map(([label, selected, change, detail]) => (
              <label key={label} className="block text-caption text-muted">
                {label}
                <select
                  value={selected ?? ''}
                  onChange={(event) => change(event.target.value)}
                  className="focus-ring ml-2 h-7 rounded-md border border-control bg-canvas px-2 text-caption tabular-nums text-fg-secondary"
                >
                  {versions.map((version) => (
                    <option key={version.versionId} value={version.versionId}>
                      {version.effectiveFrom ? `Effective ${formatShortDate(version.effectiveFrom)}` : `Indexed ${formatShortDate(version.importedAt)}`}
                    </option>
                  ))}
                </select>
                <textarea
                  readOnly
                  value={detail?.fullText ?? 'Loading version text…'}
                  aria-label={`${label} authority text`}
                  className="mt-1.5 h-28 w-full resize-y rounded border border-control bg-canvas p-2 font-mono text-caption leading-relaxed text-muted"
                />
              </label>
            ))}
          </div>
        </>
      )}
    </section>
  );
}

function versionRange(version: { effectiveFrom: string | null; effectiveTo: string | null }): string {
  const from = version.effectiveFrom ? `Effective ${formatShortDate(version.effectiveFrom)}` : 'Effective date not recorded';
  return `${from} – ${version.effectiveTo ? `until ${formatShortDate(version.effectiveTo)}` : 'no end date'}`;
}

function compareAuthorityText(left: string, right: string) {
  const leftLines = left.split('\n');
  const rightLines = right.split('\n');
  const shared = Math.min(leftLines.length, rightLines.length);
  let changed = 0;
  for (let index = 0; index < shared; index += 1) if (leftLines[index] !== rightLines[index]) changed += 1;
  return { changed, added: Math.max(0, rightLines.length - leftLines.length), removed: Math.max(0, leftLines.length - rightLines.length) };
}

function validateAuthorityEditor(
  editor: AuthorityEditorState,
  mandates: MandateEditorState[],
): { ok: true; effectiveFrom: ReturnType<typeof parseIsoDateString>; effectiveTo: ReturnType<typeof parseIsoDateString> }
  | { ok: false; error: string } {
  if (!/^[A-Za-z0-9._:-]{1,128}$/.test(editor.statuteId.trim())) {
    return { ok: false, error: 'Authority ID must use 1–128 letters, numbers, dots, colons, underscores, or hyphens.' };
  }
  for (const [label, value, limit] of [
    ['Jurisdiction', editor.jurisdiction, 256],
    ['Citation', editor.statuteCitation, 512],
    ['Title', editor.title, 512],
  ] as const) {
    if (!value.trim() || value.trim().length > limit) {
      return { ok: false, error: `${label} is required and must be at most ${limit} characters.` };
    }
  }
  if (!editor.rawText.trim() || new TextEncoder().encode(editor.rawText).length > MAXIMUM_SOURCE_BYTES) {
    return { ok: false, error: 'Authority text is required and must remain within the 8 MiB import limit.' };
  }
  const deadline = editor.mandatedDeadlineDays.trim();
  if (deadline && (!/^\d+$/.test(deadline) || Number(deadline) > 36_500)) {
    return { ok: false, error: 'Deadline days must be a whole number from 0 through 36,500.' };
  }
  const effectiveFrom = editor.effectiveFrom ? parseIsoDateString(editor.effectiveFrom) : null;
  const effectiveTo = editor.effectiveTo ? parseIsoDateString(editor.effectiveTo) : null;
  if (editor.effectiveFrom && !effectiveFrom || editor.effectiveTo && !effectiveTo) {
    return { ok: false, error: 'Effective dates must be real calendar dates in YYYY-MM-DD form.' };
  }
  if (effectiveFrom && effectiveTo && effectiveFrom > effectiveTo) {
    return { ok: false, error: 'Effective-from date must not be later than effective-to date.' };
  }
  if (editor.canonicalSourceUri.trim()) {
    try {
      const source = new URL(editor.canonicalSourceUri.trim());
      if (source.protocol !== 'https:') throw new Error('not HTTPS');
    } catch {
      return { ok: false, error: 'Canonical source must be a valid HTTPS URL.' };
    }
  }
  const mandateIds = new Set<string>();
  for (const mandate of mandates) {
    const identifier = mandate.mandateId.trim();
    if (!/^[A-Za-z0-9._:-]{1,128}$/.test(identifier) || mandateIds.has(identifier)) {
      return { ok: false, error: 'Mandate IDs must be unique and use the same safe identifier format as authority IDs.' };
    }
    mandateIds.add(identifier);
    if (!mandate.title.trim() || !mandate.description.trim()) {
      return { ok: false, error: `Mandate ${identifier} requires a title and description.` };
    }
  }
  return { ok: true, effectiveFrom, effectiveTo };
}

interface TextFieldProps {
  label: string;
  value: string;
  onChange: (value: string) => void;
  required?: boolean;
  type?: 'text' | 'url' | 'number' | 'date';
  min?: string;
  max?: string;
}

const controlClass = 'analysis-input focus-ring mt-1.5 w-full';

function TextField({ label, value, onChange, required, type = 'text', min, max }: TextFieldProps) {
  return (
    <label className="block">
      <span className="text-control text-fg-secondary">{label}{required && <span aria-hidden="true"> *</span>}</span>
      <input type={type} value={value} onChange={(event) => onChange(event.target.value)} required={required} aria-required={required || undefined} min={min} max={max} className={controlClass} />
    </label>
  );
}

function PhraseField({ label, value, onChange }: { label: string; value: string; onChange: (value: string) => void }) {
  return (
    <label className="block">
      <span className="text-control text-fg-secondary">{label}<span aria-hidden="true"> *</span></span>
      <textarea value={value} onChange={(event) => onChange(event.target.value)} required rows={4} className="focus-ring mt-1.5 w-full resize-y rounded-lg border border-control bg-canvas px-3 py-2 font-mono text-code text-fg" />
      <span className="mt-1 block text-caption text-muted">One exact phrase sequence per line</span>
    </label>
  );
}

function nullable(value: string) {
  const normalized = value.trim();
  return normalized ? normalized : null;
}

function toMandateInput(mandate: MandateEditorState): LegalMandateInput {
  return {
    mandateId: mandate.mandateId.trim(),
    title: mandate.title.trim(),
    description: mandate.description.trim(),
    applicabilityPhrases: splitPhrases(mandate.applicabilityText),
    requiredEvidencePhrases: splitPhrases(mandate.evidenceText),
    severity: mandate.severity,
  };
}

function splitPhrases(value: string) {
  return value.split('\n').map((phrase) => phrase.trim()).filter(Boolean);
}
