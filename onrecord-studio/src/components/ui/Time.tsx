import { cn } from '../../utils/cn';
import { formatInstant, formatShortDate } from '../objects/labels';

const DATE_ONLY = /^\d{4}-\d{2}-\d{2}$/;
// `new Date('2026-10-09')` is UTC midnight; formatting it in the local zone shows the day before
// anywhere west of UTC, so a calendar date is formatted in UTC.
const CALENDAR_DATE = new Intl.DateTimeFormat(undefined, { month: 'short', day: 'numeric', year: 'numeric', timeZone: 'UTC' });

function formatRecorded(value: string): string {
  return DATE_ONLY.test(value) && !Number.isNaN(Date.parse(value)) ? CALENDAR_DATE.format(new Date(value)) : formatShortDate(value);
}

/**
 * A recorded date or instant. The visible text is the short reading; the exact value stays in
 * `dateTime` and the tooltip. A date recorded without a time keeps it that way in the tooltip
 * rather than gaining a midnight and a zone. A value that does not parse is shown as recorded.
 */
export default function Time({ value, format = formatRecorded, className }: {
  value: string;
  format?: (value: string) => string;
  className?: string;
}) {
  return (
    <time dateTime={value} title={DATE_ONLY.test(value) ? value : formatInstant(value)} className={cn('tabular-nums', className)}>
      {format(value)}
    </time>
  );
}
