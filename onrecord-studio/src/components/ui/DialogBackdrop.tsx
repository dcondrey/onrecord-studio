import type { ReactNode } from 'react';
import { useDialogFocus } from '../licensing/useDialogFocus';

/**
 * Modal overlay with the focus contract `aria-modal` promises: focus enters on open, Tab stays
 * inside, Escape closes, and focus returns to the opener. Mount it only while the dialog is open.
 */
export default function DialogBackdrop({
  labelledBy,
  label,
  onClose,
  className,
  children,
}: {
  labelledBy?: string;
  label?: string;
  onClose: () => void;
  className: string;
  children: ReactNode;
}) {
  const { ref, onKeyDown } = useDialogFocus(onClose);
  return (
    <div
      ref={ref}
      tabIndex={-1}
      role="dialog"
      aria-modal="true"
      aria-labelledby={labelledBy}
      aria-label={label}
      className={`${className} outline-none`}
      onKeyDown={onKeyDown}
    >
      {children}
    </div>
  );
}
