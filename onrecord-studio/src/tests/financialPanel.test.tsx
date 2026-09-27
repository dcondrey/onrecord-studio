// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import type { FinancialReport, FinancialScenario, FinancialWorkspace } from '../types/financial';
const bridge=vi.hoisted(()=>({invoke:vi.fn()}));
vi.mock('../ipc',()=>({invokeCommand:bridge.invoke}));
vi.mock('@tauri-apps/api/core',()=>({isTauri:()=>true}));
import FinancialInvestigationsPanel from '../components/dashboard/FinancialInvestigationsPanel';
const workspace:FinancialWorkspace={datasets:[],mappings:[],defaultScope:{fiscalYear:2025,fund:'100000',department:'Police'},roles:[],fields:[],requirements:{},warnings:[],fiscalYears:[2025,2024],funds:['100000','200000'],departments:['Police','Parks']};
const report:FinancialReport={id:'case',createdAt:'2026-09-26',scope:workspace.defaultScope,reconciliation:[{fundingDuration:'unknown',agency:'City of San Diego',key:'funding',fiscalYear:'2025',fund:'100000',account:'5001',accountLabel:'Salaries',department:'Police',center:'001',program:'Shelter',currency:'USD',adopted:'100',revised:'200',actual:'50',encumbered:null,actualMinusAdopted:'-50',uncommittedRevised:null,citations:['source'],limitations:['Encumbrances are unknown.']}],chains:[],findings:[{calculations:[{label:'Observed difference',formula:'50 - 100',result:'-50.00',unit:'USD'}],id:'finding',kind:'budget_variance',title:'Exact funding difference',observation:'Compare the revised authority before alleging excess spending.',citations:['source'],alternatives:['A documented transfer may explain the difference.'],nextTest:'Obtain the dated appropriation bridge.'}],outcomes:[{agency:'City of San Diego',key:'outcome',program:'Shelter',contractId:'C',periodStart:'2024-07-01',periodEnd:'2025-06-30',population:'Adults',metric:'Permanent housing exits',unit:'people',denominatorUnit:'people',currency:'USD',cost:'100',quantity:'10',denominator:'20',costPerUnit:'10',successRatePercent:'50',comparisonGroup:'group',citations:['source'],limitations:[]}],gaps:[{title:'Open encumbrance register',recordsNeeded:'Existing commitments by fund/year/account.',why:'Unspent cash is not uncommitted authority.',citations:[]}],evidence:[{id:'source',role:'budget',snapshotId:'snapshot',sourceUrl:'https://www.sandiego.gov/source.csv',sourceSha256:'a'.repeat(64),queryId:'query',columns:['amount'],row:['100.00']}],queries:{query:'SELECT amount FROM analysis_fixture'},records:[],limitations:['An appropriation is not a payment.']};
const result:FinancialScenario={fundingDuration:'one_time',periodScope:'FY2025 only; one fiscal period',status:'conditional_estimate',targetProgram:'Shelter',availableRevisedBalance:'1000',proposedAmount:'100',currency:'USD',unit:'people',estimatedWholeUnits:'10',lowerUnits:'5',upperUnits:'10',formula:'Exact source formula',assumptions:['Conditional capacity, not transfer authority.'],blockers:[],citations:[],queries:{}};
function deferred<T>(){let resolve!:(v:T)=>void;const promise=new Promise<T>(r=>{resolve=r;});return{promise,resolve};}
beforeEach(()=>{bridge.invoke.mockReset();bridge.invoke.mockImplementation((command:string)=>command==='get_financial_workspace'?Promise.resolve(workspace):Promise.resolve(report));});afterEach(cleanup);
async function mounted(){render(<FinancialInvestigationsPanel/>);await screen.findByRole('button',{name:'Investigate these records'});}
async function run(){fireEvent.click(screen.getByRole('button',{name:'Investigate these records'}));await screen.findByText('Exact funding difference');}
it('shows real cited comparisons, explicit gaps and reproducible source rows',async()=>{
 await mounted();await run();expect(screen.getByText('50 - 100')).toBeTruthy();expect(screen.getByText('USD -50.00')).toBeTruthy();expect(screen.getAllByText('Unknown').length).toBeGreaterThan(0);expect(screen.getByText(/Obtain the dated appropriation bridge/)).toBeTruthy();
 fireEvent.click(screen.getAllByRole('button',{name:'Inspect 1 cited rows'})[0]);expect(screen.getByRole('region',{name:'Financial source evidence'})).toBeTruthy();expect(screen.getByText('SELECT amount FROM analysis_fixture')).toBeTruthy();
 fireEvent.click(screen.getByRole('tab',{name:'Missing evidence'}));expect(screen.getByText('Open encumbrance register')).toBeTruthy();
});
it('discards a financial report when the selected scope changes before completion',async()=>{
 const pending=deferred<FinancialReport>();bridge.invoke.mockImplementation((command:string)=>command==='get_financial_workspace'?Promise.resolve(workspace):pending.promise);
 await mounted();fireEvent.click(screen.getByRole('button',{name:'Investigate these records'}));fireEvent.change(screen.getByLabelText('Financial fund'),{target:{value:'200000'}});
 await act(async()=>pending.resolve(report));expect(screen.queryByText('Exact funding difference')).toBeNull();expect((screen.getByLabelText('Financial fund') as HTMLSelectElement).value).toBe('200000');
});
it('discards scenario estimates when the requested amount changes during native revalidation',async()=>{
 const pending=deferred<FinancialScenario>();bridge.invoke.mockImplementation((command:string)=>command==='get_financial_workspace'?Promise.resolve(workspace):command==='evaluate_financial_scenario'?pending.promise:Promise.resolve(report));
 await mounted();await run();fireEvent.click(screen.getByRole('tab',{name:'Policy scenarios'}));fireEvent.change(screen.getByLabelText('Scenario amount'),{target:{value:'100'}});fireEvent.click(screen.getByRole('button',{name:'Calculate sourced scenario'}));fireEvent.change(screen.getByLabelText('Scenario amount'),{target:{value:'900'}});
 await act(async()=>pending.resolve(result));expect(screen.queryByLabelText('Scenario result')).toBeNull();
});

it('automatically investigates mapped defaults and discards that result after a scope change',async()=>{
 const ready:FinancialWorkspace={...workspace,mappings:['budget','actual'].map(role=>({role,snapshotId:role,columns:{},currency:'USD',agency:'City of San Diego'}))};
 const pending=deferred<FinancialReport>();bridge.invoke.mockImplementation((command:string)=>command==='get_financial_workspace'?Promise.resolve(ready):pending.promise);
 render(<FinancialInvestigationsPanel/>);await screen.findByLabelText('Financial fund');expect(bridge.invoke).toHaveBeenCalledWith('run_financial_investigation',{scope:ready.defaultScope});
 fireEvent.change(screen.getByLabelText('Financial fund'),{target:{value:'200000'}});
 await act(async()=>pending.resolve(report));expect(screen.queryByText('Exact funding difference')).toBeNull();
});
