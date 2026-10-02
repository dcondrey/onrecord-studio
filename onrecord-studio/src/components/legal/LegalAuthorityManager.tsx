import DialogBackdrop from '../ui/DialogBackdrop';
import { useEffect, useRef, useState, type ChangeEvent, type FormEvent } from 'react';
import { invokeCommand } from '../../ipc';
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

  useEffect(() => {
    if (!isOpen) return;
    let active = true;
    setIsLoading(true);
    setError(null);
    void invokeCommand('list_legal_authorities', {})
      .then((value) => {
        if (active) setAuthorities(value);
      })
      .catch((reason) => {
        if (active) setError(String(reason));
      })
      .finally(() => {
        if (active) setIsLoading(false);
      });
    return () => {
      active = false;
    };
  }, [isOpen, setAuthorities]);

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
      if (active) setVersionError(String(reason));
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
      if (active) setVersionError(String(reason));
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
      setError(`Could not read the selected source: ${String(reason)}`);
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
      setError(String(reason));
    } finally {
      setIsImporting(false);
    }
  };

  return (
    <DialogBackdrop labelledBy="legal-library-title" onClose={onClose} className="app-backdrop fixed inset-0 z-[70] grid place-items-center p-6 backdrop-blur-sm">
      <div className="grid h-[min(860px,92vh)] w-[min(1380px,96vw)] grid-cols-[360px_minmax(0,1fr)] overflow-hidden rounded-2xl border border-line-strong bg-canvas shadow-2xl shadow-black/60">
        <aside className="flex min-h-0 flex-col border-r border-line bg-surface/40">
          <div className="border-b border-line px-5 py-4">
            <div className="text-caption font-bold uppercase tracking-[0.2em] text-highlight">
              Versioned authority index
            </div>
            <h2 id="legal-library-title" className="mt-1 text-base font-semibold text-fg">
              Legal authority library
            </h2>
            <p className="mt-1 text-label leading-relaxed text-muted">
              Local statutes, ordinances, and operational policies with traceable source versions.
            </p>
          </div>
          <div className="min-h-0 flex-1 overflow-y-auto p-3">
            {isLoading ? (
              <div className="p-3 text-label text-muted">Loading indexed authorities…</div>
            ) : authorities.length === 0 ? (
              <div className="rounded-lg border border-dashed border-line p-4 text-label leading-relaxed text-muted">
                No authorities are indexed yet.
              </div>
            ) : (
              <div className="space-y-2">
                {authorities.map((authority) => (
                  <button type="button" key={authority.statuteId} onClick={() => setSelectedStatuteId(authority.statuteId)} className={cn('focus-ring block w-full rounded-lg border bg-canvas/70 p-3 text-left', selectedStatuteId === authority.statuteId ? 'border-highlight/30' : 'border-control')}>
                    <div className="flex items-start justify-between gap-2">
                      <div className="min-w-0">
                        <div className="truncate text-label font-semibold text-fg">
                          {authority.statuteCitation}
                        </div>
                        <div className="mt-1 truncate text-caption text-muted">
                          {authority.jurisdiction}
                        </div>
                      </div>
                      <span className="rounded border border-line-strong px-1.5 py-0.5 text-caption text-muted">
                        {authority.authorityKind.split('_').join(' ')}
                      </span>
                    </div>
                    <p className="mt-2 line-clamp-2 text-caption leading-relaxed text-muted">
                      {authority.title}
                    </p>
                    <div className="mt-2 flex gap-3 font-mono text-caption text-muted">
                      <span>{authority.provisionCount} provisions</span>
                      <span>{authority.mandateCount} evidence rules</span>
                    </div>
                    <div className="mt-2 truncate font-mono text-caption text-muted">
                      SHA-256 {authority.contentSha256}
                    </div>
                  </button>
                ))}
              </div>
            )}
          </div>
        </aside>

        <section className="flex min-h-0 flex-col">
          <div className="flex items-center justify-between border-b border-line px-5 py-3">
            <div>
              <h3 className="text-label leading-4 font-semibold text-fg">Index an authority</h3>
              <p className="mt-0.5 text-caption text-muted">
                Markdown headings and legal section labels become independently searchable provisions.
              </p>
            </div>
            <button
              type="button"
              autoFocus
              onClick={onClose}
              className="rounded-md border border-control px-2.5 py-1.5 text-caption font-semibold text-muted transition hover:border-control hover:text-fg"
            >
              Close
            </button>
          </div>

          {selectedStatuteId && <AuthorityVersionWorkspace versions={versions} leftVersionId={leftVersionId} rightVersionId={rightVersionId} leftVersion={leftVersion} rightVersion={rightVersion} error={versionError} onLeftChange={setLeftVersionId} onRightChange={setRightVersionId} />}

          <form onSubmit={(event) => void submit(event)} className="min-h-0 flex-1 overflow-y-auto p-5">
            <div className="grid grid-cols-2 gap-3">
              <TextField label="Stable authority ID" value={editor.statuteId} onChange={(value) => updateEditor('statuteId', value)} required />
              <TextField label="Jurisdiction" value={editor.jurisdiction} onChange={(value) => updateEditor('jurisdiction', value)} required />
              <TextField label="Citation" value={editor.statuteCitation} onChange={(value) => updateEditor('statuteCitation', value)} required />
              <TextField label="Title" value={editor.title} onChange={(value) => updateEditor('title', value)} required />
              <label className="block">
                <span className="text-caption font-semibold uppercase tracking-wider text-muted">Authority type</span>
                <select value={editor.authorityKind} onChange={(event) => updateEditor('authorityKind', event.target.value as LegalAuthorityKind)} className={controlClass}>
                  <option value="STATE_STATUTE">State statute</option>
                  <option value="MUNICIPAL_ORDINANCE">Municipal ordinance</option>
                  <option value="AGENCY_POLICY">Agency policy</option>
                  <option value="PROCEDURE_MANUAL">Procedure manual</option>
                  <option value="REGULATION">Regulation</option>
                  <option value="OTHER">Other authority</option>
                </select>
              </label>
              <TextField label="Calendar-day deadline" value={editor.mandatedDeadlineDays} onChange={(value) => updateEditor('mandatedDeadlineDays', value)} type="number" min="0" max="36500" />
              <TextField label="Official HTTPS source" value={editor.canonicalSourceUri} onChange={(value) => updateEditor('canonicalSourceUri', value)} type="url" />
              <div className="grid grid-cols-2 gap-3">
                <TextField label="Effective from" value={editor.effectiveFrom} onChange={(value) => updateEditor('effectiveFrom', value)} type="date" />
                <TextField label="Effective to" value={editor.effectiveTo} onChange={(value) => updateEditor('effectiveTo', value)} type="date" />
              </div>
            </div>

            <div className="mt-4 rounded-xl border border-line bg-surface/40 p-4">
              <div className="flex items-center justify-between gap-3">
                <div>
                  <div className="text-caption font-semibold text-fg-secondary">Legislative or policy text</div>
                  <div className="mt-0.5 text-caption text-muted">UTF-8 Markdown or plain text, up to 8 MiB</div>
                </div>
                <label className="cursor-pointer rounded-md border border-highlight/30 bg-highlight/10 px-2.5 py-1.5 text-caption font-semibold text-highlight transition hover:bg-highlight/20">
                  Choose source file
                  <input ref={fileInput} type="file" accept=".md,.markdown,.txt,text/markdown,text/plain" onChange={(event) => void loadFile(event)} className="sr-only" />
                </label>
              </div>
              {sourceFileName && (
                <div className="mt-2 font-mono text-caption text-success">
                  {sourceFileName} · {editor.rawText.length.toLocaleString()} characters loaded
                </div>
              )}
              <textarea
                value={editor.rawText}
                onChange={(event) => updateEditor('rawText', event.target.value)}
                required
                rows={8}
                aria-label="Legal source text"
                className={`${controlClass} mt-3 resize-y font-mono leading-relaxed`}
              />
            </div>

            <div className="mt-4">
              <div className="flex items-center justify-between">
                <div>
                  <div className="text-caption font-semibold text-fg-secondary">Policy evidence rules</div>
                  <p className="mt-0.5 text-caption text-muted">Optional. Phrase sequences are tokenized, stemmed, and matched on word boundaries.</p>
                </div>
                <button type="button" onClick={() => setMandates((current) => [...current, emptyMandate()])} className="rounded-md border border-control px-2.5 py-1.5 text-caption font-semibold text-muted transition hover:border-control hover:text-fg">
                  Add evidence rule
                </button>
              </div>
              <div className="mt-3 space-y-3">
                {mandates.map((mandate, index) => (
                  <div key={mandate.key} className="rounded-xl border border-line bg-surface/40 p-4">
                    <div className="flex items-center justify-between">
                      <span className="text-caption font-bold uppercase tracking-[0.16em] text-muted">Evidence rule {index + 1}</span>
                      <button type="button" onClick={() => setMandates((current) => current.filter((_, position) => position !== index))} className="text-caption font-semibold text-danger hover:text-danger">Remove</button>
                    </div>
                    <div className="mt-3 grid grid-cols-[1fr_1.5fr_140px] gap-3">
                      <TextField label="Rule ID" value={mandate.mandateId} onChange={(value) => updateMandate(index, 'mandateId', value)} required />
                      <TextField label="Rule title" value={mandate.title} onChange={(value) => updateMandate(index, 'title', value)} required />
                      <label className="block">
                        <span className="text-caption font-semibold uppercase tracking-wider text-muted">Severity</span>
                        <select value={mandate.severity} onChange={(event) => updateMandate(index, 'severity', event.target.value as LegalMandateSeverity)} className={controlClass}>
                          <option value="ADVISORY">Advisory</option>
                          <option value="MODERATE">Moderate</option>
                          <option value="HIGH">High</option>
                          <option value="CRITICAL">Critical</option>
                        </select>
                      </label>
                    </div>
                    <label className="mt-3 block">
                      <span className="text-caption font-semibold uppercase tracking-wider text-muted">Rule rationale</span>
                      <textarea value={mandate.description} onChange={(event) => updateMandate(index, 'description', event.target.value)} required rows={2} className={`${controlClass} resize-y`} />
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
            <div className="mt-5 flex items-center justify-between border-t border-line pt-4">
              <p className="max-w-lg text-caption leading-relaxed text-muted">
                Imports are content-addressed. Reimporting changed text creates a new immutable version while retaining prior findings and review history.
              </p>
              <button type="submit" disabled={isImporting} className="rounded-lg border border-highlight/30 bg-highlight/20 px-4 py-2 text-caption font-semibold text-highlight transition hover:bg-highlight/20 disabled:cursor-wait disabled:opacity-60">
                {isImporting ? 'Parsing and indexing…' : 'Index authority'}
              </button>
            </div>
          </form>
        </section>
      </div>
    </DialogBackdrop>
  );
}

function AuthorityVersionWorkspace({ versions, leftVersionId, rightVersionId, leftVersion, rightVersion, error, onLeftChange, onRightChange }: { versions: LegalAuthorityVersionSummary[]; leftVersionId: string | null; rightVersionId: string | null; leftVersion: LegalAuthorityVersionDetail | null; rightVersion: LegalAuthorityVersionDetail | null; error: string | null; onLeftChange: (value: string) => void; onRightChange: (value: string) => void }) {
  const changes = leftVersion && rightVersion ? compareAuthorityText(leftVersion.fullText, rightVersion.fullText) : null;
  return <section className="border-b border-line bg-surface/40 px-5 py-3" aria-label="Authority version comparison"><div className="flex items-center justify-between gap-3"><div><div className="text-caption font-semibold text-fg-secondary">Effective-date timeline and immutable text comparison</div><div className="mt-0.5 text-caption text-muted">{versions.length} content-addressed version{versions.length === 1 ? '' : 's'}, newest first</div></div>{changes && <span className="font-mono text-caption text-highlight">{changes.changed.toLocaleString()} changed · {changes.added.toLocaleString()} added · {changes.removed.toLocaleString()} removed lines</span>}</div>{error ? <p role="alert" className="mt-2 text-caption text-danger">{error}</p> : versions.length === 0 ? <p className="mt-2 text-caption text-muted">No immutable versions are indexed for this authority.</p> : <><div className="mt-2 flex gap-1 overflow-x-auto pb-1">{versions.map((version) => <span key={version.versionId} className="shrink-0 rounded border border-line bg-canvas px-2 py-1 font-mono text-caption text-muted" title={version.contentSha256}>{version.effectiveFrom ?? 'start unknown'} → {version.effectiveTo ?? 'open ended'} · {formatVersionDate(version.importedAt)}</span>)}</div><div className="mt-2 grid grid-cols-2 gap-2">{([['Earlier / baseline', leftVersionId, onLeftChange, leftVersion], ['Later / comparison', rightVersionId, onRightChange, rightVersion]] as const).map(([label, selected, change, detail]) => <label key={label} className="block text-caption text-muted">{label}<select value={selected ?? ''} onChange={(event) => change(event.target.value)} className="ml-2 rounded border border-control bg-canvas px-2 py-1 font-mono text-caption text-fg-secondary">{versions.map((version) => <option key={version.versionId} value={version.versionId}>{formatVersionDate(version.importedAt)} · {version.contentSha256.slice(0, 10)}</option>)}</select><textarea readOnly value={detail?.fullText ?? 'Loading version text…'} aria-label={`${label} authority text`} className="mt-1.5 h-28 w-full resize-y rounded border border-control bg-canvas p-2 font-mono text-caption leading-relaxed text-muted" /></label>)}</div></>}</section>;
}

function compareAuthorityText(left: string, right: string) {
  const leftLines = left.split('\n');
  const rightLines = right.split('\n');
  const shared = Math.min(leftLines.length, rightLines.length);
  let changed = 0;
  for (let index = 0; index < shared; index += 1) if (leftLines[index] !== rightLines[index]) changed += 1;
  return { changed, added: Math.max(0, rightLines.length - leftLines.length), removed: Math.max(0, leftLines.length - rightLines.length) };
}

function formatVersionDate(value: string) {
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? value : date.toLocaleDateString();
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

const controlClass = 'mt-1.5 w-full rounded-md border border-line bg-canvas/70 px-2.5 py-2 text-label text-fg-secondary outline-none transition focus:border-highlight/60 focus:ring-1 focus:ring-highlight/30';

function TextField({ label, value, onChange, required, type = 'text', min, max }: TextFieldProps) {
  return (
    <label className="block">
      <span className="text-caption font-semibold uppercase tracking-wider text-muted">{label}</span>
      <input type={type} value={value} onChange={(event) => onChange(event.target.value)} required={required} min={min} max={max} className={controlClass} />
    </label>
  );
}

function PhraseField({ label, value, onChange }: { label: string; value: string; onChange: (value: string) => void }) {
  return (
    <label className="block">
      <span className="text-caption font-semibold uppercase tracking-wider text-muted">{label}</span>
      <textarea value={value} onChange={(event) => onChange(event.target.value)} required rows={4} className={`${controlClass} resize-y font-mono leading-relaxed`} />
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
