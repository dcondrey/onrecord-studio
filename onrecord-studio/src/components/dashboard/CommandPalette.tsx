import { useEffect, useMemo, useRef, useState, type KeyboardEvent as ReactKeyboardEvent } from 'react';
import { invokeCommand, formatNativeError } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import { useAppStore } from '../../store/useAppStore';
import type { IntentAnswer, IntentOption, StoryLead } from '../../types';
import { useGatedLoad } from '../../hooks/useGatedLoad';
import ScoreSpark from '../charts/figures/ScoreSpark';
import { scoreLabel } from './LeadLine';
import { COUNTED_FACTOR, isUnassessed, orderedFactors, points } from './storyFactors';
import IntentAnswerView from './IntentAnswerView';
import { cn } from '../../utils/cn';
import Icon, { type IconName } from '../ui/icons';
import InlineError from '../ui/InlineError';
import Skeleton from '../ui/Skeleton';
import { openHelp } from '../help/helpEvents';
import { openSourceConnector } from './sourceEvents';
import { OBJECT_ICONS } from '../objects/ObjectLink';
import { useObjectSearch, type ObjectGroup } from './useObjectSearch';
import { contrastCommandRegistry, motionCommandRegistry, scoreCommand, themeCommandRegistry, workspaceCommandRegistry } from '../../commands/registry';

export const OPEN_COMMAND_PALETTE_EVENT = 'onrecord:open-command-palette';

const INTENT_ROW_LIMIT = 10;
/** Leads on the palette's first page, before anything is typed. */
const PALETTE_LEADS = 3;

interface CommandAction {
  id: string;
  label: string;
  detail: string;
  group: 'Ask' | 'Leads' | 'Navigate' | 'Create' | 'Help' | 'Appearance' | 'Accessibility' | ObjectGroup;
  icon: IconName;
  keywords: string;
  shortcut?: string;
  /** The row starts work whose result the palette shows, so choosing it must not close the palette. */
  keepOpen?: boolean;
  /** A setting row whose value is the one in force. */
  current?: boolean;
  /** A ranked lead: the row shows its rank and its attention score at sparkline scale. */
  lead?: { rank: number; lead: StoryLead; scaleMax: number };
  perform: () => void;
}

/** Settings rows are listed only when the query asks for them; they would otherwise bury the rest. */
const SETTING_GROUPS = new Set<CommandAction['group']>(['Appearance', 'Accessibility']);

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
  const theme = useAppStore((state) => state.theme);
  const motion = useAppStore((state) => state.motion);
  const contrast = useAppStore((state) => state.contrast);
  const audioAlertsEnabled = useAppStore((state) => state.audioAlertsEnabled);
  const setAudioAlertsEnabled = useAppStore((state) => state.setAudioAlertsEnabled);
  // 'dossier' turns the input into the subject field for "Open subject dossier…".
  const [mode, setMode] = useState<'commands' | 'dossier'>('commands');
  const dialogRef = useRef<HTMLElement>(null);
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
    setMode('commands');
    setQuery('');
    setSelectedIndex(0);
    setOpen(true);
  };

  const closePalette = () => {
    clearAnswer();
    setMode('commands');
    setOpen(false);
    window.requestAnimationFrame(() => previousFocus.current?.focus());
  };

  const actions = useMemo<CommandAction[]>(() => [
    ...workspaceCommandRegistry.map((command) => ({
      id: command.id,
      label: `Go to ${command.navigationLabel}`,
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
      icon: 'clock',
      keywords: 'timeline chronology dates events history sequence',
      perform: () => {
        setEvidenceMode('timeline');
        setWorkspaceView('evidence');
      },
    },
    {
      id: 'open-subject-dossier',
      label: 'Open subject dossier…',
      detail: 'Findings, connections and provenance for one person, agency or record',
      group: 'Navigate',
      icon: 'evidence',
      keywords: 'dossier subject entity person agency who look up investigate',
      keepOpen: true,
      perform: () => {
        setMode('dossier');
        setQuery('');
        window.requestAnimationFrame(() => inputRef.current?.focus());
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
      label: 'Open help',
      detail: 'Search the documentation for views, findings, and data handling',
      group: 'Help',
      icon: 'search',
      keywords: 'help documentation manual guide how to learn support faq privacy data',
      shortcut: `${modifier}⇧/`,
      // The palette restores focus to its opener on the next frame. Opening Help after that frame
      // lets the dialog keep the focus it takes on mount.
      perform: () => window.requestAnimationFrame(() => openHelp()),
    },
    ...themeCommandRegistry.map((command) => ({
      id: command.id,
      label: command.label,
      detail: command.detail,
      group: 'Appearance' as const,
      icon: command.icon,
      keywords: command.keywords,
      current: theme === command.theme,
      perform: () => setTheme(command.theme),
    })),
    {
      id: 'toggle-audio-alerts',
      label: audioAlertsEnabled ? 'Turn off audio alerts' : 'Turn on audio alerts',
      detail: 'A sound when a collection finishes or fails',
      group: 'Appearance',
      icon: 'volume',
      keywords: 'audio sound alerts notifications chime mute',
      perform: () => setAudioAlertsEnabled(!audioAlertsEnabled),
    },
    ...motionCommandRegistry.map((command) => ({
      id: command.id,
      label: command.label,
      detail: command.detail,
      group: 'Accessibility' as const,
      icon: command.icon,
      keywords: command.keywords,
      current: motion === command.motion,
      perform: () => setMotion(command.motion),
    })),
    ...contrastCommandRegistry.map((command) => ({
      id: command.id,
      label: command.label,
      detail: command.detail,
      group: 'Accessibility' as const,
      icon: command.icon,
      keywords: command.keywords,
      current: contrast === command.contrast,
      perform: () => setContrast(command.contrast),
    })),
  ], [audioAlertsEnabled, contrast, modifier, motion, setAudioAlertsEnabled, setContrast, setEvidenceMode, setMotion, setTheme, setWorkspaceView, theme, toggleSidebarCollapsed]);

  const openDossier = (value: string) => {
    const trimmed = value.trim();
    if (!trimmed) return;
    closePalette();
    openSubjectDossier({ kind: 'name', value: trimmed });
    setWorkspaceView('overview');
  };

  const objectResults = useObjectSearch(isOpen && mode === 'commands' ? query : '');
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

  // Read each time the palette opens; a read still in flight when it closes is discarded.
  const findingsRefreshRequests = useAppStore((state) => state.findingsRefreshRequests);
  const openLeadDossier = useAppStore((state) => state.openLeadDossier);
  const storyLeads = useGatedLoad(isOpen ? `leads:${findingsRefreshRequests}` : 'closed', () => (isOpen ? invokeCommand('list_story_leads', { limit: PALETTE_LEADS }) : Promise.resolve([])));
  const leadActions = useMemo<CommandAction[]>(() => {
    if (storyLeads.status !== 'ready') return [];
    const scaleMax = storyLeads.data[0]?.publicInterest.totalBasisPoints ?? 1;
    return storyLeads.data.slice(0, PALETTE_LEADS).map((lead, index) => ({
      id: `lead-${lead.findingId}`,
      label: lead.headline,
      detail: '',
      group: 'Leads' as const,
      icon: 'flag' as const,
      keywords: '',
      lead: { rank: index + 1, lead, scaleMax },
      perform: () => openLeadDossier(lead.findingId),
    }));
  }, [openLeadDossier, storyLeads]);

  const filteredActions = useMemo(() => {
    if (mode === 'dossier') return [];
    const normalized = query.trim().toLocaleLowerCase();
    if (!normalized) return [...leadActions, ...actions.filter((action) => !SETTING_GROUPS.has(action.group))];
    const matches = groupContiguously(actions
      .map((action) => ({ action, score: scoreCommand(normalized, action.label, `${action.detail} ${action.group}`, action.keywords) }))
      .filter((entry): entry is { action: CommandAction; score: number } => entry.score !== null)
      .sort((left, right) => right.score - left.score || left.action.label.localeCompare(right.action.label))
      .map(({ action }) => action));
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
  }, [actions, leadActions, mode, objectActions, query]);

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
        else if (mode === 'dossier') {
          setMode('commands');
          setQuery('');
        } else closePalette();
      } else if (mode === 'dossier') {
        if (event.key === 'Enter') {
          event.preventDefault();
          openDossier(query);
        }
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
  }, [answer, askError, asking, filteredActions, isOpen, mode, query, selectedIndex, setWorkspaceView]);

  useEffect(() => {
    document.getElementById(`command-${filteredActions[selectedIndex]?.id}`)?.scrollIntoView({ block: 'nearest' });
  }, [filteredActions, selectedIndex]);

  if (!isOpen) return null;

  const answering = asking || askError !== null || answer !== null;
  const groups = groupRuns(filteredActions);

  // The input owns focus and drives the options through aria-activedescendant, so options are not
  // Tab stops. Tab cycles only between the input and any buttons an answer shows.
  const trapTab = (event: ReactKeyboardEvent<HTMLElement>) => {
    if (event.key !== 'Tab') return;
    event.preventDefault();
    const focusable = Array.from(dialogRef.current?.querySelectorAll<HTMLElement>('input, button:not([tabindex="-1"]):not(:disabled)') ?? []);
    if (focusable.length === 0) return;
    const index = focusable.indexOf(document.activeElement as HTMLElement);
    const next = index < 0 ? 0 : (index + (event.shiftKey ? -1 : 1) + focusable.length) % focusable.length;
    focusable[next].focus();
  };

  return (
    <div className="fixed inset-0 z-palette flex justify-center px-4 pt-[14vh]" role="presentation">
      <div aria-hidden="true" className="motion-fade absolute inset-0 bg-scrim" onMouseDown={closePalette} />
      <section
        ref={dialogRef}
        className="glass motion-pop relative h-fit w-[min(640px,92vw)] overflow-hidden rounded-2xl shadow-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="command-palette-title"
        onKeyDown={trapTab}
      >
        <h2 id="command-palette-title" className="sr-only">Command palette</h2>
        <div className="flex h-16 items-center gap-3 border-b border-line px-5">
          {mode === 'dossier' ? (
            <span className="shrink-0 text-control text-fg-secondary">Dossier</span>
          ) : (
            <Icon name="search" size="md" className="shrink-0 text-muted" />
          )}
          <input
            ref={inputRef}
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder={mode === 'dossier' ? 'Name, email, organization or entity ID' : 'Search findings, records, agencies and commands, or ask a question'}
            aria-label={mode === 'dossier' ? 'Subject for the dossier: name, email, organization or entity ID' : 'Search findings, records, agencies, entities, passages and commands, or ask a question'}
            className="h-full min-w-0 flex-1 border-0 bg-transparent text-headline font-normal tracking-[-0.012em] text-fg outline-none placeholder:text-muted"
            role="combobox"
            aria-expanded={mode === 'commands' && !answering}
            aria-controls="command-results"
            aria-activedescendant={mode === 'commands' && !answering && filteredActions[selectedIndex] ? `command-${filteredActions[selectedIndex].id}` : undefined}
            autoComplete="off"
            spellCheck={false}
          />
        </div>

        {/* The live region stays mounted while the palette is open: an answer that mounts its own
            live region already populated is not announced, so only its contents may change. */}
        <div aria-live="polite">
          {answering && (
            <div id="command-results" className="max-h-[min(60vh,480px)] overflow-y-auto">
              {asking && (
                <Skeleton
                  variant="lines"
                  lines={3}
                  label="Searching findings, the entity graph and document text…"
                  className="px-5 py-4"
                />
              )}
              {askError !== null && (
                <div className="p-3">
                  <InlineError
                    summary="This question couldn't be answered right now."
                    detail={askError}
                    onRetry={() => runIntent(query.trim())}
                  />
                </div>
              )}
              {answer !== null && <IntentAnswerView answer={answer} onChooseOption={chooseOption} onOpenFinding={closePalette} />}
            </div>
          )}
        </div>
        {!answering && mode === 'dossier' && (
          <p id="command-results" className="px-5 py-4 text-body text-muted">
            Type a subject and press Return to open its dossier.
          </p>
        )}
        {!answering && mode === 'commands' && (
          <div id="command-results" className="max-h-[min(60vh,480px)] overflow-y-auto p-2" role="listbox" aria-label="Commands">
            {filteredActions.length === 0 ? (
              <div className="px-4 py-10 text-center" role="presentation">
                <Icon name="search" size="lg" className="mx-auto text-muted" />
                <p className="mt-3 text-heading text-fg-secondary">No matching commands</p>
                <p className="mt-1 text-body text-muted">Try an agency, a finding, a phrase from a document, or a view name.</p>
              </div>
            ) : groups.map(({ group, start, items }) => (
              <div key={`${group}-${start}`} role="group" aria-label={group}>
                <div role="presentation" className="flex items-center gap-3 px-3 pb-1.5 pt-3 text-overline uppercase text-muted">
                  {group === 'Leads' ? 'Top leads' : group}
                  <span aria-hidden="true" className="h-px flex-1 bg-line" />
                </div>
                {items.map((action, offset) => {
                  const index = start + offset;
                  const selected = selectedIndex === index;
                  return (
                    <button
                      key={action.id}
                      id={`command-${action.id}`}
                      type="button"
                      role="option"
                      tabIndex={-1}
                      aria-selected={selected}
                      onMouseEnter={() => setSelectedIndex(index)}
                      onClick={() => {
                        if (!action.keepOpen) closePalette();
                        action.perform();
                      }}
                      className={cn(
                        'flex w-full items-center gap-3 rounded-lg px-3 text-left outline-none transition-colors',
                        action.lead ? 'h-12' : 'h-10',
                        selected ? 'command-row-selected text-fg' : 'text-fg-secondary',
                      )}
                    >
                      {action.lead ? (
                        <>
                          <span aria-hidden="true" className="w-5 shrink-0 text-center text-headline tabular-nums text-muted">{action.lead.rank}</span>
                          <span className="min-w-0 flex-1 truncate text-control text-fg">{action.label}</span>
                          <ScoreSpark
                            width={72}
                            scaleMax={action.lead.scaleMax}
                            label={scoreLabel(action.lead.lead)}
                            factors={orderedFactors(action.lead.lead).map((factor) => ({ key: factor.name, basisPoints: factor.basisPoints, counted: factor.name === COUNTED_FACTOR, unassessed: isUnassessed(action.lead!.lead, factor.name) }))}
                          />
                          <span aria-hidden="true" className="w-5 shrink-0 text-right text-caption font-medium tabular-nums text-fg-secondary">{points(action.lead.lead.publicInterest.totalBasisPoints)}</span>
                        </>
                      ) : (
                        <>
                          <Icon name={action.icon} size="md" className="shrink-0 text-muted" />
                          <span className="flex min-w-0 flex-1 items-baseline gap-2">
                            <span className="shrink-0 text-control text-fg">{action.label}</span>
                            <span className="min-w-0 truncate text-caption text-muted">{action.detail}</span>
                          </span>
                        </>
                      )}
                      {action.current && <span className="shrink-0 text-caption text-muted">Current</span>}
                      {action.shortcut && <Keycap>{action.shortcut}</Keycap>}
                    </button>
                  );
                })}
              </div>
            ))}
          </div>
        )}
        <footer className="flex items-center justify-end gap-4 border-t border-line-strong/70 bg-raised/60 px-4 py-2.5 text-caption text-muted">
          {answering || mode === 'dossier' ? (
            <span className="flex items-center gap-1.5"><Keycap>Esc</Keycap> Back to commands</span>
          ) : (
            <>
              <span className="flex items-center gap-1.5"><Keycap>↑↓</Keycap> Navigate</span>
              <span className="flex items-center gap-1.5"><Keycap>↵</Keycap> Run</span>
              <span className="flex items-center gap-1.5"><Keycap>Esc</Keycap> Close</span>
            </>
          )}
        </footer>
      </section>
    </div>
  );
}

/** Three characters and a space or question mark: a phrase, not a lone view name. */
function isQuestionLike(normalized: string) {
  return normalized.length >= 3 && (/\s/.test(normalized) || normalized.endsWith('?'));
}

/** Keeps the score order but lists each group once, at the position of its best match. */
function groupContiguously(actions: CommandAction[]) {
  const order: CommandAction['group'][] = [];
  const byGroup = new Map<CommandAction['group'], CommandAction[]>();
  for (const action of actions) {
    const members = byGroup.get(action.group);
    if (members) members.push(action);
    else {
      order.push(action.group);
      byGroup.set(action.group, [action]);
    }
  }
  return order.flatMap((group) => byGroup.get(group) ?? []);
}

function groupRuns(actions: CommandAction[]) {
  const runs: { group: CommandAction['group']; start: number; items: CommandAction[] }[] = [];
  actions.forEach((action, index) => {
    const last = runs.at(-1);
    if (last && last.group === action.group) last.items.push(action);
    else runs.push({ group: action.group, start: index, items: [action] });
  });
  return runs;
}

function Keycap({ children }: { children: string }) {
  return (
    <kbd className="keycap inline-flex h-5 min-w-6 shrink-0 items-center justify-center rounded border px-1 font-sans text-caption font-medium tabular-nums text-muted">
      {children}
    </kbd>
  );
}

function isApplePlatform() {
  const platform = navigator.platform.toLocaleLowerCase();
  const userAgent = navigator.userAgent.toLocaleLowerCase();
  return platform.includes('mac') || userAgent.includes('mac os');
}
