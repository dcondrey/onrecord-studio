import { describe, expect, it } from 'vitest';
import type { Form990Statement, OrganisationFilings, OrganisationReturn } from '../types';
import { portfolioCsv, portfolioRow } from '../utils/organisationPortfolio';

const statement = (overrides: Partial<Form990Statement>): Form990Statement => ({
  returnVersion: null, ein: null, filerName: null, taxPeriodBegin: null, taxPeriodEnd: null, amended: null,
  totalRevenue: null, totalExpenses: null, totalAssetsEndOfYear: null, totalLiabilitiesEndOfYear: null, netAssetsEndOfYear: null, governmentGrants: null,
  significantDiversionReported: null, formerListed: null, totalReportableCompensation: null, people: [],
  scheduleJAttached: false, severancePaymentReported: null, scheduleJPeople: [], scheduleJExplanations: [],
  scheduleLAttached: false, scheduleLTransactions: [], scheduleLExplanations: [], scheduleOExplanations: [], ...overrides,
});
const listed = (taxPeriod: string, returnType: string, s: Form990Statement | null): OrganisationReturn => ({
  objectId: taxPeriod, indexYear: 2024, returnType, taxPeriod, taxpayerName: 'A', archiveUrl: null, memberName: null, xmlSha256: null, retrievedAt: null, statement: s, unreadReason: s ? null : 'not read',
});
const filings = (overrides: Partial<OrganisationFilings>): OrganisationFilings => ({
  ein: '1', name: 'A', checkedAt: 't', indexYears: [], returns: [], summary: null, singleAudits: null, warnings: [], publicMoney: [], publicMoneyNotes: [], ...overrides,
});

describe('portfolio rows', () => {
  it('takes the latest read return and keeps absence apart from No, zero and none', () => {
    const row = portfolioRow(filings({
      returns: [
        listed('202306', '990', statement({ totalRevenue: 10, significantDiversionReported: false, scheduleJAttached: true, severancePaymentReported: null })),
        listed('202406', '990', null),
        listed('202206', '990EZ', null),
      ],
      publicMoney: [
        { role: 'payment', datasetName: 'd', sourceUrl: null, vendorValue: 'A', fiscalYear: '2023', rows: 2, amount: '300.5' },
        { role: 'payment', datasetName: 'd', sourceUrl: null, vendorValue: 'A', fiscalYear: '2024', rows: 1, amount: null },
      ],
    }));
    expect(row.latestPeriod).toBe('2023-06');
    expect(row.totalRevenue).toBe(10);
    expect(row.governmentGrants).toBeNull();
    expect(row.significantDiversion).toBe('No');
    expect(row.formerListed).toBe('no answer');
    expect(row.severance).toBe('no answer');
    expect(row.scheduleLRows).toBe(0);
    expect(row.unread).toEqual(['990 2024-06', '990EZ 2022-06']);
    expect(row.audits).toBeNull();
    expect(row.paymentsTotal).toBeNull();
    expect(row.paymentRows).toBe(3);
  });

  it('reports no Schedule J and sums payments only when every group was summed', () => {
    const row = portfolioRow(filings({
      returns: [listed('202306', '990', statement({ scheduleJAttached: false }))],
      singleAudits: { audits: [
        { reportId: 'r1', auditYear: '2023', fiscalYearEnd: null, acceptedDate: null, auditor: null, federalAmountExpended: null, lines: [], findings: [{ referenceNumber: '1', complianceRequirement: null, lines: [] }] },
        { reportId: 'r2', auditYear: '2021', fiscalYearEnd: null, acceptedDate: null, auditor: null, federalAmountExpended: null, lines: [], findings: [] },
      ], sourceUrl: 'u', retrievedAt: 't' },
      publicMoney: [{ role: 'payee table', datasetName: 'd', sourceUrl: null, vendorValue: 'A', fiscalYear: null, rows: 1, amount: '5.0000' }],
    }));
    expect(row.severance).toBe('no Schedule J');
    expect(row.audits).toBe(2);
    expect(row.latestAuditYear).toBe('2023');
    expect(row.auditFindingRows).toBe(1);
    expect(row.paymentsTotal).toBe(5);
    expect(portfolioRow(filings({})).significantDiversion).toBe('not read');
  });

  it('writes absent values as empty cells and quotes names with commas', () => {
    const csv = portfolioCsv([portfolioRow(filings({ name: 'Alpha Project, Inc.' }))]);
    const [head, line] = csv.split('\r\n');
    expect(head.split(',').length).toBe(16);
    expect(line).toBe('1,"Alpha Project, Inc.",t,,,,not read,not read,not read,,,,,,,0');
  });
});
