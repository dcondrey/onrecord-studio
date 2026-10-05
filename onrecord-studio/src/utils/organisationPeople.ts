import type { OrganisationFilings, OrganisationSummaryYear } from '../types';
import { normalisedName } from './organisationNames';

/** One person's listing on one read return. */
export interface PersonYear {
  period: string;
  title: string | null;
  positions: string[];
  former: boolean;
  /** Part VII column (D): reportable compensation from the organisation. */
  reportable: number | null;
  other: number | null;
  /** Schedule J Part II total from the filing organisation, when the person is listed there. */
  scheduleJTotal: number | null;
}

export interface PersonAcrossReturns {
  name: string;
  years: PersonYear[];
}

const period = (value: string) => value.length === 6 ? `${value.slice(0, 4)}-${value.slice(4)}` : value;

/** Everyone named in Part VII or Schedule J of any read return, with one entry per return they
 * appear on, newest first. Names are joined on letters and digits only; the newest return's
 * spelling is shown. A person absent from a return has no entry for it: absence is not a zero. */
export function peopleAcrossReturns(filings: OrganisationFilings): PersonAcrossReturns[] {
  const people = new Map<string, PersonAcrossReturns>();
  const returns = [...filings.returns].filter((r) => r.statement).sort((a, b) => b.taxPeriod.localeCompare(a.taxPeriod));
  for (const listed of returns) {
    const statement = listed.statement!;
    const scheduleJ = new Map(statement.scheduleJPeople.filter((p) => p.name).map((p) => [normalisedName(p.name!), p.filingOrganisation.total]));
    const seen = new Set<string>();
    for (const person of statement.people) {
      if (!person.name) continue;
      const key = normalisedName(person.name);
      if (!key || seen.has(key)) continue;
      seen.add(key);
      const entry = people.get(key) ?? { name: person.name, years: [] };
      entry.years.push({ period: period(listed.taxPeriod), title: person.title, positions: person.positions, former: person.former, reportable: person.reportableCompensation, other: person.otherCompensation, scheduleJTotal: scheduleJ.get(key) ?? null });
      people.set(key, entry);
    }
    for (const person of statement.scheduleJPeople) {
      if (!person.name) continue;
      const key = normalisedName(person.name);
      if (!key || seen.has(key)) continue;
      seen.add(key);
      const entry = people.get(key) ?? { name: person.name, years: [] };
      entry.years.push({ period: period(listed.taxPeriod), title: person.title, positions: [], former: false, reportable: null, other: null, scheduleJTotal: person.filingOrganisation.total });
      people.set(key, entry);
    }
  }
  return [...people.values()].sort((a, b) => {
    const paid = (p: PersonAcrossReturns) => Math.max(...p.years.map((y) => y.reportable ?? y.scheduleJTotal ?? -1));
    return paid(b) - paid(a) || a.name.localeCompare(b.name);
  });
}

/** The change from the previous summary year for a stated figure, or null when either year leaves it unstated. */
export function yearChange(years: OrganisationSummaryYear[], index: number, pick: (year: OrganisationSummaryYear) => number | null): number | null {
  const current = pick(years[index]);
  const previous = years[index + 1] ? pick(years[index + 1]) : null;
  return current === null || previous === null ? null : current - previous;
}
