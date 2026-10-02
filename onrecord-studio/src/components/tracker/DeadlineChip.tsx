import { ShapeGlyph, type GlyphShape } from '../ui/shapeGlyph';
import { cn } from '../../utils/cn';
import type { RequestDeadline } from '../../types';

interface ChipSpec {
  shape: GlyphShape;
  dashed: boolean;
  label: string;
  tone: string;
}

/**
 * Shape, border style and words together carry the basis; colour only reinforces. A sourced
 * result is a solid diamond, a proxy a triangle, a provisional count a dashed ring.
 */
export function deadlineChipSpec(deadline: RequestDeadline): ChipSpec {
  const { countdown } = deadline;
  if (deadline.violationAsserted) {
    return { shape: 'diamond', dashed: false, label: 'Violation counted (sourced determination)', tone: 'border-danger/60 bg-danger/10 text-danger' };
  }
  switch (countdown.kind) {
    case 'DETERMINATION_RECORDED':
      return deadline.determinationBasis === 'SOURCED_DETERMINATION_EVENT'
        ? { shape: 'diamond', dashed: false, label: 'Determination recorded (from an email)', tone: 'border-success/60 bg-success/10 text-success' }
        : { shape: 'triangle', dashed: false, label: 'Determination recorded (your entry, proxy)', tone: 'border-warning/60 bg-warning/10 text-warning' };
    case 'DAYS_REMAINING':
      return { shape: 'ring', dashed: true, label: `${countdown.days} day${countdown.days === 1 ? '' : 's'} left (provisional)`, tone: 'border-line-strong bg-raised/40 text-fg' };
    case 'DEADLINE_PASSED':
      return { shape: 'ring', dashed: true, label: `Deadline passed ${countdown.days} day${countdown.days === 1 ? '' : 's'} ago, no determination recorded (provisional)`, tone: 'border-warning/60 bg-warning/10 text-warning' };
    case 'NO_SUBMISSION_DATE':
      return { shape: 'bar', dashed: true, label: 'No submission date recorded', tone: 'border-line-strong bg-raised/40 text-fg' };
    case 'NO_DEADLINE_MODEL':
      return { shape: 'bar', dashed: true, label: 'No deadline model for this jurisdiction', tone: 'border-line-strong bg-raised/40 text-fg' };
    case 'UNAVAILABLE':
      return { shape: 'ring', dashed: false, label: 'Dates conflict: not evaluated', tone: 'border-warning/60 bg-warning/10 text-warning' };
  }
}

export function DeadlineChip({ deadline }: { deadline: RequestDeadline }) {
  const spec = deadlineChipSpec(deadline);
  return (
    <span
      data-countdown={deadline.countdown.kind}
      className={cn(
        'inline-flex items-center gap-1 rounded-full border px-1.5 py-0.5 text-caption font-semibold',
        spec.dashed && 'border-dashed',
        spec.tone,
      )}
    >
      <svg viewBox="-6 -6 12 12" className="h-2.5 w-2.5 shrink-0" aria-hidden="true">
        <ShapeGlyph
          shape={spec.shape}
          cx={0}
          cy={0}
          radius={4.4}
          fill={spec.shape === 'ring' ? 'none' : 'currentColor'}
          stroke="currentColor"
          strokeWidth={spec.shape === 'ring' ? 1.6 : 0}
          strokeDasharray={spec.dashed ? '2 1.6' : undefined}
        />
      </svg>
      {spec.label}
    </span>
  );
}
