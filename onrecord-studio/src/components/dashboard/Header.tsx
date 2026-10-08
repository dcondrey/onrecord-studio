import { useEffect } from 'react';
import { useAppStore } from '../../store/useAppStore';
import Button from '../ui/Button';
import Dialog, { DialogBody, DialogHeader } from '../ui/Dialog';
import { useSourceConnector } from '../../hooks/useSourceConnector';
import TrialBanner from '../licensing/TrialBanner';
import { HeaderIdentity } from './HeaderIdentity';
import { HeaderControls } from './HeaderControls';
import { SourceConnectorForm } from './SourceConnectorForm';
import { SourceConnectorDialogs } from './SourceConnectorDialogs';
import { OPEN_SOURCE_CONNECTOR_EVENT } from './sourceEvents';
import NavigationControls from './NavigationControls';

// openFileImport (utils/sanDiego.ts) opens the connector and then clicks `${id}-import-files`.
const CONNECTOR_INPUT_ID = 'portal-url-mobile';

export default function Header() {
  const setSidebarOpen = useAppStore((state) => state.setSidebarOpen);

  const connector = useSourceConnector();
  const { isConnectorOpen, setConnectorOpen, cancelSourceOperation, error, activePortal } = connector;

  useEffect(() => {
    const open = () => {
      setSidebarOpen(false);
      setConnectorOpen(true);
      requestAnimationFrame(() => document.getElementById(CONNECTOR_INPUT_ID)?.focus());
    };
    window.addEventListener(OPEN_SOURCE_CONNECTOR_EVENT, open);
    return () => window.removeEventListener(OPEN_SOURCE_CONNECTOR_EVENT, open);
  }, [setConnectorOpen, setSidebarOpen]);

  const closeConnector = () => {
    cancelSourceOperation();
    setConnectorOpen(false);
  };

  return (
    <>
      <header className="header-surface relative z-header shrink-0 border-b">
        <TrialBanner />
        <div className="flex h-14 items-center gap-3 px-4 md:px-6">
          <Button
            variant="ghost"
            iconOnly
            icon="menu"
            aria-label="Open navigation"
            onClick={() => setSidebarOpen(true)}
            className="-ml-1 md:hidden"
          />
          <NavigationControls />
          <HeaderIdentity activePortal={activePortal} />
          <HeaderControls isConnectorOpen={isConnectorOpen} onOpenConnector={() => setConnectorOpen(true)} />
        </div>
        {error && !isConnectorOpen && (
          <div role="alert" className="border-t border-danger/30 bg-danger/10 px-4 py-2 text-caption text-danger md:px-6">
            {error}
          </div>
        )}
      </header>
      {isConnectorOpen && (
        <Dialog size="lg" onClose={closeConnector}>
          <DialogHeader
            title="Add a source"
            description="Choose a built-in San Diego source, paste a public records portal or page URL, or add files."
          />
          <DialogBody className="space-y-3">
            <div className="flex">
              <SourceConnectorForm connector={connector} inputId={CONNECTOR_INPUT_ID} />
            </div>
            {error && <p role="alert" className="text-caption text-danger">{error}</p>}
          </DialogBody>
        </Dialog>
      )}
      <SourceConnectorDialogs connector={connector} />
    </>
  );
}
