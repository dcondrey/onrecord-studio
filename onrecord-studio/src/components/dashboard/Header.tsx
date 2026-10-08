import { useEffect } from 'react';
import { useAppStore } from '../../store/useAppStore';
import Icon from '../ui/icons';
import { useSourceConnector } from '../../hooks/useSourceConnector';
import TrialBanner from '../licensing/TrialBanner';
import { HeaderIdentity } from './HeaderIdentity';
import { HeaderControls } from './HeaderControls';
import { SourceConnectorForm } from './SourceConnectorForm';
import { SourceConnectorDialogs } from './SourceConnectorDialogs';
import { OPEN_SOURCE_CONNECTOR_EVENT } from './sourceEvents';
import NavigationControls from './NavigationControls';

export default function Header() {
  const workspaceView = useAppStore((state) => state.workspaceView);
  const setSidebarOpen = useAppStore((state) => state.setSidebarOpen);

  const connector = useSourceConnector();
  const { isConnectorOpen, setConnectorOpen, cancelSourceOperation, error, activePortal, syncingPortalKeys } = connector;

  useEffect(() => {
    const open = () => {
      setSidebarOpen(false);
      const desktop = document.getElementById('portal-url-desktop');
      if (desktop?.getClientRects().length) {
        desktop.focus();
        return;
      }
      setConnectorOpen(true);
      requestAnimationFrame(() => document.getElementById('portal-url-mobile')?.focus());
    };
    window.addEventListener(OPEN_SOURCE_CONNECTOR_EVENT, open);
    return () => window.removeEventListener(OPEN_SOURCE_CONNECTOR_EVENT, open);
  }, [setConnectorOpen, setSidebarOpen]);

  const toggleConnector = () => {
    if (isConnectorOpen) cancelSourceOperation();
    setConnectorOpen((open) => !open);
  };

  return (
    <>
      <header className="header-surface relative z-30 shrink-0 border-b">
        <TrialBanner />
        {/* 64px, the sidebar brand row's height, so the two bottom borders meet on one line. */}
        <div className="flex h-16 items-center gap-3 px-4 sm:px-5 lg:px-6">
          <button
            type="button"
            aria-label="Open navigation"
            onClick={() => setSidebarOpen(true)}
            className="focus-ring -ml-1 rounded-xl border border-control bg-surface/70 p-2.5 text-muted hover:text-fg md:hidden"
          >
            <Icon name="menu" />
          </button>

          <NavigationControls />
          <HeaderIdentity workspaceView={workspaceView} activePortal={activePortal} />

          {/* Inline only where the row has room for it; narrower windows open it below the row. */}
          <div className="hidden min-w-0 max-w-[720px] flex-1 2xl:flex">
            <SourceConnectorForm connector={connector} inputId="portal-url-desktop" />
          </div>

          <HeaderControls
            syncingPortalKeys={syncingPortalKeys}
            isConnectorOpen={isConnectorOpen}
            onToggleConnector={toggleConnector}
          />
        </div>
        {isConnectorOpen && (
          <div className="border-t border-line px-4 py-3 2xl:hidden">
            <SourceConnectorForm connector={connector} inputId="portal-url-mobile" />
          </div>
        )}
        {error && (
          <div role="alert" className="border-t border-danger/30 bg-danger/5 px-5 py-2 text-right text-caption text-danger">
            {error}
          </div>
        )}
      </header>
      <SourceConnectorDialogs connector={connector} />
    </>
  );
}
