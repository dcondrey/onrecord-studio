import { useMemo } from 'react';
import { sinceInstant, useAppStore, type SincePreset } from '../../store/useAppStore';
import { formatShortDate } from '../objects/labels';
import Icon from '../ui/icons';

const LABELS: Record<Exclude<SincePreset, 'lastOpened'>, string> = {
  day: 'Last 24 hours',
  week: 'Last 7 days',
  month: 'Last 30 days',
  all: 'Any time',
};

/** Chooses how far back "new" reaches for the attention feed and the story desk. */
export default function SinceControl() {
  const preset = useAppStore((state) => state.sincePreset);
  const previousOpenAt = useAppStore((state) => state.previousOpenAt);
  const setSincePreset = useAppStore((state) => state.setSincePreset);
  const lastOpenedLabel = previousOpenAt ? `Since last opened (${formatShortDate(previousOpenAt)})` : 'Since last opened (first launch: any time)';
  return (
    <label className="secondary-action focus-within:ring-1 focus-within:ring-accent/60 flex items-center gap-1.5 rounded-lg border px-2.5 py-1.5 text-label text-muted">
      <Icon name="clock" className="h-3.5 w-3.5" />
      <span className="sr-only">Show what was found</span>
      <select
        value={preset}
        onChange={(event) => setSincePreset(event.target.value as SincePreset)}
        className="bg-transparent font-medium text-fg outline-none"
      >
        <option value="lastOpened">{lastOpenedLabel}</option>
        {(Object.keys(LABELS) as (keyof typeof LABELS)[]).map((key) => <option key={key} value={key}>{LABELS[key]}</option>)}
      </select>
    </label>
  );
}

/** Fixed when the preset is chosen, so a relative window does not slide (and refetch) on every render. */
export function useSinceInstant(): string | null {
  const preset = useAppStore((state) => state.sincePreset);
  const previousOpenAt = useAppStore((state) => state.previousOpenAt);
  return useMemo(() => sinceInstant(preset, previousOpenAt), [preset, previousOpenAt]);
}
