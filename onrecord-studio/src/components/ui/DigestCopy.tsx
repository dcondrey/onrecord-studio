import { useState } from 'react';
import { cn } from '../../utils/cn';
import Button from './Button';

/**
 * A SHA-256 digest printed in full (an evidence identifier is never shortened; it breaks anywhere
 * instead) with a button that copies it. The copy's outcome is announced beside it.
 */
export default function DigestCopy({ sha256, className }: { sha256: string; className?: string }) {
  const [copied, setCopied] = useState<boolean | null>(null);
  return (
    <span className={cn('flex min-w-0 items-start gap-1', className)}>
      <span className="min-w-0 break-all font-mono text-code">SHA-256 {sha256}</span>
      <Button
        size="sm"
        variant="ghost"
        iconOnly
        icon="clipboard"
        className="-mt-1 h-6 w-6 shrink-0"
        aria-label="Copy the SHA-256 digest"
        title="Copy the SHA-256 digest"
        onClick={() => {
          const writing = navigator.clipboard?.writeText(sha256);
          if (writing) void writing.then(() => setCopied(true), () => setCopied(false));
          else setCopied(false);
        }}
      />
      {copied !== null && <span role="status" className={cn('shrink-0', copied ? 'text-success' : 'text-warning')}>{copied ? 'Copied' : 'Copying was blocked'}</span>}
    </span>
  );
}
