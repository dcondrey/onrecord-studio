// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import type { Form990Statement, OrganisationFilings } from '../types';

const bridge = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('../ipc', () => ({ invokeCommand: bridge.invoke }));
import OrganisationFilingsPanel from '../components/dashboard/OrganisationFilingsPanel';

const statement = (overrides: Partial<Form990Statement>): Form990Statement => ({
  returnVersion: '2022v5.0', ein: '111111111', filerName: 'A', taxPeriodBegin: '2022-07-01', taxPeriodEnd: '2023-06-30', amended: null,
  totalRevenue: 1000, totalExpenses: null, totalAssetsEndOfYear: 0, totalLiabilitiesEndOfYear: null, netAssetsEndOfYear: null, governmentGrants: null,
  significantDiversionReported: false, formerListed: null, totalReportableCompensation: null, people: [],
  scheduleJAttached: false, severancePaymentReported: null, scheduleJPeople: [], scheduleJExplanations: [],
  scheduleLAttached: false, scheduleLTransactions: [], scheduleLExplanations: [], ...overrides,
});
const filings = (ein: string, name: string, overrides: Partial<OrganisationFilings> = {}): OrganisationFilings => ({
  ein, name, checkedAt: '2026-10-05T00:00:00Z', indexYears: [{ year: 2024, checkedAt: '2026-10-05T00:00:00Z', returnsListed: 0 }], returns: [],
  summary: null, singleAudits: null, warnings: [], publicMoney: [], publicMoneyNotes: [], ...overrides,
});
const open = async (text: string) => {
  fireEvent.change(screen.getByLabelText('Organisation name or EIN'), { target: { value: text } });
  fireEvent.click(screen.getByRole('button', { name: 'Find' }));
};
afterEach(() => { cleanup(); vi.clearAllMocks(); });

describe('organisation filings', () => {
  it('discards a check that resolves after another organisation was opened', async () => {
    let resolveCheck: (value: OrganisationFilings) => void = () => {};
    bridge.invoke.mockImplementation((name: string, args: { ein?: string }) => {
      if (name === 'list_organisation_checks') return Promise.resolve([]);
      if (name === 'check_organisation_filings') return new Promise((resolve) => { resolveCheck = resolve; });
      return Promise.resolve(args.ein === '222222222' ? filings('222222222', 'Second Organisation') : null);
    });
    render(<OrganisationFilingsPanel />);
    await open('11-1111111');
    fireEvent.click(await screen.findByRole('button', { name: 'Check filings' }));
    await open('222222222');
    await screen.findByRole('heading', { name: 'Second Organisation' });
    resolveCheck(filings('111111111', 'First Organisation'));
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(screen.getByRole('heading', { name: 'Second Organisation' })).toBeTruthy();
    expect(screen.queryByText('First Organisation')).toBeNull();
    expect(screen.getByRole('button', { name: 'Check again' }).hasAttribute('disabled')).toBe(false);
  });

  it('keeps an absent answer, an unread return and a lookup that never ran apart from No, zero and none', async () => {
    const record = filings('111111111', 'First Organisation', { returns: [
      { objectId: '1', indexYear: 2024, returnType: '990', taxPeriod: '202306', taxpayerName: 'FIRST', archiveUrl: null, memberName: null, xmlSha256: null, retrievedAt: null, unreadReason: null,
        statement: statement({ formerListed: null, significantDiversionReported: false }) },
      { objectId: '2', indexYear: 2024, returnType: '990EZ', taxPeriod: '202206', taxpayerName: 'FIRST', archiveUrl: null, memberName: null, xmlSha256: null, retrievedAt: null, statement: null, unreadReason: 'This return type is not read; only Form 990 is.' },
    ] });
    bridge.invoke.mockImplementation((name: string) => Promise.resolve(name === 'list_organisation_checks' ? [] : record));
    render(<OrganisationFilingsPanel />);
    await open('111111111');
    const read = within(await screen.findByRole('article', { name: 'Form 990, tax period 2023-06' }));
    expect(read.getByText(/significant diversion/).parentElement?.textContent).toContain('The return answers "No".');
    expect(read.getByText(/Part VII line 3/).parentElement?.textContent).toContain('The return carries no answer.');
    expect(read.getByText('Total expenses').nextElementSibling?.textContent).toBe('not stated');
    expect(read.getByText('Total assets, end of year').nextElementSibling?.textContent).toBe('$0');
    expect(read.getByText(/No Schedule J is attached/)).toBeTruthy();
    expect(within(screen.getByRole('article', { name: 'Form 990EZ, tax period 2022-06' })).getByText(/Not read: This return type is not read/)).toBeTruthy();
    expect(screen.getByText(/No single-audit lookup has run/)).toBeTruthy();
  });
});
