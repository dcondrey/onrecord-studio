import type { HTMLAttributes, ReactNode } from 'react';
import { cn } from '../../utils/cn';
import { ShapeGlyph, type GlyphShape } from './shapeGlyph';

export type BadgeTone = 'neutral' | 'accent' | 'success' | 'warning' | 'danger';

const TONE: Record<BadgeTone, string> = {
  neutral: 'border-line bg-raised text-fg-secondary',
  accent: 'border-accent/30 bg-accent/10 text-accent',
  success: 'border-success/30 bg-success/10 text-success',
  warning: 'border-warning/30 bg-warning/10 text-warning',
  danger: 'border-danger/30 bg-danger/10 text-danger',
};

/** The leading shape that carries a badge's meaning without relying on its colour. */
export function BadgeGlyph({ shape, dashed }: { shape: GlyphShape; dashed?: boolean }) {
  return (
    <svg viewBox="-6 -6 12 12" className="h-2.5 w-2.5 shrink-0" aria-hidden="true">
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

export interface BadgeProps extends HTMLAttributes<HTMLSpanElement> {
  tone?: BadgeTone;
  glyph?: GlyphShape;
  /** A dashed border marks an unassessed, pattern-only or proximity reading; nothing else. */
  dashed?: boolean;
  children: ReactNode;
}

/** A sentence-case status pill. Text is the carrier; tone and glyph reinforce it. */
export default function Badge({ tone = 'neutral', glyph, dashed = false, className, children, ...props }: BadgeProps) {
  return (
    <span
      className={cn(
        'inline-flex h-5 items-center gap-1 whitespace-nowrap rounded-full border px-2 text-caption font-medium',
        TONE[tone],
        dashed && 'border-dashed',
        className,
      )}
      {...props}
    >
      {glyph && <BadgeGlyph shape={glyph} dashed={dashed} />}
      {children}
    </span>
  );
}

/** A count beside a nav item or tab. An unmeasured count is omitted by the caller, never shown as 0. */
export function CountBadge({ value, className, ...props }: { value: number | string } & HTMLAttributes<HTMLSpanElement>) {
  return (
    <span
      className={cn('inline-flex h-5 min-w-5 items-center justify-center rounded-full bg-raised px-1.5 text-caption font-medium tabular-nums text-fg-secondary', className)}
      {...props}
    >
      {value}
    </span>
  );
}
