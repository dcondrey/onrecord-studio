import { cn } from '../../utils/cn';
import Icon from '../ui/icons';
import type { AppTheme } from '../../store/useAppStore';

/** The right-hand icon row: audio-alert toggle, appearance switcher, sync status, and the
 * mobile-only connector toggle. */
export function HeaderControls({
  audioAlertsEnabled,
  setAudioAlertsEnabled,
  theme,
  setTheme,
  syncingPortalKeys,
  isConnectorOpen,
  onToggleConnector,
}: {
  audioAlertsEnabled: boolean;
  setAudioAlertsEnabled: (enabled: boolean) => void;
  theme: AppTheme;
  setTheme: (theme: AppTheme) => void;
  syncingPortalKeys: string[];
  isConnectorOpen: boolean;
  onToggleConnector: () => void;
}) {
  return (
    <div className="ml-auto flex shrink-0 items-center gap-2">
      <button
        type="button"
        title={audioAlertsEnabled ? 'Disable completion and failure sounds' : 'Enable completion and failure sounds'}
        aria-label={audioAlertsEnabled ? 'Disable audio alerts' : 'Enable audio alerts'}
        aria-pressed={audioAlertsEnabled}
        onClick={() => setAudioAlertsEnabled(!audioAlertsEnabled)}
        className={cn('focus-ring grid h-9 w-9 place-items-center rounded-xl border transition', audioAlertsEnabled ? 'border-cyan-400/30 bg-cyan-400/10 text-cyan-300' : 'border-slate-800 bg-slate-900/50 text-slate-400 hover:text-slate-300')}
      >
        <Icon name="volume" className="h-3.5 w-3.5" />
      </button>
      <div className="theme-switcher flex items-center rounded-xl border p-0.5" role="group" aria-label="Appearance">
        <button
          type="button"
          title="Light appearance"
          aria-label="Use light appearance"
          aria-pressed={theme === 'light'}
          onClick={() => setTheme('light')}
          className={cn('focus-ring grid h-8 w-8 place-items-center rounded-[9px] transition', theme === 'light' ? 'theme-switcher-active' : 'text-slate-400 hover:text-slate-300')}
        >
          <Icon name="sun" className="h-3.5 w-3.5" />
        </button>
        <button
          type="button"
          title="Dark appearance"
          aria-label="Use dark appearance"
          aria-pressed={theme === 'dark'}
          onClick={() => setTheme('dark')}
          className={cn('focus-ring grid h-8 w-8 place-items-center rounded-[9px] transition', theme === 'dark' ? 'theme-switcher-active' : 'text-slate-400 hover:text-slate-300')}
        >
          <Icon name="moon" className="h-3.5 w-3.5" />
        </button>
        <button
          type="button"
          title="Automatic appearance"
          aria-label="Follow system appearance"
          aria-pressed={theme === 'auto'}
          onClick={() => setTheme('auto')}
          className={cn('focus-ring grid h-8 w-8 place-items-center rounded-[9px] transition', theme === 'auto' ? 'theme-switcher-active' : 'text-slate-400 hover:text-slate-300')}
        >
          <Icon name="system" className="h-3.5 w-3.5" />
        </button>
      </div>
      <div className="system-status hidden h-9 items-center gap-2 rounded-xl border px-2.5 text-[9px] sm:flex" role="status" title={syncingPortalKeys.length > 0 ? `${syncingPortalKeys.length} sources synchronizing` : 'All systems ready'}>
        <span className={cn('grid h-5 w-5 place-items-center rounded-md', syncingPortalKeys.length > 0 ? 'status-pulse bg-emerald-400/10 text-emerald-300' : 'bg-emerald-400/10 text-emerald-300')}>
          <Icon name={syncingPortalKeys.length > 0 ? 'sync' : 'shield'} className={cn('h-3.5 w-3.5', syncingPortalKeys.length > 0 && 'animate-spin')} />
        </span>
        <span className="hidden text-slate-400 2xl:inline">{syncingPortalKeys.length > 0 ? `${syncingPortalKeys.length} source${syncingPortalKeys.length === 1 ? '' : 's'} live` : 'System ready'}</span>
      </div>
      <button
        type="button"
        aria-label="Connect a data source"
        onClick={onToggleConnector}
        className={cn('focus-ring rounded-xl border p-2.5 transition lg:hidden', isConnectorOpen ? 'border-cyan-400/30 bg-cyan-400/10 text-cyan-300' : 'border-slate-800 bg-slate-900/50 text-slate-400 hover:text-slate-100')}
      >
        <Icon name="database" />
      </button>
    </div>
  );
}
