import type { ReactNode } from 'react';
import { useDialogFocus } from './useDialogFocus';

/** Focus moves into the dialog on open, Tab stays inside, Escape closes it, and focus returns to the opener. */
export default function ModalDialog({
  label,
  onClose,
  children,
}: {
  label: string;
  onClose: () => void;
  children: ReactNode;
}) {
  const { ref, onKeyDown } = useDialogFocus(onClose);

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-label={label}
      className="fixed inset-0 z-[110] grid place-items-center bg-black/60 p-4"
      onKeyDown={onKeyDown}
    >
      <div
        ref={ref}
        tabIndex={-1}
        className="max-h-full w-full max-w-lg overflow-y-auto rounded-2xl border border-slate-700 bg-slate-900 p-6 text-slate-100 shadow-2xl outline-none"
      >
        {children}
        <button
          type="button"
          onClick={onClose}
          className="focus-ring mt-4 rounded-lg border border-slate-500 px-3 py-1.5 text-xs font-semibold text-slate-100 hover:bg-slate-800"
        >
          Close
        </button>
      </div>
    </div>
  );
}
