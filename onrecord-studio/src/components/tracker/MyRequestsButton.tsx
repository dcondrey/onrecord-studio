import { useState } from 'react';
import Icon from '../ui/icons';
import MyRequestsPanel from './MyRequestsPanel';

/** Header entry for the request tracker. Owns its own open state. */
export default function MyRequestsButton() {
  const [open, setOpen] = useState(false);
  return (
    <>
      <button
        type="button"
        title="Requests you filed and their deadlines"
        aria-label="Open my requests"
        onClick={() => setOpen(true)}
        className="focus-ring grid h-9 w-9 place-items-center rounded-xl border border-control bg-surface/70 text-muted transition hover:text-fg-secondary"
      >
        <Icon name="clipboard" className="h-3.5 w-3.5" />
      </button>
      {open && <MyRequestsPanel onClose={() => setOpen(false)} />}
    </>
  );
}
