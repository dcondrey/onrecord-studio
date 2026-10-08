import Icon from '../../ui/icons';
import { isApplePlatform, Keycap, openCommandPalette, SUGGESTED_QUESTIONS } from '../CommandPalette';

/**
 * The overview's primary entry: an input-shaped trigger for the one command palette, so search,
 * navigation, actions and questions share a single input. The suggestions under it are questions
 * the deterministic intent router answers as written; choosing one asks it in the palette.
 */
export default function CommandBar() {
  const modifier = isApplePlatform() ? '⌘' : 'Ctrl ';
  return (
    <div className="grid gap-2.5">
      <button
        type="button"
        onClick={() => openCommandPalette()}
        aria-keyshortcuts={isApplePlatform() ? 'Meta+K' : 'Control+K'}
        className="command-bar focus-ring group flex h-12 w-full items-center gap-3 px-4 text-left"
      >
        <Icon name="search" size="md" className="shrink-0 text-muted transition-colors group-hover:text-fg-secondary" />
        <span className="min-w-0 flex-1 truncate text-body text-muted">Search records and findings, ask a question, or run a command</span>
        <Keycap>{`${modifier}K`}</Keycap>
      </button>
      <div className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1.5">
        <span className="text-caption text-muted">Ask</span>
        {SUGGESTED_QUESTIONS.map((question) => (
          <button
            key={question}
            type="button"
            onClick={() => openCommandPalette(question)}
            className="focus-ring inline-flex h-6 items-center rounded-full border border-line bg-panel px-2.5 text-caption text-fg-secondary transition-colors duration-fast hover:border-line-strong hover:text-fg"
          >
            {question}
          </button>
        ))}
      </div>
    </div>
  );
}
