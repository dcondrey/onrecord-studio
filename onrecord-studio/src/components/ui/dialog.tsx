import React, { useEffect } from 'react';
import { cn } from '../../utils/cn';

interface DialogProps {
  isOpen: boolean;
  onClose: () => void;
  title?: string;
  children: React.ReactNode;
  className?: string;
}

export function Dialog({ isOpen, onClose, title, children, className }: DialogProps) {
  // The backdrop closes on click, which a keyboard cannot do. Escape is the keyboard equivalent,
  // and without it a dialog opened by keyboard can only be left through the close button.
  useEffect(() => {
    if (!isOpen) return undefined;
    const dismiss = (event: KeyboardEvent) => {
      if (event.key === 'Escape') onClose();
    };
    window.addEventListener('keydown', dismiss);
    return () => window.removeEventListener('keydown', dismiss);
  }, [isOpen, onClose]);

  if (!isOpen) return null;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center p-4 select-none" role="dialog" aria-modal="true" aria-label={title}>
      {/* Absolute backdrop blur shadow layer */}
      <div 
        className="app-backdrop absolute inset-0 backdrop-blur-sm transition-opacity"
        aria-hidden="true"
        onClick={onClose} 
      />
      
      {/* Primary Modal Canvas Frame */}
      <div className={cn(
        "relative w-full max-w-lg rounded-xl border border-slate-800 bg-slate-900 shadow-2xl overflow-hidden flex flex-col max-h-[85vh] animate-in fade-in zoom-in-95 duration-150",
        className
      )}>
        {/* Upper Control Bar header */}
        <div className="h-11 border-b border-slate-800 px-4 flex items-center justify-between bg-slate-900/40">
          {title ? (
            <h3 className="text-xs font-bold text-slate-300 uppercase tracking-wider font-sans">
              {title}
            </h3>
          ) : <div />}
          <button 
            type="button"
            aria-label={title ? `Close ${title.toLowerCase()}` : 'Close dialog'}
            onClick={onClose}
            className="text-slate-400 hover:text-slate-300 p-1 rounded-md transition-all hover:bg-slate-800/50"
          >
            <svg className="h-3.5 w-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2.5}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>

        {/* Content body space */}
        <div className="flex-1 overflow-y-auto p-5 text-xs text-slate-300">
          {children}
        </div>
      </div>
    </div>
  );
}
