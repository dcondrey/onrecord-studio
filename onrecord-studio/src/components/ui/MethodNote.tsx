import { useId, useState, type ReactNode } from 'react';
import { cn } from '../../utils/cn';
import Icon from './icons';

/** Keeps a panel's basis and caveats one click away instead of above its content. */
export default function MethodNote({ children, label = 'How this works' }: { children: ReactNode; label?: string }) {
  const [open, setOpen] = useState(false);
  const id = useId();
  return (
    <>
      <button
        type="button"
        aria-expanded={open}
        aria-controls={id}
        onClick={() => setOpen((current) => !current)}
        className="focus-ring inline-flex items-center gap-1 rounded text-label text-accent hover:underline"
      >
        {label}
        <Icon name="chevron" className={cn('h-3 w-3 transition-transform', open && 'rotate-90')} />
      </button>
      {open && (
        <div id={id} className="motion-reveal mt-1.5 rounded-lg border border-line bg-surface/60 px-3 py-2 text-label leading-relaxed text-fg-secondary">
          {children}
        </div>
      )}
    </>
  );
}
