/**
 * Turns a `datetime-local` value ("2026-01-05T09:30") into RFC 3339 in the browser's own UTC
 * offset. The statute counts calendar days in the offset a date was recorded in, so converting to
 * UTC here could move an evening filing onto the next day. Returns null for anything unparseable:
 * an unknown date stays unknown.
 */
export function localInputToRfc3339(value: string): string | null {
  const match = /^(\d{4}-\d{2}-\d{2})T(\d{2}:\d{2})(?::(\d{2}))?$/.exec(value.trim());
  if (!match) return null;
  const date = new Date(`${match[1]}T${match[2]}:${match[3] ?? '00'}`);
  if (Number.isNaN(date.getTime())) return null;
  const minutes = -date.getTimezoneOffset();
  const sign = minutes < 0 ? '-' : '+';
  const abs = Math.abs(minutes);
  const offset = `${sign}${String(Math.floor(abs / 60)).padStart(2, '0')}:${String(abs % 60).padStart(2, '0')}`;
  return `${match[1]}T${match[2]}:${match[3] ?? '00'}${offset}`;
}

/** The `datetime-local` value for an RFC 3339 instant, in the offset it was written with. */
export function rfc3339ToLocalInput(value: string | null): string {
  const match = value ? /^(\d{4}-\d{2}-\d{2})T(\d{2}:\d{2})/.exec(value) : null;
  return match ? `${match[1]}T${match[2]}` : '';
}
