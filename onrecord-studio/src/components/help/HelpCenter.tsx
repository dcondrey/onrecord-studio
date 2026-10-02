import { useEffect, useMemo, useRef, useState } from 'react';
import { HELP_GROUP_ORDER, HELP_TOPICS, findHelpTopic, type HelpTopic } from '../../content/help';
import { cn } from '../../utils/cn';
import Icon from '../ui/icons';
import { useDialogFocus } from '../licensing/useDialogFocus';
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
  const { ref, onKeyDown } = useDialogFocus(onClose);
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
    <div
      className="app-backdrop fixed inset-0 z-[115] grid place-items-center p-3 backdrop-blur-md sm:p-6"
      role="presentation"
      onKeyDown={onKeyDown}
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <div
        ref={ref}
        role="dialog"
        aria-modal="true"
        aria-labelledby="help-title"
        tabIndex={-1}
        className="app-panel flex h-[min(780px,92vh)] w-full max-w-5xl flex-col overflow-hidden text-slate-100 outline-none"
      >
        <header className="flex items-center justify-between gap-3 border-b border-slate-800/70 px-4 py-3">
          <div>
            <h2 id="help-title" className="text-[15px] font-semibold text-slate-100">OnRecord Studio Help</h2>
            <p className="text-label text-slate-400">Version {__APP_VERSION__}. Cmd/Ctrl+Shift+/ opens Help from anywhere. Esc closes it.</p>
            {supportNotice ? <p role="alert" className="mt-1 text-label text-amber-200">{supportNotice}</p> : null}
          </div>
          <div className="flex shrink-0 items-center gap-2">
            <button
              type="button"
              onClick={() => void openSupport()}
              className="focus-ring h-9 rounded-xl border border-slate-700 px-3 text-label font-medium text-slate-200 hover:bg-slate-800"
            >
              Support
            </button>
            <button
              type="button"
              onClick={onClose}
              aria-label="Close help"
              className="focus-ring grid h-9 w-9 shrink-0 place-items-center rounded-xl border border-slate-700 text-slate-300 hover:bg-slate-800"
            >
              <Icon name="x" className="h-4 w-4" />
            </button>
          </div>
        </header>

        <div className="grid min-h-0 flex-1 grid-cols-1 md:grid-cols-[300px_minmax(0,1fr)]">
          <div className="flex min-h-0 max-h-[38vh] flex-col border-b border-slate-800/70 md:max-h-none md:border-b-0 md:border-r">
            <div className="p-3">
              <label htmlFor="help-search" className="sr-only">Search help topics</label>
              <div className="flex items-center gap-2 rounded-xl border border-slate-700 bg-slate-950/60 px-3 focus-within:border-cyan-400/60">
                <Icon name="search" className="h-4 w-4 shrink-0 text-slate-400" />
                <input
                  id="help-search"
                  ref={searchRef}
                  type="search"
                  value={query}
                  onChange={(event) => onQueryChange(event.target.value)}
                  placeholder="Search help"
                  autoComplete="off"
                  spellCheck={false}
                  className="focus-ring h-10 min-w-0 flex-1 rounded border-0 bg-transparent text-label text-slate-100 outline-none placeholder:text-slate-400"
                />
              </div>
              <p className="mt-2 text-label text-slate-400" role="status" aria-live="polite">
                {searching ? `${hits.length} ${hits.length === 1 ? 'topic matches' : 'topics match'}` : `${HELP_TOPICS.length} topics`}
              </p>
            </div>
            <nav aria-label="Help topics" className="min-h-0 flex-1 overflow-y-auto px-2 pb-3">
              {searching ? (
                hits.length === 0 ? (
                  <p className="px-3 py-6 text-label text-slate-300">No topic matches. Try a shorter word, or a term such as deadline, provenance or research.</p>
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
            className="min-h-0 overflow-y-auto px-5 py-4 outline-none focus-visible:ring-2 focus-visible:ring-sky-400"
          >
            <TopicView topic={selected} onOpenTopic={select} />
          </article>
        </div>
      </div>
    </div>
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
          selected ? 'command-row-selected text-slate-100' : 'text-slate-300 hover:bg-slate-800/60',
        )}
      >
        <span className="block text-label font-medium">{topic.title}</span>
        {excerpt ? <span className="mt-0.5 block text-label leading-snug text-slate-400">{excerpt}</span> : null}
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
      <p className="app-eyebrow text-slate-400">{topic.group}</p>
      <h2 id="help-topic-title" className="mt-1 text-[22px] font-semibold tracking-[-0.02em] text-slate-100">{topic.title}</h2>
      <p className="mt-1 text-[14px] leading-relaxed text-slate-300">{topic.summary}</p>
      {Figure ? <Figure /> : null}
      {topic.sections.map((section, sectionIndex) => (
        <section key={`${section.heading ?? 'intro'}-${sectionIndex}`} className="mt-4">
          {section.heading ? <h3 className="text-[14px] font-semibold text-slate-100">{section.heading}</h3> : null}
          {section.paragraphs?.map((paragraph) => (
            <p key={paragraph} className="mt-1.5 text-label leading-relaxed text-slate-300">{paragraph}</p>
          ))}
          {section.items ? (
            <ul className="mt-1.5 list-disc space-y-1.5 pl-5 text-label leading-relaxed text-slate-300 marker:text-slate-400">
              {section.items.map((item) => <li key={item}>{item}</li>)}
            </ul>
          ) : null}
        </section>
      ))}
      {shot ? (
        <figure className="mt-5">
          <img src={shot.src} alt={shot.alt} loading="lazy" className="w-full rounded-lg border border-slate-700" />
          <figcaption className="mt-1.5 text-label text-slate-400">{shot.caption}</figcaption>
        </figure>
      ) : null}
      {related.length > 0 ? (
        <section className="mt-6 border-t border-slate-800/70 pt-3" aria-label="Related topics">
          <h3 className="text-label font-semibold uppercase tracking-wider text-slate-400">Related</h3>
          <ul className="mt-2 flex flex-wrap gap-2">
            {related.map((entry) => (
              <li key={entry.id}>
                <button
                  type="button"
                  onClick={() => onOpenTopic(entry.id)}
                  className="focus-ring rounded-lg border border-slate-700 px-2.5 py-1 text-label font-medium text-slate-200 hover:bg-slate-800"
                >
                  {entry.title}
                </button>
              </li>
            ))}
          </ul>
        </section>
      ) : null}
    </>
  );
}
