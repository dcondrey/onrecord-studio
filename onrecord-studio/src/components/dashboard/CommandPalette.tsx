import { useEffect, useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent as ReactKeyboardEvent } from 'react';
import { invokeCommand, formatNativeError } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import { useRankedLeads } from '../../hooks/useRankedLeads';
import { useAppStore } from '../../store/useAppStore';
import type { IntentAnswer, IntentOption, StoryLead } from '../../types';
import { points } from '../../utils/attentionFactors';
import ScoreSpark, { scoreSentence, sparkFactors } from '../charts/figures/ScoreSpark';
import IntentAnswerView from './IntentAnswerView';
import { cn } from '../../utils/cn';
import Icon, { type IconName } from '../ui/icons';
import InlineError from '../ui/InlineError';
import Skeleton from '../ui/Skeleton';
import { openHelp } from '../help/helpEvents';
import { openSourceConnector } from './sourceEvents';
import { OBJECT_ICONS } from '../objects/ObjectLink';
import { useObjectSearch, type ObjectGroup } from './useObjectSearch';
import { useKeepInView } from '../../hooks/useKeepInView';
import { contrastCommandRegistry, motionCommandRegistry, scoreCommand, themeCommandRegistry, workspaceCommandRegistry } from '../../commands/registry';

export const OPEN_COMMAND_PALETTE_EVENT = 'onrecord:open-command-palette';

const INTENT_ROW_LIMIT = 10;
/** Leads listed with an empty query; a query searches every lead the palette holds. */
const LEAD_ROWS_SHOWN = 3;
const ASK_DETAIL = 'Answered from findings, the entity graph and document text. No AI model is used.';

/**
 * Questions the intent router answers with nothing to fill in (analysis/intent.rs), offered before
 * anything is typed. Each runs the same deterministic query as typing it.
 */
export const SUGGESTED_QUESTIONS = ['which agencies changed behavior', 'same denial language across agencies', 'show contradictions'] as const;

/** Opens the palette; with a query, the palette asks it at once, exactly as if it had been typed and run. */
export function openCommandPalette(query?: string) {
  window.dispatchEvent(new CustomEvent(OPEN_COMMAND_PALETTE_EVENT, { detail: query ? { query } : null }));
}

interface CommandAction {
  id: string;
  label: string;
  detail: string;
  group: 'Ask' | 'Leads' | 'Questions' | 'Navigate' | 'Create' | 'Help' | 'Appearance' | 'Accessibility' | 'Search' | ObjectGroup;
  icon: IconName;
  keywords: string;
  shortcut?: string;
  /** The row starts work whose result the palette shows, so choosing it must not close the palette. */
  keepOpen?: boolean;
  /** A setting row whose value is the one in force, or the lead on the stage. */
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
  const [selectedId, setSelectedId] = useState<string | null>(null);
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
  const setOverviewPerspective = useAppStore((state) => state.setOverviewPerspective);
  const openCasework = useAppStore((state) => state.openCasework);
  const requestAnalysisPanel = useAppStore((state) => state.requestAnalysisPanel);
  const setSearchFilters = useAppStore((state) => state.setSearchFilters);
  const theme = useAppStore((state) => state.theme);
  const motion = useAppStore((state) => state.motion);
  const contrast = useAppStore((state) => state.contrast);
  const audioAlertsEnabled = useAppStore((state) => state.audioAlertsEnabled);
  const workbenchQueue = useAppStore((state) => state.workbenchQueue);
  const selectWorkbenchLead = useAppStore((state) => state.selectWorkbenchLead);
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
  // With no workbench on screen, the palette reads the same ranking the workbenches queue.
  const topLeads = useRankedLeads(isOpen && !workbenchQueue);

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

  /** Asks `text` through the query change, so whatever was asked about the previous text is dropped first. */
  const ask = (text: string) => {
    if (text === query) runIntent(text.trim());
    else {
      askOnQuery.current = text;
      setQuery(text);
    }
  };

  const chooseOption = (option: IntentOption) => ask(option.example);

  const openPalette = () => {
    previousFocus.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    clearAnswer();
    setMode('commands');
    setQuery('');
    setSelectedId(null);
    setOpen(true);
  };

  const closePalette = () => {
    clearAnswer();
    setMode('commands');
    setOpen(false);
    window.requestAnimationFrame(() => previousFocus.current?.focus());
  };

  // Every lead the palette can list, in rank order: the visible workbench's queue, or the top of the
  // ranking when no workbench is on screen. Choosing one puts it on a workbench's stage.
  const leadRows = useMemo<CommandAction[]>(() => {
    if (workbenchQueue) {
      const { surface, leads } = workbenchQueue;
      const scaleMax = leads[0]?.lead.publicInterest.totalBasisPoints ?? 1;
      return leads.map((queued) => ({
        id: `lead-${queued.id}`,
        label: queued.headline,
        detail: queued.kind,
        group: 'Leads' as const,
        icon: 'flag' as const,
        keywords: `lead ${queued.kind} ${queued.rank}`,
        current: queued.id === workbenchQueue.current,
        lead: { rank: queued.rank, lead: queued.lead, scaleMax },
        perform: () => {
          selectWorkbenchLead(surface, queued.id);
          setWorkspaceView(surface);
        },
      }));
    }
    const top = topLeads.leads.slice(0, LEAD_ROWS_SHOWN);
    const scaleMax = top[0]?.publicInterest.totalBasisPoints ?? 1;
    return top.map((lead, index) => ({
      id: `lead-${lead.findingId}`,
      label: lead.headline,
      detail: '',
      group: 'Leads' as const,
      icon: 'flag' as const,
      keywords: 'lead',
      lead: { rank: index + 1, lead, scaleMax },
      perform: () => {
        setWorkspaceView('overview');
        setOverviewPerspective('leads');
        selectWorkbenchLead('overview', lead.findingId);
      },
    }));
  }, [selectWorkbenchLead, setOverviewPerspective, setWorkspaceView, topLeads.leads, workbenchQueue]);

  // Next and previous along the visible queue, named by the lead each one lands on.
  const stepRows = useMemo<CommandAction[]>(() => {
    if (!workbenchQueue || workbenchQueue.leads.length === 0) return [];
    const { leads } = workbenchQueue;
    // -1 when the stage shows something outside the queue (the deadline ledger): Next is lead 1.
    const at = leads.findIndex((lead) => lead.id === workbenchQueue.current);
    const step = (delta: 1 | -1, label: string, key: string, icon: IconName): CommandAction | null => {
      const target = leads[at + delta];
      return target ? {
        id: `lead-step-${key}`,
        label,
        detail: target.headline,
        group: 'Leads',
        icon,
        keywords: `${label} lead queue step`,
        shortcut: key.toUpperCase(),
        perform: () => useAppStore.getState().stepWorkbenchLead(delta),
      } : null;
    };
    return [step(1, 'Next lead', 'j', 'forward'), step(-1, 'Previous lead', 'k', 'back')].filter((action): action is CommandAction => action !== null);
  }, [workbenchQueue]);

  const questionRows = useMemo<CommandAction[]>(() => SUGGESTED_QUESTIONS.map((question, index) => ({
    id: `question-${index}`,
    label: `Ask: ${question}`,
    detail: '',
    group: 'Questions' as const,
    icon: 'search' as const,
    keywords: question,
    keepOpen: true,
    perform: () => ask(question),
  // `ask` closes over the query and stable setters; the rows are rebuilt as the query changes.
  })), [query]);

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
      id: 'overview-following',
      label: 'Overview: Following',
      detail: 'Watchlists and standing searches',
      group: 'Navigate',
      icon: 'activity',
      keywords: 'following watchlist watch standing search alerts',
      perform: () => {
        setWorkspaceView('overview');
        setOverviewPerspective('following');
      },
    },
    {
      id: 'overview-analytics',
      label: 'Overview: Analytics',
      detail: 'Response times, the deadline ledger and period comparison',
      group: 'Navigate',
      icon: 'bars',
      keywords: 'analytics response time deadline ledger statutory misses period comparison',
      perform: () => {
        setWorkspaceView('overview');
        setOverviewPerspective('analytics');
      },
    },
    {
      id: 'overview-casework',
      label: 'Overview: Casework',
      detail: 'Cases, spending, coverage, evidence review and organization filings',
      group: 'Navigate',
      icon: 'briefcase',
      keywords: 'casework case cases investigation track requests commitments',
      perform: () => openCasework(''),
    },
    {
      id: 'casework-spending',
      label: 'Casework: Spending',
      detail: 'Reconcile payments, contracts and funds',
      group: 'Navigate',
      icon: 'briefcase',
      keywords: 'spending payments vendors contracts funds payee lease lessee money',
      perform: () => openCasework('', 'Spending'),
    },
    {
      id: 'casework-filings',
      label: 'Casework: Organization filings',
      detail: 'An organization’s public filings and the payments it received',
      group: 'Navigate',
      icon: 'building',
      keywords: 'organization filings nonprofit 990 irs payee operator vendor',
      perform: () => openCasework('', 'Organization filings'),
    },
    {
      id: 'analysis-budget',
      label: 'Analysis: Budget versus actuals',
      detail: 'Adopted budget against actual spending for a fiscal year',
      group: 'Navigate',
      icon: 'bars',
      keywords: 'budget actuals adopted spending fiscal year compare department fund',
      perform: () => {
        setWorkspaceView('analysis');
        requestAnalysisPanel('budget');
      },
    },
    {
      id: 'analysis-pack',
      label: 'Analysis: Import a data pack',
      detail: 'Download published tables into the analysis library',
      group: 'Navigate',
      icon: 'database',
      keywords: 'pack data pack import download published tables dataset',
      perform: () => {
        setWorkspaceView('analysis');
        requestAnalysisPanel('pack');
      },
    },
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
  ], [audioAlertsEnabled, contrast, modifier, motion, openCasework, requestAnalysisPanel, setAudioAlertsEnabled, setContrast, setEvidenceMode, setMotion, setOverviewPerspective, setTheme, setWorkspaceView, theme, toggleSidebarCollapsed]);

  // A stored finding the visible workbench queues opens on its stage, where J and K continue from it.
  const stageFinding = (id: string) => {
    if (!workbenchQueue?.leads.some((lead) => lead.id === id)) return false;
    selectWorkbenchLead(workbenchQueue.surface, id);
    setWorkspaceView(workbenchQueue.surface);
    return true;
  };

  const openDossier = (value: string) => {
    const trimmed = value.trim();
    if (!trimmed) return;
    closePalette();
    openSubjectDossier({ kind: 'name', value: trimmed });
    setWorkspaceView('overview');
  };

  const objectSearch = useObjectSearch(isOpen && mode === 'commands' ? query : '');
  const inspect = useAppStore((state) => state.inspect);
  const objectActions = useMemo<CommandAction[]>(() => objectSearch.results.map((result) => ({
    id: `object-${result.key}`,
    label: result.label,
    detail: result.detail,
    group: result.group,
    icon: OBJECT_ICONS[result.target.kind],
    keywords: '',
    perform: () => inspect(result.target, 'view'),
  })), [inspect, objectSearch.results]);

  const trimmedQuery = query.trim();
  const normalized = trimmedQuery.toLocaleLowerCase();
  const searching = normalized !== '' && objectSearch.pending;

  const { filteredActions, noMatches } = useMemo(() => {
    if (mode === 'dossier') return { filteredActions: [], noMatches: false };
    if (!normalized) {
      // The lead on the stage is already shown; list the few after it.
      const at = leadRows.findIndex((row) => row.current);
      const upcoming = leadRows.slice(at + 1, at + 1 + LEAD_ROWS_SHOWN);
      return { filteredActions: [...stepRows, ...upcoming, ...questionRows, ...actions.filter((action) => !SETTING_GROUPS.has(action.group))], noMatches: false };
    }
    const matches = groupContiguously([...stepRows, ...leadRows, ...actions]
      .map((action) => ({ action, score: scoreCommand(normalized, action.label, `${action.detail} ${action.group}`, action.keywords) }))
      .filter((entry): entry is { action: CommandAction; score: number } => entry.score !== null)
      .sort((left, right) => right.score - left.score || left.action.label.localeCompare(right.action.label))
      .map(({ action }) => action));
    const nothing = matches.length === 0 && objectActions.length === 0 && !objectSearch.pending;
    const fallback: CommandAction[] = nothing ? [{
      id: 'search-all-records',
      label: `Search all records for “${trimmedQuery}”`,
      detail: 'Open Evidence with this text as the search',
      group: 'Search',
      icon: 'evidence',
      keywords: '',
      perform: () => {
        setSearchFilters(trimmedQuery, null, null, null);
        setWorkspaceView('evidence');
      },
    }] : [];
    if (!isQuestionLike(normalized)) return { filteredActions: [...matches, ...objectActions, ...fallback], noMatches: nothing };
    const askRow: CommandAction = {
      id: 'ask-intent',
      label: `Ask: ${trimmedQuery}`,
      detail: '',
      group: 'Ask',
      icon: 'search',
      keywords: '',
      keepOpen: true,
      perform: () => runIntent(trimmedQuery),
    };
    // A sentence is a question first; a word or two that also names a view stays a view jump.
    const sentence = normalized.split(/\s+/).length >= 3 || normalized.endsWith('?');
    return {
      filteredActions: sentence ? [askRow, ...matches, ...objectActions, ...fallback] : [...matches, ...objectActions, askRow, ...fallback],
      noMatches: nothing,
    };
    // runIntent closes over the gate and setters only, all stable.
  }, [actions, leadRows, mode, normalized, objectActions, objectSearch.pending, questionRows, setSearchFilters, setWorkspaceView, stepRows, trimmedQuery]);

  // Selection follows the row, not its position, so a group that arrives after the palette opens
  // never moves it. With nothing chosen, an empty query selects the first navigation row (opening
  // the palette and pressing Return still goes to the overview) and a typed query its first row.
  const chosenIndex = selectedId === null ? -1 : filteredActions.findIndex((action) => action.id === selectedId);
  const selectedIndex = chosenIndex >= 0 ? chosenIndex : normalized ? 0 : Math.max(0, filteredActions.findIndex((action) => action.group === 'Navigate'));
  const select = (index: number) => setSelectedId(filteredActions[index]?.id ?? null);

  useEffect(() => {
    const openFromEvent = (event: Event) => {
      const asked = (event as CustomEvent<{ query?: unknown } | null>).detail?.query;
      openPalette();
      if (typeof asked !== 'string' || !asked.trim()) return;
      // openPalette clears the field; setting the same text back changes nothing, so it is asked here.
      setQuery(asked);
      if (asked === query) runIntent(asked.trim());
      else askOnQuery.current = asked;
    };
    window.addEventListener(OPEN_COMMAND_PALETTE_EVENT, openFromEvent);
    return () => window.removeEventListener(OPEN_COMMAND_PALETTE_EVENT, openFromEvent);
  });

  // Focus moves before the browser paints the palette, so a key typed straight after the shortcut
  // lands in the field instead of the page behind it.
  useLayoutEffect(() => {
    if (isOpen) inputRef.current?.focus();
  }, [isOpen]);

  useEffect(() => {
    setSelectedId(null);
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
        select(filteredActions.length ? (selectedIndex + 1) % filteredActions.length : 0);
      } else if (event.key === 'ArrowUp') {
        event.preventDefault();
        select(filteredActions.length ? (selectedIndex - 1 + filteredActions.length) % filteredActions.length : 0);
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

  // Keeps the row Return would run in view, from the moment the list opens: the default selection
  // can sit below the Leads and Questions groups.
  const resultsScroller = useRef<HTMLDivElement>(null);
  const showingList = isOpen && mode === 'commands' && !(asking || askError !== null || answer !== null);
  useKeepInView(resultsScroller, '[role="option"][aria-selected="true"]', [filteredActions, selectedIndex, showingList]);

  if (!isOpen) return null;

  const answering = asking || askError !== null || answer !== null;
  const groups = groupRuns(filteredActions);
  const groupTitle = (group: CommandAction['group']) => (group === 'Leads' && !workbenchQueue ? 'Top leads' : group);

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
        className="motion-pop relative h-fit w-[min(640px,92vw)] overflow-hidden rounded-2xl border border-line-strong bg-panel shadow-dialog"
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
              {answer !== null && <IntentAnswerView answer={answer} onChooseOption={chooseOption} onOpenFinding={closePalette} onStageFinding={stageFinding} />}
            </div>
          )}
        </div>
        {!answering && mode === 'dossier' && (
          <p id="command-results" className="px-5 py-4 text-body text-muted">
            Type a subject and press Return to open its dossier.
          </p>
        )}
        {!answering && mode === 'commands' && (
          <div ref={resultsScroller} className="max-h-[min(60vh,480px)] overflow-y-auto p-2">
            {noMatches && (
              <p className="px-3 pb-1 pt-3 text-heading text-fg-secondary">No matches for “{trimmedQuery}”</p>
            )}
            <div id="command-results" role="listbox" aria-label="Commands" aria-busy={searching || undefined}>
            {groups.map(({ group, start, items }) => (
              <div key={`${group}-${start}`} role="group" aria-label={groupTitle(group)}>
                {group !== 'Search' && (
                  <div role="presentation" className="flex items-center gap-3 px-3 pb-1.5 pt-3 text-overline uppercase text-muted">
                    {groupTitle(group)}
                    {/* Said once in the heading rather than cut short on every question row. */}
                    {(group === 'Questions' || group === 'Ask') && <span className="normal-case tracking-normal text-caption">{ASK_DETAIL}</span>}
                    <span aria-hidden="true" className="h-px flex-1 bg-line" />
                  </div>
                )}
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
                      onMouseEnter={() => setSelectedId(action.id)}
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
                      {action.lead ? <LeadRow action={action} lead={action.lead} /> : (
                        <>
                          <Icon name={action.icon} size="md" className="shrink-0 text-muted" />
                          <span className="flex min-w-0 flex-1 items-baseline gap-2">
                            <span className="min-w-0 max-w-[75%] shrink-0 truncate text-control text-fg">{action.label}</span>
                            <span className="min-w-0 truncate text-caption text-muted">{action.detail}</span>
                          </span>
                          {action.current && <span className="shrink-0 text-caption text-muted">Current</span>}
                          {action.shortcut && <Keycap>{action.shortcut}</Keycap>}
                        </>
                      )}
                    </button>
                  );
                })}
              </div>
            ))}
            </div>
            {searching && (
              <div role="status" className="flex h-10 items-center gap-3 px-3 text-caption text-muted">
                <span aria-hidden="true" className="h-3 w-3 shrink-0 animate-spin rounded-full border-2 border-line-strong border-t-accent" />
                Searching findings, records and passages…
              </div>
            )}
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

/** A ranked lead as a palette row: its rank, its headline, and its score at sparkline scale. */
function LeadRow({ action, lead }: { action: CommandAction; lead: NonNullable<CommandAction['lead']> }) {
  return (
    <>
      <span aria-hidden="true" className="w-6 shrink-0 text-center text-headline tabular-nums text-muted">{lead.rank}</span>
      <span className="min-w-0 flex-1 truncate text-control text-fg">{action.label}</span>
      {action.current && <span className="shrink-0 text-caption text-muted">On stage</span>}
      <ScoreSpark width={72} scaleMax={lead.scaleMax} label={`Rank ${lead.rank}. ${scoreSentence(lead.lead)}`} factors={sparkFactors(lead.lead)} />
      <span aria-hidden="true" className="w-6 shrink-0 text-right text-caption font-medium tabular-nums text-fg-secondary">{points(lead.lead.publicInterest.totalBasisPoints)}</span>
    </>
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
