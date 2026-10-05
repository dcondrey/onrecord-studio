import { describe, expect, it } from 'vitest';
import type { Form990Person, Form990Statement, OrganisationFilings, OrganisationReturn, ScheduleJPerson } from '../types';
import { peopleAcrossReturns, yearChange } from '../utils/organisationPeople';

const person = (name: string, o: Partial<Form990Person> = {}): Form990Person => ({ name, title: 'CEO', positions: ['Officer'], former: false, reportableCompensation: 100, relatedOrganisationCompensation: 0, otherCompensation: 5, ...o });
const j = (name: string, total: number | null): ScheduleJPerson => ({ name, title: 'CEO', filingOrganisation: { base: null, bonus: null, other: null, deferred: null, nontaxableBenefits: null, total, reportedInPriorReturn: null }, relatedOrganisations: { base: null, bonus: null, other: null, deferred: null, nontaxableBenefits: null, total: null, reportedInPriorReturn: null } });
const statement = (people: Form990Person[], scheduleJPeople: ScheduleJPerson[]): Form990Statement => ({
  returnVersion: null, ein: null, filerName: null, taxPeriodBegin: null, taxPeriodEnd: null, amended: null, totalRevenue: null, totalExpenses: null, totalAssetsEndOfYear: null, totalLiabilitiesEndOfYear: null, netAssetsEndOfYear: null, governmentGrants: null,
  significantDiversionReported: null, formerListed: null, totalReportableCompensation: null, people, scheduleJAttached: true, severancePaymentReported: null, scheduleJPeople, scheduleJExplanations: [], scheduleLAttached: false, scheduleLTransactions: [], scheduleLExplanations: [], scheduleOExplanations: [],
});
const listed = (taxPeriod: string, s: Form990Statement | null): OrganisationReturn => ({ objectId: taxPeriod, indexYear: 2024, returnType: '990', taxPeriod, taxpayerName: 'A', archiveUrl: null, memberName: null, xmlSha256: null, retrievedAt: null, statement: s, unreadReason: null });
const filings = (returns: OrganisationReturn[]): OrganisationFilings => ({ ein: '1', name: 'A', checkedAt: 't', indexYears: [], returns, summary: null, singleAudits: null, warnings: [], publicMoney: [], publicMoneyNotes: [] });

describe('people across returns', () => {
  it('joins a person across returns by name, newest first, marks the former year, and leaves an absent year absent', () => {
    const people = peopleAcrossReturns(filings([
      listed('202206', statement([person('Jane Q. Doe', { reportableCompensation: 200 })], [j('JANE Q DOE', 230)])),
      listed('202306', statement([person('JANE Q DOE', { title: 'Former CEO', former: true, positions: ['Former'], reportableCompensation: 150 }), person('New Chief')], [])),
      listed('202106', null),
      listed('202406', statement([person('New Chief', { reportableCompensation: 120 })], [j('Someone Only On J', 90)])),
    ]));
    expect(people.map((p) => p.name)).toEqual(['JANE Q DOE', 'New Chief', 'Someone Only On J']);
    expect(people[0].years.map((y) => [y.period, y.former, y.reportable, y.scheduleJTotal])).toEqual([['2023-06', true, 150, null], ['2022-06', false, 200, 230]]);
    expect(people[1].years.map((y) => y.period)).toEqual(['2024-06', '2023-06']);
    expect(people[2].years[0]).toMatchObject({ period: '2024-06', reportable: null, scheduleJTotal: 90 });
  });

  it('computes a year change only when both years state the figure', () => {
    const year = (taxYear: number, totalRevenue: number | null) => ({ taxYear, taxPeriod: null, formType: null, totalRevenue, totalExpenses: null, totalAssetsEndOfYear: null, totalLiabilitiesEndOfYear: null, officerCompensation: null });
    const years = [year(2023, 50), year(2022, null), year(2021, 30)];
    expect(yearChange(years, 0, (y) => y.totalRevenue)).toBeNull();
    expect(yearChange(years, 1, (y) => y.totalRevenue)).toBeNull();
    expect(yearChange(years, 2, (y) => y.totalRevenue)).toBeNull();
    expect(yearChange([year(2023, 50), year(2022, 30)], 0, (y) => y.totalRevenue)).toBe(20);
  });
});
