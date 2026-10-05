import type { OrganisationFilings, RecordGapRequestInput } from '../types';
import { latestReadReturn } from './organisationPortfolio';

const bullets = (items: string[]) => items.map((item) => `- ${item}`).join('\n');

/** The contract file an agency holds on an organisation it pays: the record gap the filings and
 * payment rows leave open, as a request the reader reviews and files. */
export function contractFileRequest(filings: OrganisationFilings, custodian: string, custodianKey: string): RecordGapRequestInput {
  const name = filings.name ?? `EIN ${filings.ein}`;
  const latest = latestReadReturn(filings.returns)?.statement;
  const period = latest?.taxPeriodBegin && latest.taxPeriodEnd ? `${latest.taxPeriodBegin} to ${latest.taxPeriodEnd} and the two preceding fiscal years` : 'the last three fiscal years';
  const paymentRows = filings.publicMoney.reduce((count, line) => count + line.rows, 0);
  const recordTypes = [
    `Contracts, agreements and amendments between ${custodian} and ${name}`,
    'Invoices, payment records and budget-to-actual reports under those contracts',
    'Monitoring, site-visit and audit reports on the contracts, and corrective-action correspondence',
    'Performance and outcome reports submitted under the contracts',
  ];
  const fields = ['Contract number, term, scope and total authority', 'Payment date, amount and invoice reference', 'Monitoring date, what was found and how it was resolved'];
  const draft = [
    `To: ${custodian}`,
    `This is a request under the California Public Records Act (Government Code § 7920.000 et seq.).`,
    `I request the following public records for ${period}:\n${bullets(recordTypes)}`,
    `Within those records I am asking for:\n${bullets(fields)}`,
    `The organisation is ${name}, EIN ${filings.ein}.`,
    'Please provide records in their native electronic format where one exists. If any part is withheld, please identify the record and the exemption relied on, and release the remainder.',
  ].join('\n\n');
  return {
    gapKey: `organisation:${filings.ein}:contract-file:${custodianKey}`,
    leadRef: `organisation:${filings.ein}`,
    leadTitle: `${name} (EIN ${filings.ein})`,
    leadQuestion: `What has ${custodian} contracted with, paid to, and found in monitoring ${name}?`,
    title: `Contract file for ${name}`,
    custodian,
    gap: `The library holds the organisation's filings${paymentRows ? ` and ${paymentRows} payment rows naming it` : ''}, but no contract, invoice or monitoring record from ${custodian}.`,
    resolves: 'Which services were contracted, what was paid against them, and what monitoring found.',
    draft, recordTypes, fields, period,
  };
}
