import type { ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { useDialogFocus } from '../licensing/useDialogFocus';

/**
 * Modal overlay with the focus contract `aria-modal` promises: focus enters on open, Tab stays
 * inside, Escape closes, and focus returns to the opener. Mount it only while the dialog is open.
 * It renders into `document.body` so an ancestor with a filter or transform cannot trap its fixed
 * overlay. New dialogs use `Dialog`, which also owns the panel, header and footer.
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
  return createPortal(
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
    </div>,
    document.body,
  );
}
