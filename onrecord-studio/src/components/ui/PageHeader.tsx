import { useId, useState, type ReactNode } from 'react';
import Icon, { type IconName } from './icons';
import { cn } from '../../utils/cn';

interface PageHeaderProps {
  icon: IconName;
  eyebrow: string;
  title: string;
  /** One sentence. Anything longer belongs in `method`. */
  summary?: ReactNode;
  /** How to read the page: basis, caveats, what a number does and does not claim. Shown on request. */
  method?: ReactNode;
  actions?: ReactNode;
}

export default function PageHeader({ icon, eyebrow, title, summary, method, actions }: PageHeaderProps) {
  const [methodOpen, setMethodOpen] = useState(false);
  const methodId = useId();
  return (
    <header className="flex flex-col gap-3 sm:flex-row sm:items-end sm:justify-between">
      <div className="min-w-0">
        <div className="app-eyebrow flex items-center gap-2 text-accent">
          <Icon name={icon} className="h-3.5 w-3.5" /> {eyebrow}
        </div>
        <h2 className="mt-2 text-[26px] font-semibold tracking-[-0.035em] text-fg sm:text-[30px]">{title}</h2>
        {(summary || method) && (
          <p className="mt-1.5 max-w-3xl text-label leading-relaxed text-muted">
            {summary}
            {method && (
              <button
                type="button"
                aria-expanded={methodOpen}
                aria-controls={methodId}
                onClick={() => setMethodOpen((open) => !open)}
                className="focus-ring ml-1.5 inline-flex items-center gap-1 rounded text-accent hover:underline"
              >
                How to read this
                <Icon name="chevron" className={cn('h-3 w-3 transition-transform', methodOpen && 'rotate-90')} />
              </button>
            )}
          </p>
        )}
        {method && methodOpen && (
          <div id={methodId} className="motion-reveal mt-2 max-w-3xl rounded-lg border border-line bg-surface/60 px-3 py-2 text-label leading-relaxed text-fg-secondary">
            {method}
          </div>
        )}
      </div>
      {actions && <div className="flex shrink-0 flex-wrap items-center gap-2">{actions}</div>}
    </header>
  );
}
