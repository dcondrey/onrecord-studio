import { cn } from '../../utils/cn';
import { formatInstant, formatShortDate } from '../objects/labels';

/**
 * A recorded date or instant. The visible text is the short reading; the exact value stays in
 * `dateTime` and the tooltip. A value that does not parse is shown as recorded.
 */
export default function Time({ value, format = formatShortDate, className }: {
  value: string;
  format?: (value: string) => string;
  className?: string;
}) {
  return (
    <time dateTime={value} title={formatInstant(value)} className={cn('tabular-nums', className)}>
      {format(value)}
    </time>
  );
}
