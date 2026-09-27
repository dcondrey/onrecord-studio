import { describe, expect, it } from 'vitest';
import { financialAmount, rankFinancialLines, suggestFinancialMapping } from '../utils/financial';
import type { FinancialDataset, FinancialReconciliation } from '../types/financial';
describe('exact financial presentation',()=>{
  it('preserves amounts beyond JavaScript numeric precision and distinguishes missing from zero',()=>{
    expect(financialAmount('90071992547409.01','USD')).toBe('USD 90,071,992,547,409.01');
    expect(financialAmount(null)).toBe('Unknown');expect(financialAmount('0.00')).toBe('0.00');expect(financialAmount('-0.0012')).toBe('-0.0012');
    const line=(key:string,value:string)=>({key,actualMinusAdopted:value,actual:null} as FinancialReconciliation);
    expect(rankFinancialLines([line('low','90071992547409.01'),line('high','90071992547409.02')]).map(r=>r.key)).toEqual(['high','low']);
  });
  it('suggests only columns actually present and never invents a currency or missing identity',()=>{
    const dataset={snapshotId:'payments',columns:['amount','invoice_number','contract_number']} as FinancialDataset;
    const result=suggestFinancialMapping('payment',dataset,['amount','invoice_id','contract_id','transaction_id','currency']);
    expect(result.columns).toEqual({amount:'amount',invoice_id:'invoice_number',contract_id:'contract_number'});
    expect(result.currency).toBeNull();
  });
});
