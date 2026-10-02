import { useEffect, useMemo, useRef, useState } from 'react';
import { invokeCommand, formatNativeError } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import { useAppStore } from '../../store/useAppStore';
import type { IntentAnswer, IntentOption } from '../../types';
import IntentAnswerView from './IntentAnswerView';
import { cn } from '../../utils/cn';
import Icon, { type IconName } from '../ui/icons';
import { openHelp } from '../help/helpEvents';
import { openSourceConnector } from './sourceEvents';
import { OBJECT_ICONS } from '../objects/ObjectLink';
import { useObjectSearch, type ObjectGroup } from './useObjectSearch';
import { contrastCommandRegistry, motionCommandRegistry, scoreCommand, themeCommandRegistry, workspaceCommandRegistry } from '../../commands/registry';

export const OPEN_COMMAND_PALETTE_EVENT = 'onrecord:open-command-palette';

const INTENT_ROW_LIMIT = 10;

interface CommandAction {
  id: string;
  label: string;
  detail: string;
  group: 'Ask' | 'Navigate' | 'Create' | 'Help' | 'Appearance' | 'Accessibility' | ObjectGroup;
  icon: IconName;
  keywords: string;
  shortcut?: string;
  /** The row starts work whose result the palette shows, so choosing it must not close the palette. */
  keepOpen?: boolean;
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
  const setEvidenceMode = useAppStore((state) => state.setEvidenceMode);
  const setTheme = useAppStore((state) => state.setTheme);
  const setMotion = useAppStore((state) => state.setMotion);
  const setContrast = useAppStore((state) => state.setContrast);
  const toggleSidebarCollapsed = useAppStore((state) => state.toggleSidebarCollapsed);
  const openSubjectDossier = useAppStore((state) => state.openSubjectDossier);
  const [answer, setAnswer] = useState<IntentAnswer | null>(null);
  const [asking, setAsking] = useState(false);
  const [askError, setAskError] = useState<string | null>(null);
  const answerGate = useLatestRequestGate();
  // A chosen suggestion changes the query, and that change invalidates any answer in flight. The
  // suggestion is asked from the effect that handles the change, after the invalidation.
  const askOnQuery = useRef<string | null>(null);

  const clearAnswer = () => {
    answerGate.invalidate();
    setAnswer(null);
    setAsking(false);
    setAskError(null);
  };

  const runIntent = (utterance: string) => {
    const token = answerGate.begin();
    setAsking(true);
    setAskError(null);
    setAnswer(null);
    invokeCommand('run_intent_query', { utterance, limit: INTENT_ROW_LIMIT })
      .then((result) => {
        if (!answerGate.isCurrent(token)) return;
        setAnswer(result);
      })
      .catch((error: unknown) => {
        if (!answerGate.isCurrent(token)) return;
        setAskError(formatNativeError(error));
      })
      .finally(() => {
        if (answerGate.isCurrent(token)) setAsking(false);
      });
  };

  const chooseOption = (option: IntentOption) => {
    askOnQuery.current = option.example;
    setQuery(option.example);
  };

  const openPalette = () => {
    previousFocus.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    clearAnswer();
    setQuery('');
    setSelectedIndex(0);
    setOpen(true);
  };

  const closePalette = () => {
    clearAnswer();
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
      id: 'open-timeline',
      label: 'Show evidence timeline',
      detail: 'Requests, findings and watch changes on one dated list',
      group: 'Navigate',
      icon: 'activity',
      keywords: 'timeline chronology dates events history sequence',
      perform: () => {
        setEvidenceMode('timeline');
        setWorkspaceView('evidence');
      },
    },
    {
      id: 'navigate-back',
      label: 'Go back',
      detail: 'Return to the previous view with its filters and scroll position',
      group: 'Navigate',
      icon: 'back',
      keywords: 'back previous history return',
      shortcut: `${modifier}[`,
      perform: () => useAppStore.getState().navigateBack(),
    },
    {
      id: 'navigate-forward',
      label: 'Go forward',
      detail: 'Redo the last back step',
      group: 'Navigate',
      icon: 'forward',
      keywords: 'forward next history',
      shortcut: `${modifier}]`,
      perform: () => useAppStore.getState().navigateForward(),
    },
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
      perform: openSourceConnector,
    },
    {
      id: 'open-help',
      label: 'Open Help',
      detail: 'Search the documentation for views, findings, and data handling',
      group: 'Help',
      icon: 'search',
      keywords: 'help documentation manual guide how to learn support faq privacy data',
      shortcut: `${modifier}⇧/`,
      // The palette restores focus to its opener on the next frame. Opening Help after that frame
      // lets the dialog keep the focus it takes on mount.
      perform: () => window.requestAnimationFrame(() => openHelp()),
    },
    {
      // Opens the subject dossier for a name the user types. The prompt is the native window
      // one — the palette itself is a keyboard-first surface, and a full inline text step here
      // would drift into Tier 5's intent-router territory, which is out of Wave 2E's scope.
      id: 'open-subject-dossier',
      label: 'Open subject dossier…',
      detail: 'Stitch findings, entity graph, and provenance for one person, agency or record',
      group: 'Navigate',
      icon: 'evidence',
      keywords: 'dossier subject entity person agency who look up investigate',
      perform: () => {
        if (typeof window === 'undefined') return;
        const value = window.prompt(
          'Subject to open dossier for (name, email, organization, or entity id):',
        );
        const trimmed = value?.trim();
        if (!trimmed) return;
        openSubjectDossier({ kind: 'name', value: trimmed });
        setWorkspaceView('overview');
      },
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
    ...motionCommandRegistry.map((command) => ({
      id: command.id,
      label: command.label,
      detail: command.detail,
      group: 'Accessibility' as const,
      icon: command.icon,
      keywords: command.keywords,
      perform: () => setMotion(command.motion),
    })),
    ...contrastCommandRegistry.map((command) => ({
      id: command.id,
      label: command.label,
      detail: command.detail,
      group: 'Accessibility' as const,
      icon: command.icon,
      keywords: command.keywords,
      perform: () => setContrast(command.contrast),
    })),
  ], [modifier, openSubjectDossier, setContrast, setEvidenceMode, setMotion, setTheme, setWorkspaceView, toggleSidebarCollapsed]);

  const objectResults = useObjectSearch(isOpen ? query : '');
  const inspect = useAppStore((state) => state.inspect);
  const objectActions = useMemo<CommandAction[]>(() => objectResults.map((result) => ({
    id: `object-${result.key}`,
    label: result.label,
    detail: result.detail,
    group: result.group,
    icon: OBJECT_ICONS[result.target.kind],
    keywords: '',
    perform: () => inspect(result.target, 'view'),
  })), [inspect, objectResults]);

  const filteredActions = useMemo(() => {
    const normalized = query.trim().toLocaleLowerCase();
    if (!normalized) return actions;
    const matches = actions
      .map((action) => ({ action, score: scoreCommand(normalized, action.label, `${action.detail} ${action.group}`, action.keywords) }))
      .filter((entry): entry is { action: CommandAction; score: number } => entry.score !== null)
      .sort((left, right) => right.score - left.score || left.action.label.localeCompare(right.action.label))
      .map(({ action }) => action);
    if (!isQuestionLike(normalized)) return [...matches, ...objectActions];
    const ask: CommandAction = {
      id: 'ask-intent',
      label: `Ask: ${query.trim()}`,
      detail: 'Answered from findings, the entity graph and document text. No AI model is used.',
      group: 'Ask',
      icon: 'search',
      keywords: '',
      keepOpen: true,
      perform: () => runIntent(query.trim()),
    };
    // A sentence is a question first; a word or two that also names a view stays a view jump.
    return normalized.split(/\s+/).length >= 3 || normalized.endsWith('?') ? [ask, ...matches, ...objectActions] : [...matches, ...objectActions, ask];
    // runIntent closes over the gate and setters only, all stable.
  }, [actions, objectActions, query]);

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
    // Whatever was asked about the previous text says nothing about this text.
    clearAnswer();
    if (askOnQuery.current === query) {
      askOnQuery.current = null;
      runIntent(query.trim());
    }
    // clearAnswer and runIntent close over the gate and setters only, all stable.
  }, [query]);

  useEffect(() => {
    const handleKeyboard = (event: KeyboardEvent) => {
      // Enter and the arrows steer an IME composition before the text is committed. Acting on
      // them would run the highlighted command on text the user has not finished typing.
      if (event.isComposing || event.keyCode === 229) return;
      const modified = isApplePlatform() ? event.metaKey : event.ctrlKey;
      const key = event.key.toLocaleLowerCase();
      if (modified && key === 'k') {
        event.preventDefault();
        if (isOpen) closePalette();
        else openPalette();
        return;
      }
      const shortcut = workspaceCommandRegistry.find((command) => String(command.shortcutIndex) === event.key);
      if (!isOpen && modified && shortcut) {
        event.preventDefault();
        setWorkspaceView(shortcut.view);
        return;
      }
      if (!isOpen && modified && !event.shiftKey && (event.key === '[' || event.key === ']')) {
        event.preventDefault();
        if (event.key === '[') useAppStore.getState().navigateBack();
        else useAppStore.getState().navigateForward();
        return;
      }
      if (!isOpen && modified && event.shiftKey && (event.key === '?' || event.key === '/')) {
        event.preventDefault();
        openHelp();
        return;
      }
      if (!isOpen && modified && key === 'n') {
        event.preventDefault();
        openSourceConnector();
        return;
      }
      if (!isOpen) return;
      const showingAnswer = answer !== null || asking || askError !== null;
      if (event.key === 'Escape') {
        event.preventDefault();
        if (showingAnswer) clearAnswer();
        else closePalette();
      } else if (showingAnswer) {
        return;
      } else if (event.key === 'ArrowDown') {
        event.preventDefault();
        setSelectedIndex((index) => filteredActions.length ? (index + 1) % filteredActions.length : 0);
      } else if (event.key === 'ArrowUp') {
        event.preventDefault();
        setSelectedIndex((index) => filteredActions.length ? (index - 1 + filteredActions.length) % filteredActions.length : 0);
      } else if (event.key === 'Enter' && filteredActions[selectedIndex]) {
        event.preventDefault();
        const chosen = filteredActions[selectedIndex];
        if (!chosen.keepOpen) closePalette();
        chosen.perform();
      }
    };
    window.addEventListener('keydown', handleKeyboard);
    return () => window.removeEventListener('keydown', handleKeyboard);
    // closePalette and clearAnswer close over the gate and setters only, all stable.
  }, [answer, askError, asking, filteredActions, isOpen, selectedIndex, setWorkspaceView]);

  useEffect(() => {
    document.getElementById(`command-${filteredActions[selectedIndex]?.id}`)?.scrollIntoView({ block: 'nearest' });
  }, [filteredActions, selectedIndex]);

  if (!isOpen) return null;

  const answering = asking || askError !== null || answer !== null;

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
        <div className="command-search flex items-center gap-3 border-b border-line px-4">
          <Icon name="search" className="h-[18px] w-[18px] shrink-0 text-muted" />
          <input
            ref={inputRef}
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder="Search findings, records, agencies and commands, or ask a question"
            aria-label="Search findings, records, agencies, entities, passages and commands, or ask a question"
            className="h-14 min-w-0 flex-1 border-0 bg-transparent text-[14px] text-fg outline-none placeholder:text-muted"
            role="combobox"
            aria-expanded="true"
            aria-controls="command-results"
            aria-activedescendant={filteredActions[selectedIndex] ? `command-${filteredActions[selectedIndex].id}` : undefined}
            autoComplete="off"
            spellCheck={false}
          />
          <Keycap>Esc</Keycap>
        </div>

        {/* The live region stays mounted while the palette is open: an answer that mounts its own
            live region already populated is not announced, so only its contents may change. */}
        <div aria-live="polite">
          {answering && (
            <div id="command-results" className="max-h-[min(480px,62vh)] overflow-y-auto">
              {asking && <p className="px-6 py-8 text-center text-label text-fg-secondary" role="status">Searching findings, the entity graph and document text…</p>}
              {askError !== null && (
                <p className="px-6 py-8 text-center text-label text-fg-secondary" role="alert">The question could not be answered: {askError}</p>
              )}
              {answer !== null && <IntentAnswerView answer={answer} onChooseOption={chooseOption} />}
            </div>
          )}
        </div>
        {!answering && (
        <div id="command-results" className="max-h-[min(480px,62vh)] overflow-y-auto p-2" role="listbox">
          {filteredActions.length === 0 ? (
            <div className="px-4 py-10 text-center">
              <Icon name="search" className="mx-auto h-5 w-5 text-muted" />
              <p className="mt-3 text-label font-medium text-fg-secondary">No matching commands</p>
              <p className="mt-1 text-label text-muted">Try an agency, a finding, a phrase from a document, or a view name.</p>
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
                    if (!action.keepOpen) closePalette();
                    action.perform();
                  }}
                  className={cn(
                    'command-row focus-ring flex w-full items-center gap-3 rounded-xl px-3 py-2.5 text-left transition',
                    selectedIndex === index ? 'command-row-selected text-fg' : 'text-muted',
                  )}
                >
                  <span className="command-icon grid h-8 w-8 shrink-0 place-items-center rounded-lg border">
                    <Icon name={action.icon} className="h-4 w-4" />
                  </span>
                  <span className="min-w-0 flex-1">
                    <span className="block truncate text-label font-medium">{action.label}</span>
                    <span className="mt-0.5 block truncate text-label text-muted">{action.detail}</span>
                  </span>
                  {action.shortcut && <Keycap>{action.shortcut}</Keycap>}
                </button>
              </div>
            );
          })}
        </div>
        )}
        <footer className="flex items-center justify-between border-t border-line px-4 py-2.5 text-label text-muted">
          <span>{answering ? 'Esc returns to commands' : 'OnRecord commands'}</span>
          <span className="flex items-center gap-2"><Keycap>↑↓</Keycap> Navigate <Keycap>↵</Keycap> Open or ask</span>
        </footer>
      </section>
    </div>
  );
}

/** Three characters and a space or question mark: a phrase, not a lone view name. */
function isQuestionLike(normalized: string) {
  return normalized.length >= 3 && (/\s/.test(normalized) || normalized.endsWith('?'));
}

function Keycap({ children }: { children: string }) {
  return <kbd className="keycap shrink-0 rounded-md border px-1.5 py-0.5 font-sans text-label font-medium text-muted">{children}</kbd>;
}

function isApplePlatform() {
  const platform = navigator.platform.toLocaleLowerCase();
  const userAgent = navigator.userAgent.toLocaleLowerCase();
  return platform.includes('mac') || userAgent.includes('mac os');
}
