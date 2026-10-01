/**
 * One CSV field. A leading `=`, `+`, `-`, `@`, tab or carriage return would run as a formula in a
 * spreadsheet, and these values come from document text, so such a field is prefixed with a quote.
 */
export function csvField(value: string | number | null | undefined): string {
  if (value === null || value === undefined) return '';
  const text = String(value);
  const safe = /^[=+\-@\t\r]/.test(text) && Number.isNaN(Number(text)) ? `'${text}` : text;
  return /[",\n\r]/.test(safe) ? `"${safe.replace(/"/g, '""')}"` : safe;
}

export function csvDocument(header: readonly string[], rows: readonly (readonly (string | number | null | undefined)[])[]): string {
  return [header, ...rows].map((row) => row.map(csvField).join(',')).join('\r\n');
}
