import { cn } from '../../utils/cn';
import Button from './Button';

/**
 * A failed load, stated in plain words. The transport text (error codes, command names) stays
 * available under "Technical details" for a bug report, not in the sentence the reader sees.
 */
export default function InlineError({ summary, detail, onRetry, className }: {
  summary: string;
  detail?: string;
  onRetry?: () => void;
  className?: string;
}) {
  return (
    <div role="alert" className={cn('rounded-lg border border-danger/30 bg-danger/10 px-4 py-3', className)}>
      <div className="flex flex-wrap items-center justify-between gap-x-4 gap-y-2">
        <p className="min-w-0 flex-1 text-body text-danger">{summary}</p>
        {onRetry && <Button size="sm" onClick={onRetry}>Try again</Button>}
      </div>
      {detail && detail !== summary && (
        <details className="mt-2">
          <summary className="text-caption text-muted">Technical details</summary>
          <p className="mt-1 break-words font-mono text-code text-fg-secondary">{detail}</p>
        </details>
      )}
    </div>
  );
}
