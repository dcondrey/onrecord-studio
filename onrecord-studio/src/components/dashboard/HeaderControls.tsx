import { cn } from '../../utils/cn';
import Icon from '../ui/icons';
import { useProfileLensStore } from '../../store/useProfileLensStore';
import ProfileSettings from '../account/ProfileSettings';
import SelfReview from '../account/SelfReview';
import AccountMenu from '../account/AccountMenu';
import MyRequestsButton from '../tracker/MyRequestsButton';
import HelpButton from '../help/HelpButton';
import UpdatePrompt from '../updates/UpdatePrompt';

/** The right-hand icon row: audio-alert toggle, sync status, and the mobile-only connector toggle.
 * Appearance lives in the View menu and the command palette. */
export function HeaderControls({
  audioAlertsEnabled,
  setAudioAlertsEnabled,
  syncingPortalKeys,
  isConnectorOpen,
  onToggleConnector,
}: {
  audioAlertsEnabled: boolean;
  setAudioAlertsEnabled: (enabled: boolean) => void;
  syncingPortalKeys: string[];
  isConnectorOpen: boolean;
  onToggleConnector: () => void;
}) {
  const setProfileOpen = useProfileLensStore((state) => state.setSettingsOpen);
  return (
    <div className="ml-auto flex shrink-0 items-center gap-2">
      <button
        type="button"
        title={audioAlertsEnabled ? 'Disable completion and failure sounds' : 'Enable completion and failure sounds'}
        aria-label={audioAlertsEnabled ? 'Disable audio alerts' : 'Enable audio alerts'}
        aria-pressed={audioAlertsEnabled}
        onClick={() => setAudioAlertsEnabled(!audioAlertsEnabled)}
        className={cn('focus-ring grid h-9 w-9 place-items-center rounded-xl border transition', audioAlertsEnabled ? 'border-accent/30 bg-accent/10 text-accent' : 'border-control bg-surface/70 text-muted hover:text-fg-secondary')}
      >
        <Icon name="volume" className="h-3.5 w-3.5" />
      </button>
      <button
        type="button"
        title="Local profile and declared organization"
        aria-label="Open local profile"
        onClick={() => setProfileOpen(true)}
        className="focus-ring grid h-9 w-9 place-items-center rounded-xl border border-control bg-surface/70 text-muted transition hover:text-fg-secondary"
      >
        <Icon name="shield" className="h-3.5 w-3.5" />
      </button>
      <HelpButton />
      <MyRequestsButton />
      <AccountMenu />
      <UpdatePrompt />
      <ProfileSettings />
      <SelfReview />
      <div className="system-status hidden h-9 items-center gap-2 rounded-xl border px-2.5 text-caption sm:flex" role="status" title={syncingPortalKeys.length > 0 ? `${syncingPortalKeys.length} sources synchronizing` : 'All systems ready'}>
        <span className={cn('grid h-5 w-5 place-items-center rounded-md', syncingPortalKeys.length > 0 ? 'status-pulse bg-success/10 text-success' : 'bg-success/10 text-success')}>
          <Icon name={syncingPortalKeys.length > 0 ? 'sync' : 'shield'} className={cn('h-3.5 w-3.5', syncingPortalKeys.length > 0 && 'animate-spin')} />
        </span>
        <span className="hidden text-muted 2xl:inline">{syncingPortalKeys.length > 0 ? `${syncingPortalKeys.length} source${syncingPortalKeys.length === 1 ? '' : 's'} live` : 'System ready'}</span>
      </div>
      <button
        type="button"
        aria-expanded={isConnectorOpen}
        onClick={onToggleConnector}
        className={cn('focus-ring flex items-center gap-1.5 rounded-xl border px-2.5 py-2 text-label font-semibold transition lg:hidden', isConnectorOpen ? 'border-accent/30 bg-accent/10 text-accent' : 'primary-action border-transparent text-white')}
      >
        <Icon name="filePlus" />
        <span>Add source</span>
      </button>
    </div>
  );
}
