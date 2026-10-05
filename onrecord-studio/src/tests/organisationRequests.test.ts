import { describe, expect, it } from 'vitest';
import type { OrganisationFilings } from '../types';
import { contractFileRequest } from '../utils/organisationRequests';

const filings: OrganisationFilings = { ein: '330215585', name: 'Alpha Project For The Homeless', checkedAt: 't', indexYears: [], returns: [], summary: null, singleAudits: null, warnings: [],
  publicMoney: [{ role: 'payee table', datasetName: 'd', sourceUrl: null, vendorValue: 'A', fiscalYear: null, rows: 14, amount: null }], publicMoneyNotes: [] };

describe('contract file request', () => {
  it('names the custodian and the organisation, keys the gap by both, and falls back to a period when no return was read', () => {
    const input = contractFileRequest(filings, 'San Diego Housing Commission', 'sdhc');
    expect(input.gapKey).toBe('organisation:330215585:contract-file:sdhc');
    expect(input.leadRef).toBe('organisation:330215585');
    expect(input.period).toBe('the last three fiscal years');
    expect(input.gap).toContain('14 payment rows');
    expect(input.draft).toContain('To: San Diego Housing Commission');
    expect(input.draft).toContain('EIN 330215585');
    expect(input.recordTypes[0]).toContain('between San Diego Housing Commission and Alpha Project For The Homeless');
  });
});
