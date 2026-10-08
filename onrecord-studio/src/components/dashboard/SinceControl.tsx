import { useMemo } from 'react';
import { sinceInstant, useAppStore, type SincePreset } from '../../store/useAppStore';
import { formatShortDate } from '../objects/labels';
import { Select } from '../ui/Field';

const LABELS: Record<Exclude<SincePreset, 'lastOpened'>, string> = {
  day: 'Last 24 hours',
  week: 'Last 7 days',
  month: 'Last 30 days',
  all: 'Any time',
};

/** Chooses how far back "new" reaches for the attention feed and the story desk. */
export default function SinceControl({ disabled = false }: { disabled?: boolean } = {}) {
  const preset = useAppStore((state) => state.sincePreset);
  const previousOpenAt = useAppStore((state) => state.previousOpenAt);
  const setSincePreset = useAppStore((state) => state.setSincePreset);
  const lastOpenedLabel = previousOpenAt ? `Since last opened · ${formatShortDate(previousOpenAt)}` : 'Since last opened · any time';
  return (
    <Select
      aria-label="Show findings since"
      value={preset}
      onChange={(event) => setSincePreset(event.target.value as SincePreset)}
      disabled={disabled}
      className="w-auto tabular-nums"
    >
      <option value="lastOpened">{lastOpenedLabel}</option>
      {(Object.keys(LABELS) as (keyof typeof LABELS)[]).map((key) => <option key={key} value={key}>{LABELS[key]}</option>)}
    </Select>
  );
}

/** Fixed when the preset is chosen, so a relative window does not slide (and refetch) on every render. */
export function useSinceInstant(): string | null {
  const preset = useAppStore((state) => state.sincePreset);
  const previousOpenAt = useAppStore((state) => state.previousOpenAt);
  return useMemo(() => sinceInstant(preset, previousOpenAt), [preset, previousOpenAt]);
}
