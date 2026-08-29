import { useEffect, useMemo, useRef, useState } from 'react';
import { useAppStore } from '../../store/useAppStore';
import { cn } from '../../utils/cn';
import Icon, { type IconName } from '../ui/icons';
import { scoreCommand, themeCommandRegistry, workspaceCommandRegistry } from '../../commands/registry';

export const OPEN_COMMAND_PALETTE_EVENT = 'onrecord:open-command-palette';

interface CommandAction {
  id: string;
  label: string;
  detail: string;
  group: 'Navigate' | 'Create' | 'Appearance';
  icon: IconName;
  keywords: string;
  shortcut?: string;
  perform: () => void;
}

export default function CommandPalette() {
  const [isOpen, setOpen] = useState(false);
  const [query, setQuery] = useState('');
  const [selectedIndex, setSelectedIndex] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const previousFocus = useRef<HTMLElement | null>(null);
  const modifier = isApplePlatform() ? '⌘' : 'Ctrl';
  const setWorkspaceView = useAppStore((state) => state.setWorkspaceView);
  const setTheme = useAppStore((state) => state.setTheme);
  const toggleSidebarCollapsed = useAppStore((state) => state.toggleSidebarCollapsed);

  const openPalette = () => {
    previousFocus.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    setQuery('');
    setSelectedIndex(0);
    setOpen(true);
  };

  const closePalette = () => {
    setOpen(false);
    window.requestAnimationFrame(() => previousFocus.current?.focus());
  };

  const actions = useMemo<CommandAction[]>(() => [
    ...workspaceCommandRegistry.map((command) => ({
      id: command.id,
      label: command.label,
      detail: command.detail,
      group: 'Navigate' as const,
      icon: command.icon,
      keywords: command.keywords,
      shortcut: `${modifier}${command.shortcutIndex}`,
      perform: () => setWorkspaceView(command.view),
    })),
    {
      id: 'toggle-sidebar',
      label: 'Toggle navigation sidebar',
      detail: 'Show or hide workspace navigation labels',
      group: 'Navigate',
      icon: 'collapse',
      keywords: 'sidebar navigation collapse expand',
      perform: toggleSidebarCollapsed,
    },
    {
      id: 'add-source',
      label: 'Add a data source',
      detail: 'Connect a URL, file, API, or pasted material',
      group: 'Create',
      icon: 'filePlus',
      keywords: 'new connect import url file paste source',
      shortcut: `${modifier}N`,
      perform: focusSourceComposer,
    },
    ...themeCommandRegistry.map((command) => ({
      id: command.id,
      label: command.label,
      detail: command.detail,
      group: 'Appearance' as const,
      icon: command.icon,
      keywords: command.keywords,
      perform: () => setTheme(command.theme),
    })),
  ], [modifier, setTheme, setWorkspaceView, toggleSidebarCollapsed]);

  const filteredActions = useMemo(() => {
    const normalized = query.trim().toLocaleLowerCase();
    if (!normalized) return actions;
    return actions
      .map((action) => ({ action, score: scoreCommand(normalized, action.label, `${action.detail} ${action.group}`, action.keywords) }))
      .filter((entry): entry is { action: CommandAction; score: number } => entry.score !== null)
      .sort((left, right) => right.score - left.score || left.action.label.localeCompare(right.action.label))
      .map(({ action }) => action);
  }, [actions, query]);

  useEffect(() => {
    const openFromEvent = () => openPalette();
    window.addEventListener(OPEN_COMMAND_PALETTE_EVENT, openFromEvent);
    return () => window.removeEventListener(OPEN_COMMAND_PALETTE_EVENT, openFromEvent);
  });

  useEffect(() => {
    if (!isOpen) return;
    window.requestAnimationFrame(() => inputRef.current?.focus());
  }, [isOpen]);

  useEffect(() => {
    setSelectedIndex(0);
  }, [query]);

  useEffect(() => {
    const handleKeyboard = (event: KeyboardEvent) => {
      const modified = isApplePlatform() ? event.metaKey : event.ctrlKey;
      const key = event.key.toLocaleLowerCase();
      if (modified && key === 'k') {
        event.preventDefault();
        if (isOpen) closePalette();
        else openPalette();
        return;
      }
      if (!isOpen && modified && /^[1-4]$/.test(event.key)) {
        event.preventDefault();
        setWorkspaceView(workspaceCommandRegistry[Number(event.key) - 1].view);
        return;
      }
      if (!isOpen && modified && key === 'n') {
        event.preventDefault();
        focusSourceComposer();
        return;
      }
      if (!isOpen) return;
      if (event.key === 'Escape') {
        event.preventDefault();
        closePalette();
      } else if (event.key === 'ArrowDown') {
        event.preventDefault();
        setSelectedIndex((index) => filteredActions.length ? (index + 1) % filteredActions.length : 0);
      } else if (event.key === 'ArrowUp') {
        event.preventDefault();
        setSelectedIndex((index) => filteredActions.length ? (index - 1 + filteredActions.length) % filteredActions.length : 0);
      } else if (event.key === 'Enter' && filteredActions[selectedIndex]) {
        event.preventDefault();
        closePalette();
        filteredActions[selectedIndex].perform();
      }
    };
    window.addEventListener('keydown', handleKeyboard);
    return () => window.removeEventListener('keydown', handleKeyboard);
  }, [filteredActions, isOpen, selectedIndex, setWorkspaceView]);

  useEffect(() => {
    document.getElementById(`command-${filteredActions[selectedIndex]?.id}`)?.scrollIntoView({ block: 'nearest' });
  }, [filteredActions, selectedIndex]);

  if (!isOpen) return null;

  return (
    <div
      className="app-backdrop fixed inset-0 z-[120] flex justify-center px-4 pt-[12vh] backdrop-blur-md"
      role="presentation"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) closePalette();
      }}
    >
      <section className="command-dialog app-panel h-fit w-full max-w-[620px] overflow-hidden" role="dialog" aria-modal="true" aria-labelledby="command-palette-title">
        <h2 id="command-palette-title" className="sr-only">Command palette</h2>
        <div className="command-search flex items-center gap-3 border-b border-slate-800/70 px-4">
          <Icon name="search" className="h-[18px] w-[18px] shrink-0 text-slate-400" />
          <input
            ref={inputRef}
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder="Search commands, views, and settings"
            aria-label="Search commands, views, and settings"
            className="h-14 min-w-0 flex-1 border-0 bg-transparent text-[14px] text-slate-100 outline-none placeholder:text-slate-400"
            role="combobox"
            aria-expanded="true"
            aria-controls="command-results"
            aria-activedescendant={filteredActions[selectedIndex] ? `command-${filteredActions[selectedIndex].id}` : undefined}
            autoComplete="off"
            spellCheck={false}
          />
          <Keycap>Esc</Keycap>
        </div>

        <div id="command-results" className="max-h-[min(480px,62vh)] overflow-y-auto p-2" role="listbox">
          {filteredActions.length === 0 ? (
            <div className="px-4 py-10 text-center">
              <Icon name="search" className="mx-auto h-5 w-5 text-slate-400" />
              <p className="mt-3 text-[13px] font-medium text-slate-300">No matching commands</p>
              <p className="mt-1 text-[12px] text-slate-400">Try a workspace name, “source,” or “appearance.”</p>
            </div>
          ) : filteredActions.map((action, index) => {
            const showGroup = index === 0 || filteredActions[index - 1].group !== action.group;
            return (
              <div key={action.id}>
                {showGroup && <div className="sidebar-section-label px-3 pb-1 pt-2">{action.group}</div>}
                <button
                  id={`command-${action.id}`}
                  type="button"
                  role="option"
                  aria-selected={selectedIndex === index}
                  onMouseEnter={() => setSelectedIndex(index)}
                  onClick={() => {
                    closePalette();
                    action.perform();
                  }}
                  className={cn(
                    'command-row focus-ring flex w-full items-center gap-3 rounded-xl px-3 py-2.5 text-left transition',
                    selectedIndex === index ? 'command-row-selected text-slate-100' : 'text-slate-400',
                  )}
                >
                  <span className="command-icon grid h-8 w-8 shrink-0 place-items-center rounded-lg border">
                    <Icon name={action.icon} className="h-4 w-4" />
                  </span>
                  <span className="min-w-0 flex-1">
                    <span className="block truncate text-[13px] font-medium">{action.label}</span>
                    <span className="mt-0.5 block truncate text-[12px] text-slate-400">{action.detail}</span>
                  </span>
                  {action.shortcut && <Keycap>{action.shortcut}</Keycap>}
                </button>
              </div>
            );
          })}
        </div>
        <footer className="flex items-center justify-between border-t border-slate-800/70 px-4 py-2.5 text-[12px] text-slate-400">
          <span>OnRecord commands</span>
          <span className="flex items-center gap-2"><Keycap>↑↓</Keycap> Navigate <Keycap>↵</Keycap> Open</span>
        </footer>
      </section>
    </div>
  );
}

function Keycap({ children }: { children: string }) {
  return <kbd className="keycap shrink-0 rounded-md border px-1.5 py-0.5 font-sans text-[11px] font-medium text-slate-400">{children}</kbd>;
}

function focusSourceComposer() {
  const desktopInput = document.getElementById('portal-url-desktop');
  if (desktopInput?.getClientRects().length) {
    desktopInput.focus();
    return;
  }
  document.querySelector<HTMLButtonElement>('button[aria-label="Connect a data source"]')?.click();
  window.setTimeout(() => document.getElementById('portal-url-mobile')?.focus(), 0);
}

function isApplePlatform() {
  const platform = navigator.platform.toLocaleLowerCase();
  const userAgent = navigator.userAgent.toLocaleLowerCase();
  return platform.includes('mac') || userAgent.includes('mac os');
}
