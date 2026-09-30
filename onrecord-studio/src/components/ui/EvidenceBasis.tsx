import { useId, useRef, useState, type KeyboardEvent, type ReactNode } from 'react';
import { ShapeGlyph, type GlyphShape } from './shapeGlyph';
import { cn } from '../../utils/cn';
import { isLegalClaimKind } from '../../types/findings';
import { PLAIN_FINDING_TEXT } from '../../content/plainLanguage';
import type { Finding, FindingCitation, FindingConfidence, FindingKind } from '../../types/findings';
import type { EntityValidationGrade, RelationshipBasis, RelationshipKind } from '../../types/corpus';

/**
 * Shared visual language for "what kind of claim is this, and how do we know". Every chip pairs a
 * shape with a text label; colour is redundant reinforcement, never the only carrier.
 */

function Glyph({ shape, className, dashed }: { shape: GlyphShape; className?: string; dashed?: boolean }) {
  return (
    <svg viewBox="-6 -6 12 12" className={cn('h-2.5 w-2.5 shrink-0', className)} aria-hidden="true">
      <ShapeGlyph
        shape={shape}
        cx={0}
        cy={0}
        radius={4.4}
        fill={shape === 'ring' ? 'none' : 'currentColor'}
        stroke="currentColor"
        strokeWidth={shape === 'ring' ? 1.6 : 0}
        strokeDasharray={dashed ? '2 1.6' : undefined}
      />
    </svg>
  );
}

function Chip({ shape, dashed, tone, border, children, ...rest }: {
  shape: GlyphShape;
  dashed?: boolean;
  tone: string;
  border: string;
  children: ReactNode;
} & Record<`data-${string}`, string>) {
  return (
    <span
      {...rest}
      className={cn('inline-flex items-center gap-1 rounded-full border px-1.5 py-0.5 text-[9px] font-semibold', tone, border)}
    >
      <Glyph shape={shape} dashed={dashed} />
      {children}
    </span>
  );
}

/**
 * A legal claim is a counted violation against a stated rule; everything else is a pattern or
 * statistical signal. The two never share a glyph, label, or border style.
 */
export function FindingBasisChip({ kind }: { kind: FindingKind }) {
  return isLegalClaimKind(kind) ? (
    <Chip data-basis="legal-claim" shape="diamond" tone="text-rose-200" border="border-rose-400/60 bg-rose-500/10">
      Legal claim: counted violation
    </Chip>
  ) : (
    <Chip data-basis="pattern-signal" shape="ring" dashed tone="text-slate-200" border="border-dashed border-slate-500 bg-slate-800/40">
      Pattern signal, not a legal claim
    </Chip>
  );
}

/**
 * `UNASSESSED` and a measured 0% are different facts: the first says nothing was measured, the
 * second says something was measured and found no strength. They differ in shape, border, fill,
 * text, and accessible role.
 */
export function ConfidenceMeter({ confidence }: { confidence: FindingConfidence }) {
  if (confidence.kind === 'UNASSESSED') {
    return (
      <span
        data-confidence="unassessed"
        role="img"
        aria-label="Confidence unassessed: no measurement was made"
        className="inline-flex items-center gap-1 text-[9px] font-medium text-slate-300"
      >
        <span
          aria-hidden="true"
          className="h-1.5 w-12 rounded-sm border border-dashed border-slate-400"
          style={{ backgroundImage: 'repeating-linear-gradient(135deg, transparent 0 3px, rgba(148,163,184,0.45) 3px 4px)' }}
        />
        <span>Unassessed</span>
      </span>
    );
  }
  const bp = Math.min(10000, Math.max(0, Math.round(confidence.basisPoints)));
  const percent = Math.round(bp / 100);
  return (
    <span data-confidence="measured" className="inline-flex items-center gap-1 text-[9px] font-medium text-slate-200">
      <span
        role="meter"
        aria-label="Measured confidence"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={percent}
        aria-valuetext={`${percent}% measured`}
        className="h-1.5 w-12 overflow-hidden rounded-sm border border-slate-400 bg-slate-900"
      >
        <span className="block h-full bg-cyan-300" style={{ width: `${percent}%` }} />
      </span>
      <span>{percent}% measured</span>
    </span>
  );
}

const GRADES: Record<EntityValidationGrade, { shape: GlyphShape; label: string; detail: string; tone: string }> = {
  CHECK_DIGIT_PASSED: { shape: 'diamond', label: 'Check digit passed', detail: 'The matched string passed its own check-digit test.', tone: 'text-emerald-200' },
  FORMAT_VALIDATED: { shape: 'circle', label: 'Format validated', detail: 'The matched string parsed as a well-formed value of its kind.', tone: 'text-cyan-200' },
  GAZETTEER_HIT: { shape: 'triangle', label: 'Gazetteer hit', detail: 'The string matched a known-name list; it was not otherwise verified.', tone: 'text-amber-200' },
  PATTERN_ONLY: { shape: 'ring', label: 'Pattern only: a lead', detail: 'Matched by pattern alone with no validation. A lead, not a parsed value.', tone: 'text-slate-200' },
};

export function gradeGlyphShape(grade: EntityValidationGrade): GlyphShape {
  return GRADES[grade].shape;
}

export function gradeLabel(grade: EntityValidationGrade): string {
  return GRADES[grade].label;
}

export function ValidationGradeChip({ grade }: { grade: EntityValidationGrade }) {
  const { shape, label, detail, tone } = GRADES[grade];
  return (
    <span title={detail}>
      <Chip
        data-grade={grade}
        shape={shape}
        dashed={grade === 'PATTERN_ONLY'}
        tone={tone}
        border={grade === 'PATTERN_ONLY' ? 'border-dashed border-slate-500 bg-slate-800/40' : 'border-slate-600 bg-slate-900/60'}
      >
        {label}
      </Chip>
    </span>
  );
}

const EDGE_KIND_TEXT: Record<RelationshipKind, string> = {
  CO_OCCURRENCE: 'Co-occurrence',
  SIGNED_BY: 'Signed by',
  APPROVED: 'Approved',
  CITES: 'Cites',
  PROMISES: 'Promises',
  DELIVERED: 'Delivered',
  SUPERSEDES: 'Supersedes',
  AMENDS: 'Amends',
};

const TYPED_DASH: Record<Exclude<RelationshipKind, 'CO_OCCURRENCE'>, string> = {
  SIGNED_BY: '6 2 2 2',
  APPROVED: '10 3',
  CITES: '2 3',
  PROMISES: '8 4 2 4',
  DELIVERED: '12 3 2 3',
  SUPERSEDES: '1 3',
  AMENDS: '6 3 1 3',
};

const BASIS_TEXT: Record<RelationshipBasis, string> = {
  OBSERVED: 'observed in the text',
  COMPUTED: 'computed by this app',
  ASSERTED: 'asserted by a source',
};

export interface EdgeVisual {
  /** `proximity` says two entities sat near each other; `typed` says the text carried a relation. */
  family: 'proximity' | 'typed';
  dash: string | undefined;
  label: string;
  markerShape: GlyphShape;
  opacity: number;
}

/**
 * Co-occurrence is always dotted, unlabelled by relation, and drawn fainter, whatever its basis:
 * proximity must not borrow the solid stroke that a typed, observed relation earns.
 */
export function edgeVisual(kind: RelationshipKind, basis: RelationshipBasis): EdgeVisual {
  if (kind === 'CO_OCCURRENCE') {
    return { family: 'proximity', dash: '1.5 4', label: EDGE_KIND_TEXT.CO_OCCURRENCE, markerShape: 'ring', opacity: 0.4 };
  }
  return {
    family: 'typed',
    dash: basis === 'OBSERVED' ? undefined : TYPED_DASH[kind],
    label: EDGE_KIND_TEXT[kind],
    markerShape: 'diamond',
    opacity: 0.85,
  };
}

export function EdgeBasisChip({ kind, basis }: { kind: RelationshipKind; basis: RelationshipBasis }) {
  const visual = edgeVisual(kind, basis);
  return visual.family === 'proximity' ? (
    <Chip data-edge-family="proximity" shape="ring" dashed tone="text-slate-200" border="border-dashed border-slate-500 bg-slate-800/40">
      Co-occurrence: proximity only ({BASIS_TEXT[basis]})
    </Chip>
  ) : (
    <Chip data-edge-family="typed" shape="diamond" tone="text-cyan-100" border="border-cyan-400/60 bg-cyan-500/10">
      {visual.label}: relation read from text ({BASIS_TEXT[basis]})
    </Chip>
  );
}

export function edgeExplanation(kind: RelationshipKind, basis: RelationshipBasis, sharedDocuments: number): string {
  const docs = `${sharedDocuments} shared document${sharedDocuments === 1 ? '' : 's'}`;
  if (kind === 'CO_OCCURRENCE') {
    return `Both entities appear in the same segment across ${docs}. This app derived the link (${BASIS_TEXT[basis]}); no relationship between them was read from the text.`;
  }
  return `A "${EDGE_KIND_TEXT[kind].toLowerCase()}" cue in the text connects these entities across ${docs} (${BASIS_TEXT[basis]}).`;
}

function citationRange(citation: FindingCitation): string {
  if (citation.characterStart === null || citation.characterEnd === null) return 'no character range recorded';
  return `characters ${citation.characterStart}–${citation.characterEnd}`;
}

/**
 * Keyboard-operable disclosure (click, Enter, or Space; Escape closes) that lists why a claim is
 * believed. It expands in flow rather than floating, so it works at any zoom and without a
 * pointer. Each citation names its source document and offers a link when `onOpenSource` is given.
 */
export function EvidenceTrace({
  label,
  explanation,
  citations,
  emptyNote = 'No citations recorded, so this claim has no source passage to trace to.',
  onOpenSource,
}: {
  label: string;
  explanation: string;
  citations: FindingCitation[];
  emptyNote?: string;
  onOpenSource?: (citation: FindingCitation) => void;
}) {
  const [open, setOpen] = useState(false);
  const panelId = useId();
  const triggerRef = useRef<HTMLButtonElement>(null);

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key !== 'Escape' || !open) return;
    event.stopPropagation();
    setOpen(false);
    triggerRef.current?.focus();
  };

  return (
    <div onKeyDown={onKeyDown} className="text-[9px]">
      <button
        ref={triggerRef}
        type="button"
        aria-expanded={open}
        aria-controls={panelId}
        aria-label={`Why we believe this: ${label}`}
        onClick={() => setOpen((current) => !current)}
        className="focus-ring inline-flex items-center gap-1 rounded-md border border-slate-600 px-1.5 py-1 font-semibold text-slate-200 transition hover:border-cyan-400/60"
      >
        <span aria-hidden="true">{open ? '−' : '+'}</span>
        Why we believe this
      </button>
      {open && (
        <div id={panelId} role="group" aria-label={`Evidence trace: ${label}`} className="mt-1 rounded-lg border border-slate-700 bg-slate-950/80 p-2 text-slate-300">
          <p className="leading-relaxed">{explanation}</p>
          {citations.length === 0 ? (
            <p className="mt-1 text-slate-300">{emptyNote}</p>
          ) : (
            <ol className="mt-1 flex flex-col gap-1.5" aria-label="Citations">
              {citations.map((citation, index) => (
                <li key={`${citation.portalKey}-${citation.documentId}-${index}`} className="rounded border border-slate-800 p-1.5">
                  <div className="font-semibold text-slate-100">{citation.label ?? `Document ${citation.documentId}`}</div>
                  <div className="text-slate-400">
                    {citation.portalKey} · document {citation.documentId} · {citationRange(citation)}
                  </div>
                  {citation.quote === null ? (
                    <div className="text-slate-400">No quote recorded.</div>
                  ) : (
                    <blockquote className="mt-0.5 border-l-2 border-slate-500 pl-1.5 italic text-slate-200">{citation.quote}</blockquote>
                  )}
                  {onOpenSource && (
                    <button
                      type="button"
                      onClick={() => onOpenSource(citation)}
                      className="focus-ring mt-1 rounded border border-slate-600 px-1.5 py-0.5 font-semibold text-cyan-200 hover:border-cyan-400/60"
                    >
                      Open source document {citation.documentId}
                    </button>
                  )}
                </li>
              ))}
            </ol>
          )}
        </div>
      )}
    </div>
  );
}

/**
 * Inline plain-language reading of a finding kind, collapsed by default. The basis sentence comes
 * from `isLegalClaimKind`, the same source as the chip above it, so this text can neither upgrade
 * a pattern signal nor soften a counted comparison.
 */
export function PlainLanguageDisclosure({ kind, label }: { kind: FindingKind; label: string }) {
  const [open, setOpen] = useState(false);
  const panelId = useId();
  const text = PLAIN_FINDING_TEXT[kind];
  const basis = isLegalClaimKind(kind)
    ? 'Basis: a counted comparison against a stated rule. It is not a court finding.'
    : 'Basis: a pattern signal, not a legal claim. It is a reason to look, not a conclusion.';
  return (
    <div className="text-[9px]">
      <button
        type="button"
        aria-expanded={open}
        aria-controls={panelId}
        aria-label={`Plain-language explanation: ${label}`}
        onClick={() => setOpen((current) => !current)}
        className="focus-ring inline-flex items-center gap-1 rounded-md border border-slate-600 px-1.5 py-1 font-semibold text-slate-200 transition hover:border-cyan-400/60"
      >
        <span aria-hidden="true">{open ? '−' : '+'}</span>
        In plain words
      </button>
      {open && (
        <div id={panelId} role="group" aria-label={`Plain-language explanation of ${label}`} className="mt-1 rounded-lg border border-slate-700 bg-slate-950/80 p-2 text-slate-300">
          <dl className="flex flex-col gap-1 leading-relaxed">
            <div>
              <dt className="font-semibold text-slate-100">What was observed</dt>
              <dd>{text.observed}</dd>
            </div>
            <div>
              <dt className="font-semibold text-slate-100">What it does not show</dt>
              <dd>{text.notShown}</dd>
            </div>
            <div>
              <dt className="font-semibold text-slate-100">What to do next</dt>
              <dd>{text.next}</dd>
            </div>
          </dl>
          <p className="mt-1">{basis}</p>
        </div>
      )}
    </div>
  );
}

/** Basis chip, confidence meter, and trace for one finding, as a unit. */
export function FindingEvidence({
  finding,
  onOpenSource,
}: {
  finding: Finding;
  onOpenSource?: (citation: FindingCitation) => void;
}) {
  return (
    <div className="flex flex-col gap-1">
      <div className="flex flex-wrap items-center gap-2">
        <FindingBasisChip kind={finding.kind} />
        <ConfidenceMeter confidence={finding.confidence} />
      </div>
      <EvidenceTrace
        label={finding.headline}
        explanation={finding.explanation}
        citations={finding.citations}
        onOpenSource={onOpenSource}
      />
      <PlainLanguageDisclosure kind={finding.kind} label={finding.headline} />
    </div>
  );
}
