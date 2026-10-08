import { helpTopicForView } from '../../content/help';
import { useAppStore } from '../../store/useAppStore';
import { openHelp } from './helpEvents';
import Button from '../ui/Button';

/** The help control in the header. It opens Help on the topic for whichever view is showing. */
export default function HelpButton() {
  const view = useAppStore((state) => state.workspaceView);
  const topic = helpTopicForView(view);
  return (
    <Button
      variant="ghost"
      iconOnly
      icon="help"
      title={`Help: ${topic.title}`}
      aria-label={`Open help for ${topic.title}`}
      onClick={() => openHelp(topic.id)}
    />
  );
}
