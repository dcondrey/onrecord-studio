import type { ReactNode } from 'react';
import MethodNote from './MethodNote';

interface PageHeaderProps {
  title: string;
  /** One sentence. Anything longer belongs in `method`. */
  summary?: ReactNode;
  /** How to read the page: basis, caveats, what a number does and does not claim. Shown on request. */
  method?: ReactNode;
  /** Medium (32px) controls only, so every action in the row shares one height. */
  actions?: ReactNode;
}

/** The page title. The header breadcrumb already names the view, so there is no eyebrow. */
export default function PageHeader({ title, summary, method, actions }: PageHeaderProps) {
  return (
    <header className="flex flex-col gap-4 xl:flex-row xl:items-end xl:justify-between">
      <div className="min-w-0 flex-1">
        <h2 className="text-balance text-display text-fg">{title}</h2>
        {(summary || method) && (
          <div className="mt-2 max-w-prose text-body text-muted">
            {summary}
            {summary && method && ' '}
            {method && <MethodNote label="How to read this">{method}</MethodNote>}
          </div>
        )}
      </div>
      {actions && <div className="flex flex-wrap items-center gap-2 xl:shrink-0 xl:justify-end">{actions}</div>}
    </header>
  );
}
