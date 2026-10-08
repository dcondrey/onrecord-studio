import { useState } from 'react';
import Button from '../ui/Button';
import MyRequestsPanel from './MyRequestsPanel';

/** Header entry for the request tracker. Owns its own open state. */
export default function MyRequestsButton() {
  const [open, setOpen] = useState(false);
  return (
    <>
      <Button
        variant="ghost"
        iconOnly
        icon="clipboard"
        title="Requests you filed and their deadlines"
        aria-label="Open my requests"
        onClick={() => setOpen(true)}
        className="focus-ring"
      />
      {open && <MyRequestsPanel onClose={() => setOpen(false)} />}
    </>
  );
}
