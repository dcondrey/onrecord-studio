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
    return { shape: 'diamond', dashed: false, label: 'Violation counted (sourced determination)', tone: 'border-rose-400/60 bg-rose-500/10 text-rose-200' };
  }
  switch (countdown.kind) {
    case 'DETERMINATION_RECORDED':
      return deadline.determinationBasis === 'SOURCED_DETERMINATION_EVENT'
        ? { shape: 'diamond', dashed: false, label: 'Determination recorded (from an email)', tone: 'border-emerald-400/50 bg-emerald-500/10 text-emerald-200' }
        : { shape: 'triangle', dashed: false, label: 'Determination recorded (your entry, proxy)', tone: 'border-amber-400/50 bg-amber-500/10 text-amber-200' };
    case 'DAYS_REMAINING':
      return { shape: 'ring', dashed: true, label: `${countdown.days} day${countdown.days === 1 ? '' : 's'} left (provisional)`, tone: 'border-slate-500 bg-slate-800/40 text-slate-200' };
    case 'DEADLINE_PASSED':
      return { shape: 'ring', dashed: true, label: `Deadline passed ${countdown.days} day${countdown.days === 1 ? '' : 's'} ago, no determination recorded (provisional)`, tone: 'border-amber-400/50 bg-amber-500/10 text-amber-200' };
    case 'NO_SUBMISSION_DATE':
      return { shape: 'bar', dashed: true, label: 'No submission date recorded', tone: 'border-slate-500 bg-slate-800/40 text-slate-200' };
    case 'NO_DEADLINE_MODEL':
      return { shape: 'bar', dashed: true, label: 'No deadline model for this jurisdiction', tone: 'border-slate-500 bg-slate-800/40 text-slate-200' };
    case 'UNAVAILABLE':
      return { shape: 'ring', dashed: false, label: 'Dates conflict: not evaluated', tone: 'border-amber-400/50 bg-amber-500/10 text-amber-200' };
  }
}

export function DeadlineChip({ deadline }: { deadline: RequestDeadline }) {
  const spec = deadlineChipSpec(deadline);
  return (
    <span
      data-countdown={deadline.countdown.kind}
      className={cn(
        'inline-flex items-center gap-1 rounded-full border px-1.5 py-0.5 text-[9px] font-semibold',
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
