/** Letters and digits only, lower case, single spaces: "Alpha Project, Inc." and "ALPHA PROJECT INC" compare equal. */
export const normalisedName = (value: string) => value.toLowerCase().replace(/[^a-z0-9]+/g, ' ').trim();

export const isEin = (value: string) => /^\d{2}-?\d{7}$/.test(value.trim());

/** Lines (or comma/semicolon-separated items) of a pasted list, split into EINs and names, deduplicated by their normalised form. */
export function parseOrganisationList(text: string): { eins: string[]; names: string[] } {
  const eins: string[] = [];
  const names: string[] = [];
  const seen = new Set<string>();
  for (const raw of text.split(/[\n;]+/).flatMap((line) => isEin(line) ? [line] : line.split(/,(?=\s*\d{2}-?\d{7}\s*(?:,|$))|\n/))) {
    const item = raw.trim();
    if (!item) continue;
    const key = isEin(item) ? item.replace('-', '') : normalisedName(item);
    if (!key || seen.has(key)) continue;
    seen.add(key);
    if (isEin(item)) eins.push(key); else names.push(item);
  }
  return { eins, names };
}
