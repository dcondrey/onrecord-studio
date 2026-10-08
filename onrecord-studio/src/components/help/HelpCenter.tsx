import { useEffect, useMemo, useRef, useState } from 'react';
import { HELP_GROUP_ORDER, HELP_TOPICS, findHelpTopic, type HelpTopic } from '../../content/help';
import { cn } from '../../utils/cn';
import Button from '../ui/Button';
import Dialog, { DialogHeader } from '../ui/Dialog';
import { SearchField } from '../ui/Field';
import { isApplePlatform } from '../dashboard/NavigationControls';
import { EvidenceLadderFigure, ProvenanceLadderFigure } from './HelpFigures';
import { OPEN_HELP_EVENT, type OpenHelpDetail } from './helpEvents';
import { HELP_SHOTS } from './shots';
import { buildSearchIndex, excerptFor, searchTopics } from './search';
import { openExternalUrl } from '../../utils/openExternal';

/** Public support entry point: issues and questions are filed there, not emailed. */
export const SUPPORT_URL = 'https://github.com/writerslogic/support';

const FIGURES: Record<string, () => JSX.Element> = {
  'evidence-ladder': EvidenceLadderFigure,
  'provenance-ladder': ProvenanceLadderFigure,
};

/** Mounts once. Opens on the `onrecord:open-help` event, optionally on one topic. */
export default function HelpCenter() {
  const [open, setOpen] = useState(false);
  const [initialTopic, setInitialTopic] = useState<string | null>(null);

  useEffect(() => {
    const handler = (event: Event) => {
      const detail = (event as CustomEvent<OpenHelpDetail | undefined>).detail;
      setInitialTopic(detail?.topicId ?? null);
      setOpen(true);
    };
    window.addEventListener(OPEN_HELP_EVENT, handler);
    return () => window.removeEventListener(OPEN_HELP_EVENT, handler);
  }, []);

  if (!open) return null;
  return <HelpDialog initialTopicId={initialTopic} onClose={() => setOpen(false)} />;
}

export function HelpDialog({ initialTopicId, onClose }: { initialTopicId: string | null; onClose: () => void }) {
  const searchRef = useRef<HTMLInputElement>(null);
  const articleRef = useRef<HTMLElement>(null);
  const index = useMemo(() => buildSearchIndex(HELP_TOPICS), []);
  const [query, setQuery] = useState('');
  const [selectedId, setSelectedId] = useState<string>(
    () => (initialTopicId && findHelpTopic(initialTopicId) ? initialTopicId : HELP_TOPICS[0].id),
  );
  const [supportNotice, setSupportNotice] = useState<string | null>(null);
  const openSupport = async () => {
    const result = await openExternalUrl(SUPPORT_URL);
    setSupportNotice(result.ok ? null : `${result.reason} Open ${SUPPORT_URL} in your browser.`);
  };

  useEffect(() => {
    // useDialogFocus parks focus on the panel; a reader opening Help usually wants to type.
    if (!initialTopicId) searchRef.current?.focus();
  }, [initialTopicId]);

  const hits = useMemo(() => searchTopics(index, query), [index, query]);
  const searching = query.trim() !== '';
  const selected = findHelpTopic(selectedId) ?? HELP_TOPICS[0];

  const select = (id: string) => {
    setSelectedId(id);
    if (articleRef.current) articleRef.current.scrollTop = 0;
    articleRef.current?.focus();
  };

  const onQueryChange = (value: string) => {
    setQuery(value);
    const first = searchTopics(index, value)[0];
    if (value.trim() !== '' && first) setSelectedId(first.topic.id);
  };

  return (
    <Dialog onClose={onClose} size="xl" className="h-[min(780px,92vh)]">
        <div className="border-b border-line">
          <DialogHeader
            title="OnRecord Studio Help"
            description={
              <>
                <span className="tabular-nums">Version {__APP_VERSION__}</span>
                {isApplePlatform() ? <> · <kbd className="font-sans tabular-nums">⌘⇧/</kbd> opens Help from anywhere</> : null}
                {supportNotice ? <span role="alert" className="mt-1 block text-warning">{supportNotice}</span> : null}
              </>
            }
            actions={<Button onClick={() => void openSupport()} className="focus-ring">Support</Button>}
          />
        </div>

        <div className="grid min-h-0 flex-1 grid-cols-1 md:grid-cols-[300px_minmax(0,1fr)]">
          <div className="flex min-h-0 max-h-[38vh] flex-col border-b border-line md:max-h-none md:border-b-0 md:border-r">
            <div className="p-3">
              <label htmlFor="help-search" className="sr-only">Search help topics</label>
              <SearchField
                id="help-search"
                ref={searchRef}
                aria-label="Search help topics"
                value={query}
                onChange={(event) => onQueryChange(event.target.value)}
                placeholder="Search help"
                autoComplete="off"
                spellCheck={false}
                className="outline-none"
              />
              <p className="mt-2 min-h-4 text-caption tabular-nums text-muted" role="status" aria-live="polite">
                {searching ? `${hits.length} ${hits.length === 1 ? 'topic matches' : 'topics match'}` : ''}
              </p>
            </div>
            <nav aria-label="Help topics" className="min-h-0 flex-1 overflow-y-auto px-2 pb-3">
              {searching ? (
                hits.length === 0 ? (
                  <p className="px-3 py-6 text-label text-fg-secondary">No topic matches. Try a shorter word, or a term such as deadline, provenance or research.</p>
                ) : (
                  <ul className="space-y-1">
                    {hits.map(({ topic }) => (
                      <TopicButton key={topic.id} topic={topic} selected={topic.id === selected.id} onSelect={select} excerpt={excerptFor(topic, query)} />
                    ))}
                  </ul>
                )
              ) : (
                HELP_GROUP_ORDER.map((group) => {
                  const inGroup = HELP_TOPICS.filter((topic) => topic.group === group);
                  if (inGroup.length === 0) return null;
                  return (
                    <section key={group} aria-label={group} className="mb-2">
                      <h3 className="sidebar-section-label px-3 pb-1 pt-2">{group}</h3>
                      <ul className="space-y-0.5">
                        {inGroup.map((topic) => (
                          <TopicButton key={topic.id} topic={topic} selected={topic.id === selected.id} onSelect={select} />
                        ))}
                      </ul>
                    </section>
                  );
                })
              )}
            </nav>
          </div>

          <article
            ref={articleRef}
            tabIndex={-1}
            aria-labelledby="help-topic-title"
            className="focus-ring min-h-0 overflow-y-auto px-5 pb-5 pt-4"
          >
            <TopicView topic={selected} onOpenTopic={select} />
          </article>
        </div>
    </Dialog>
  );
}

function TopicButton({
  topic,
  selected,
  onSelect,
  excerpt = null,
}: {
  topic: HelpTopic;
  selected: boolean;
  onSelect: (id: string) => void;
  excerpt?: string | null;
}) {
  return (
    <li>
      <button
        type="button"
        onClick={() => onSelect(topic.id)}
        aria-current={selected ? 'true' : undefined}
        className={cn(
          'focus-ring block w-full rounded-lg px-3 py-2 text-left transition',
          selected ? 'command-row-selected text-fg' : 'text-fg-secondary hover:bg-raised/70',
        )}
      >
        <span className="block text-label font-medium">{topic.title}</span>
        {excerpt ? <span className="mt-0.5 block text-label leading-snug text-muted">{excerpt}</span> : null}
      </button>
    </li>
  );
}

export function TopicView({ topic, onOpenTopic }: { topic: HelpTopic; onOpenTopic: (id: string) => void }) {
  const Figure = topic.figure ? FIGURES[topic.figure] : undefined;
  const shot = topic.screenshot ? HELP_SHOTS[topic.screenshot] : undefined;
  const related = topic.related.map((id) => findHelpTopic(id)).filter((entry): entry is HelpTopic => entry !== undefined);
  return (
    <>
      <p className="app-eyebrow text-muted">{topic.group}</p>
      <h2 id="help-topic-title" className="mt-1 text-title text-fg">{topic.title}</h2>
      <p className="mt-1 text-body text-fg-secondary">{topic.summary}</p>
      {Figure ? <Figure /> : null}
      {topic.sections.map((section, sectionIndex) => (
        <section key={`${section.heading ?? 'intro'}-${sectionIndex}`} className="mt-4">
          {section.heading ? <h3 className="text-heading text-fg">{section.heading}</h3> : null}
          {section.paragraphs?.map((paragraph) => (
            <p key={paragraph} className="mt-1.5 text-label leading-relaxed text-fg-secondary">{paragraph}</p>
          ))}
          {section.items ? (
            <ul className="mt-1.5 list-disc space-y-1.5 pl-5 text-label leading-relaxed text-fg-secondary marker:text-muted">
              {section.items.map((item) => <li key={item}>{item}</li>)}
            </ul>
          ) : null}
        </section>
      ))}
      {shot ? (
        <figure className="mt-5">
          <img src={shot.src} alt={shot.alt} loading="lazy" className="w-full rounded-lg border border-line-strong" />
          <figcaption className="mt-1.5 text-label text-muted">{shot.caption}</figcaption>
        </figure>
      ) : null}
      {related.length > 0 ? (
        <section className="mt-6 border-t border-line pb-5 pt-3" aria-label="Related topics">
          <h3 className="text-overline uppercase text-muted">Related</h3>
          <ul className="mt-2 flex flex-wrap gap-2">
            {related.map((entry) => (
              <li key={entry.id}>
                <Button size="sm" onClick={() => onOpenTopic(entry.id)} className="focus-ring">
                  {entry.title}
                </Button>
              </li>
            ))}
          </ul>
        </section>
      ) : null}
    </>
  );
}
