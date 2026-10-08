import { cn } from '../../utils/cn';

export type SkeletonVariant = 'lines' | 'block' | 'card' | 'chart';

/**
 * A placeholder drawn at the final layout's height, so nothing shifts when the data arrives. It is
 * a shape, never a digit, so it cannot be read as a zero. Under reduced motion it is a flat fill.
 * `height` is the block's or the chart plot's height in pixels.
 */
export default function Skeleton({ variant = 'lines', lines = 3, height = 160, label, className }: {
  variant?: SkeletonVariant;
  lines?: number;
  height?: number;
  /** What is loading, for screen readers: "Loading leads". */
  label: string;
  className?: string;
}) {
  return (
    <div role="status" aria-busy="true" className={className}>
      <span className="sr-only">{label}</span>
      <div aria-hidden="true">
        {variant === 'lines' && (
          <div className="space-y-2 py-1">
            {Array.from({ length: lines }, (_, index) => (
              <div key={index} className={cn('skeleton h-3 rounded', index === lines - 1 && lines > 1 ? 'w-3/5' : 'w-full')} />
            ))}
          </div>
        )}
        {variant === 'block' && <div className="skeleton w-full rounded-lg" style={{ height }} />}
        {variant === 'card' && (
          <div className="flex flex-col gap-3 rounded-xl border border-line bg-panel p-4">
            <div className="flex gap-2">
              <div className="skeleton h-5 w-24 rounded-full" />
              <div className="skeleton h-5 w-20 rounded-full" />
            </div>
            <div className="skeleton h-4 w-4/5 rounded" />
            <div className="skeleton h-3 w-2/5 rounded" />
          </div>
        )}
        {variant === 'chart' && (
          <div className="flex flex-col gap-4">
            <div className="skeleton h-5 w-3/5 rounded" />
            <div className="skeleton w-full rounded-lg" style={{ height }} />
          </div>
        )}
      </div>
    </div>
  );
}
