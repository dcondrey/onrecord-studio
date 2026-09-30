import { helpTopicForView } from '../../content/help';
import { useAppStore } from '../../store/useAppStore';
import { openHelp } from './helpEvents';

/** The "?" control in the header. It opens Help on the topic for whichever view is showing. */
export default function HelpButton() {
  const view = useAppStore((state) => state.workspaceView);
  const topic = helpTopicForView(view);
  return (
    <button
      type="button"
      title={`Help: ${topic.title}`}
      aria-label={`Open help for ${topic.title}`}
      onClick={() => openHelp(topic.id)}
      className="focus-ring grid h-9 w-9 place-items-center rounded-xl border border-slate-800 bg-slate-900/50 text-[14px] font-semibold text-slate-300 transition hover:text-slate-100"
    >
      <span aria-hidden="true">?</span>
    </button>
  );
}
