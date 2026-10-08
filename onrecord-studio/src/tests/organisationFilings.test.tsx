// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import type { Form990Statement, OrganisationFilings } from '../types';

const bridge = vi.hoisted(() => ({ invoke: vi.fn(), open: vi.fn() }));
vi.mock('../ipc', async (importOriginal) => ({ ...(await importOriginal<typeof import('../ipc')>()), invokeCommand: bridge.invoke }));
vi.mock('../store/useAppStore', () => ({ useAppStore: Object.assign((selector: (s: unknown) => unknown) => selector({ portals: [{ registration: { portalKey: 'sdhc', displayName: 'San Diego Housing Commission', originUrl: 'https://sdhc.test', canonicalUrl: 'https://sdhc.test/requests' } }] }), { getState: () => ({ openCasework: bridge.open }) }) }));
import OrganisationFilingsPanel from '../components/dashboard/OrganisationFilingsPanel';

const statement = (overrides: Partial<Form990Statement>): Form990Statement => ({
  returnVersion: '2022v5.0', ein: '111111111', filerName: 'A', taxPeriodBegin: '2022-07-01', taxPeriodEnd: '2023-06-30', amended: null,
  totalRevenue: 1000, totalExpenses: null, totalAssetsEndOfYear: 0, totalLiabilitiesEndOfYear: null, netAssetsEndOfYear: null, governmentGrants: null,
  significantDiversionReported: false, formerListed: null, totalReportableCompensation: null, people: [],
  scheduleJAttached: false, severancePaymentReported: null, scheduleJPeople: [], scheduleJExplanations: [],
  scheduleLAttached: false, scheduleLTransactions: [], scheduleLExplanations: [], scheduleOExplanations: [], ...overrides,
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
      if (name === 'list_organisation_checks' || name === 'list_organisation_payment_mappings') return Promise.resolve([]);
      if (name === 'get_analytical_schema') return Promise.resolve({ datasets: [], columns: [], templates: [] });
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
    bridge.invoke.mockImplementation((name: string) => Promise.resolve(
      name === 'list_organisation_checks' || name === 'list_organisation_payment_mappings' ? [] : name === 'get_analytical_schema' ? { datasets: [], columns: [], templates: [] } : record));
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

  it('looks names up, takes only a letter-for-letter match on its own, lets the reader choose the rest, then checks each once', async () => {
    const checked: string[] = [];
    let lists = 0;
    const match = (ein: string, name: string) => ({ ein, name, city: 'San Diego', state: 'CA' });
    bridge.invoke.mockImplementation((name: string, args: { ein?: string; query?: string }) => {
      if (name === 'list_organisation_checks') { lists += 1; return Promise.resolve([]); }
      if (name === 'list_organisation_payment_mappings') return Promise.resolve([]);
      if (name === 'get_analytical_schema') return Promise.resolve({ datasets: [], columns: [], templates: [] });
      if (name === 'list_organisation_payees') return Promise.resolve([{ vendorValue: 'Alpha Project for the Homeless, Inc.', rows: 3, datasetNames: ['pay'] }, { vendorValue: 'Nobody Known', rows: 1, datasetNames: ['pay'] }]);
      if (name === 'search_organisations') return Promise.resolve(args.query === 'Nobody Known' ? [] : [match('330215585', 'Alpha Project For The Homeless'), match('330580463', 'Alpha Project For The Homeless Anaheim Gardens')]);
      if (name === 'check_organisation_filings') { checked.push(args.ein!); return args.ein === '222222222' ? Promise.reject(new Error('index unread')) : Promise.resolve(filings(args.ein!, 'X')); }
      return Promise.resolve(null);
    });
    render(<OrganisationFilingsPanel />);
    fireEvent.change(screen.getByLabelText(/Organisation names or EINs/), { target: { value: '222222222\nALPHA PROJECT FOR THE HOMELESS' } });
    fireEvent.click(screen.getByRole('button', { name: 'Add payees from your payment tables' }));
    await screen.findByRole('button', { name: 'Look up 3 names' });
    expect(screen.getByRole('button', { name: 'Check 1 organisation' })).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'Look up 3 names' }));
    const choice = await screen.findByLabelText('Match for Alpha Project for the Homeless, Inc.');
    expect(screen.getByText('no match in ProPublica Nonprofit Explorer')).toBeTruthy();
    expect(screen.getByText(/→ Alpha Project For The Homeless, EIN 330215585/)).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Check 2 organisations' })).toBeTruthy();
    fireEvent.change(choice, { target: { value: '330580463' } });
    fireEvent.click(screen.getByRole('button', { name: 'Check 3 organisations' }));
    await screen.findByText(/3 of 3 checked, 1 failed/);
    expect(checked).toEqual(['222222222', '330215585', '330580463']);
    expect(screen.getByText(/222222222: index unread/)).toBeTruthy();
    expect(lists).toBe(2);
  });

  it('shows what the library holds on the organisation and tracks the contract-file request for a chosen agency', async () => {
    const record = filings('111111111', 'First Organisation');
    let trackedInput: { gapKey: string; custodian: string } | undefined;
    bridge.invoke.mockImplementation((name: string, args: { input?: { gapKey: string; custodian: string } }) => {
      if (name === 'list_organisation_checks' || name === 'list_organisation_payment_mappings') return Promise.resolve([]);
      if (name === 'get_analytical_schema') return Promise.resolve({ datasets: [], columns: [], templates: [] });
      if (name === 'hybrid_search') return Promise.resolve([{ stableId: 'd1', portalKey: 'sdhc', requestId: 1, documentId: 2, fileName: 'contract.pdf', title: 'Services agreement', agencyName: 'San Diego Housing Commission', jurisdiction: 'CA', documentType: null, publicationDate: '2025-03-01', observationDate: null, effectiveDate: null, characterStart: 0, characterEnd: 10, excerpt: 'First Organisation shall provide outreach.' }]);
      if (name === 'query_audit_records_page') return Promise.reject(new Error('no request index'));
      if (name === 'get_record_gap_requests') return Promise.resolve(trackedInput ? [{ gapKey: trackedInput.gapKey, caseId: 'c', reportId: 'lead:x', requestId: 'r', state: 'DRAFT', submittedAt: null, determinationDueOn: null, deadlinePassedWithoutRecordedResponse: false, productions: 0, remainingOmissions: 0 }] : []);
      if (name === 'request_record_gap') { trackedInput = args.input; return Promise.resolve({}); }
      if (name === 'start_data_source') return Promise.resolve({});
      return Promise.resolve(record);
    });
    render(<OrganisationFilingsPanel />);
    await open('111111111');
    expect(await screen.findByText('Services agreement')).toBeTruthy();
    expect(screen.getByText('First Organisation shall provide outreach.')).toBeTruthy();
    expect(screen.getByText(/Request search: no request index/)).toBeTruthy();
    fireEvent.change(screen.getByLabelText('Agency'), { target: { value: 'sdhc' } });
    fireEvent.click(await screen.findByRole('button', { name: 'Search San Diego Housing Commission for this name' }));
    await screen.findByText(/Collection started from San Diego Housing Commission for "First Organisation"/);
    const [, collection] = bridge.invoke.mock.calls.find(([command]) => command === 'start_data_source')!;
    expect(collection).toMatchObject({ url: 'https://sdhc.test', guidance: { subject: 'First Organisation' } });
    fireEvent.click(await screen.findByRole('button', { name: 'Track this request' }));
    expect(trackedInput).toMatchObject({ gapKey: 'organisation:111111111:contract-file:sdhc', custodian: 'San Diego Housing Commission' });
    fireEvent.click(await screen.findByRole('button', { name: 'Open casework' }));
    expect(bridge.open).toHaveBeenCalledWith('lead:x');
  });

  it('looks a vendor value handed over from another view up as soon as it opens', async () => {
    bridge.invoke.mockImplementation((name: string) => Promise.resolve(
      name === 'search_organisations' ? [{ ein: '330215585', name: 'Alpha Project For The Homeless', city: 'San Diego', state: 'CA' }]
        : name === 'get_analytical_schema' ? { datasets: [], columns: [], templates: [] } : []));
    render(<OrganisationFilingsPanel initialQuery="ALPHA PROJECT FOR THE HOMELESS INC" />);
    expect(await screen.findByRole('button', { name: 'Alpha Project For The Homeless' })).toBeTruthy();
    expect(bridge.invoke).toHaveBeenCalledWith('search_organisations', { query: 'ALPHA PROJECT FOR THE HOMELESS INC' });
  });
});
