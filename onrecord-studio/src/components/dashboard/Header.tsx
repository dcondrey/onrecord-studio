import { useAppStore } from '../../store/useAppStore';
import Icon from '../ui/icons';
import { useSourceConnector } from '../../hooks/useSourceConnector';
import TrialBanner from '../licensing/TrialBanner';
import { HeaderIdentity } from './HeaderIdentity';
import { HeaderControls } from './HeaderControls';
import { SourceConnectorForm } from './SourceConnectorForm';
import { SourceConnectorDialogs } from './SourceConnectorDialogs';

export default function Header() {
  const workspaceView = useAppStore((state) => state.workspaceView);
  const audioAlertsEnabled = useAppStore((state) => state.audioAlertsEnabled);
  const setSidebarOpen = useAppStore((state) => state.setSidebarOpen);
  const setAudioAlertsEnabled = useAppStore((state) => state.setAudioAlertsEnabled);

  const connector = useSourceConnector();
  const { isConnectorOpen, setConnectorOpen, cancelSourceOperation, error, activePortal, syncingPortalKeys } = connector;

  const toggleConnector = () => {
    if (isConnectorOpen) cancelSourceOperation();
    setConnectorOpen((open) => !open);
  };

  return (
    <>
      <header className="header-surface relative z-30 shrink-0 border-b backdrop-blur-xl">
        <TrialBanner />
        <div className="flex min-h-[68px] items-center gap-3 px-4 sm:px-5 lg:px-6">
          <button
            type="button"
            aria-label="Open navigation"
            onClick={() => setSidebarOpen(true)}
            className="focus-ring -ml-1 rounded-xl border border-slate-800 bg-slate-900/50 p-2.5 text-slate-400 hover:text-slate-100 md:hidden"
          >
            <Icon name="menu" />
          </button>

          <HeaderIdentity workspaceView={workspaceView} activePortal={activePortal} />

          <div className="hidden min-w-[300px] max-w-[720px] flex-1 lg:flex">
            <SourceConnectorForm connector={connector} inputId="portal-url-desktop" />
          </div>

          <HeaderControls
            audioAlertsEnabled={audioAlertsEnabled}
            setAudioAlertsEnabled={setAudioAlertsEnabled}
            syncingPortalKeys={syncingPortalKeys}
            isConnectorOpen={isConnectorOpen}
            onToggleConnector={toggleConnector}
          />
        </div>
        {isConnectorOpen && (
          <div className="border-t border-slate-800/70 px-4 py-3 lg:hidden">
            <SourceConnectorForm connector={connector} inputId="portal-url-mobile" />
          </div>
        )}
        {error && (
          <div role="alert" className="border-t border-rose-500/10 bg-rose-500/[0.05] px-5 py-2 text-right text-caption text-rose-300">
            {error}
          </div>
        )}
      </header>
      <SourceConnectorDialogs connector={connector} />
    </>
  );
}
