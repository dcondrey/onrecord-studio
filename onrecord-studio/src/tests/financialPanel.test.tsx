// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import type { FinancialReport, FinancialScenario, FinancialWorkspace } from '../types/financial';
const bridge=vi.hoisted(()=>({invoke:vi.fn()}));
vi.mock('../ipc',()=>({invokeCommand:bridge.invoke,formatNativeError:(reason:unknown)=>String(reason)}));
vi.mock('@tauri-apps/api/core',()=>({isTauri:()=>true}));
import FinancialInvestigationsPanel from '../components/dashboard/FinancialInvestigationsPanel';
const workspace:FinancialWorkspace={datasets:[],mappings:[],defaultScope:{fiscalYear:2025,fund:'100000',department:'Police'},roles:[],fields:[],requirements:{},warnings:[],fiscalYears:[2025,2024],funds:['100000','200000'],departments:['Police','Parks']};
const report:FinancialReport={id:'case',createdAt:'2026-09-26',scope:workspace.defaultScope,reconciliation:[{fundingDuration:'unknown',agency:'City of San Diego',key:'funding',fiscalYear:'2025',fund:'100000',account:'5001',accountLabel:'Salaries',department:'Police',center:'001',program:'Shelter',currency:'USD',adopted:'100',revised:'200',actual:'50',encumbered:null,actualMinusAdopted:'-50',uncommittedRevised:null,citations:['source'],limitations:['Encumbrances are unknown.']}],chains:[],findings:[{calculations:[{label:'Observed difference',formula:'50 - 100',result:'-50.00',unit:'USD'}],id:'finding',kind:'budget_variance',basis:'lead',confidenceBasisPoints:null,title:'Exact funding difference',observation:'Compare the revised authority before alleging excess spending.',citations:['source'],alternatives:['A documented transfer may explain the difference.'],nextTest:'Obtain the dated appropriation bridge.'}],outcomes:[{agency:'City of San Diego',key:'outcome',program:'Shelter',contractId:'C',periodStart:'2024-07-01',periodEnd:'2025-06-30',population:'Adults',metric:'Permanent housing exits',unit:'people',denominatorUnit:'people',currency:'USD',cost:'100',quantity:'10',denominator:'20',costPerUnit:'10',successRatePercent:'50',comparisonGroup:'group',citations:['source'],limitations:[]}],gaps:[{title:'Open encumbrance register',recordsNeeded:'Existing commitments by fund/year/account.',why:'Unspent cash is not uncommitted authority.',citations:[]}],evidence:[{id:'source',role:'budget',snapshotId:'snapshot',sourceUrl:'https://www.sandiego.gov/source.csv',sourceSha256:'a'.repeat(64),queryId:'query',columns:['amount'],row:['100.00']}],queries:{query:'SELECT amount FROM analysis_fixture'},records:[],incompleteRoles:[],limitations:['An appropriation is not a payment.']};
const result:FinancialScenario={fundingDuration:'one_time',periodScope:'FY2025 only; one fiscal period',status:'conditional_estimate',targetProgram:'Shelter',availableRevisedBalance:'1000',proposedAmount:'100',currency:'USD',unit:'people',estimatedWholeUnits:'10',lowerUnits:'5',upperUnits:'10',formula:'Exact source formula',assumptions:['Conditional capacity, not transfer authority.'],blockers:[],citations:[],queries:{}};
function deferred<T>(){let resolve!:(v:T)=>void;const promise=new Promise<T>(r=>{resolve=r;});return{promise,resolve};}
beforeEach(()=>{bridge.invoke.mockReset();bridge.invoke.mockImplementation((command:string)=>command==='get_financial_workspace'?Promise.resolve(workspace):Promise.resolve(report));});afterEach(cleanup);
async function mounted(){render(<FinancialInvestigationsPanel/>);await screen.findByRole('button',{name:'Investigate these records'});}
async function run(){fireEvent.click(screen.getByRole('button',{name:'Investigate these records'}));await screen.findByText('Exact funding difference');}
it('shows real cited comparisons, explicit gaps and reproducible source rows',async()=>{
 await mounted();await run();expect(screen.getByText('50 - 100')).toBeTruthy();expect(screen.getByText('USD -50.00')).toBeTruthy();expect(screen.getAllByText('Unknown').length).toBeGreaterThan(0);expect(screen.getByText(/Obtain the dated appropriation bridge/)).toBeTruthy();
 fireEvent.click(screen.getAllByRole('button',{name:'Inspect 1 cited rows'})[0]);expect(screen.getByRole('region',{name:'Financial source evidence'})).toBeTruthy();expect(screen.getByText('SELECT amount FROM analysis_fixture')).toBeTruthy();
 fireEvent.click(screen.getByRole('button',{name:'Missing evidence'}));expect(screen.getByText('Open encumbrance register')).toBeTruthy();
});
it('discards a financial report when the selected scope changes before completion',async()=>{
 const pending=deferred<FinancialReport>();bridge.invoke.mockImplementation((command:string)=>command==='get_financial_workspace'?Promise.resolve(workspace):pending.promise);
 await mounted();fireEvent.click(screen.getByRole('button',{name:'Investigate these records'}));fireEvent.change(screen.getByLabelText('Financial fund'),{target:{value:'200000'}});
 await act(async()=>pending.resolve(report));expect(screen.queryByText('Exact funding difference')).toBeNull();expect((screen.getByLabelText('Financial fund') as HTMLSelectElement).value).toBe('200000');
});
it('discards scenario estimates when the requested amount changes during native revalidation',async()=>{
 const pending=deferred<FinancialScenario>();bridge.invoke.mockImplementation((command:string)=>command==='get_financial_workspace'?Promise.resolve(workspace):command==='evaluate_financial_scenario'?pending.promise:Promise.resolve(report));
 await mounted();await run();fireEvent.click(screen.getByRole('button',{name:'Policy scenarios'}));fireEvent.change(screen.getByLabelText('Scenario amount'),{target:{value:'100'}});fireEvent.click(screen.getByRole('button',{name:'Calculate sourced scenario'}));fireEvent.change(screen.getByLabelText('Scenario amount'),{target:{value:'900'}});
 await act(async()=>pending.resolve(result));expect(screen.queryByLabelText('Scenario result')).toBeNull();
});

it('automatically investigates mapped defaults and discards that result after a scope change',async()=>{
 const ready:FinancialWorkspace={...workspace,mappings:['budget','actual'].map(role=>({role,snapshotId:role,columns:{},currency:'USD',agency:'City of San Diego'}))};
 const pending=deferred<FinancialReport>();bridge.invoke.mockImplementation((command:string)=>command==='get_financial_workspace'?Promise.resolve(ready):pending.promise);
 render(<FinancialInvestigationsPanel/>);await screen.findByLabelText('Financial fund');expect(bridge.invoke).toHaveBeenCalledWith('run_financial_investigation',{scope:ready.defaultScope});
 fireEvent.change(screen.getByLabelText('Financial fund'),{target:{value:'200000'}});
 await act(async()=>pending.resolve(report));expect(screen.queryByText('Exact funding difference')).toBeNull();
});
it('labels a counted spending case and a pattern differently and shows the cited rows',async()=>{
 const spending:FinancialReport={...report,findings:[
  {...report.findings[0],id:'counted',kind:'appropriation_exceeded',basis:'counted_comparison',confidenceBasisPoints:9000},
  {...report.findings[0],id:'pattern',kind:'split_purchase_pattern',title:'Payments clustered just under a stated approval limit',basis:'pattern',confidenceBasisPoints:4000},
 ]};
 bridge.invoke.mockImplementation((command:string)=>command==='get_financial_workspace'?Promise.resolve(workspace):Promise.resolve(spending));
 await mounted();await run();
 expect(screen.getByText(/Counted comparison of recorded rows with a written limit · confidence 90%/)).toBeTruthy();
 expect(screen.getByText(/Labeled pattern, not a finding that a rule was broken · confidence 40%/)).toBeTruthy();
 expect(screen.queryByText(/wasted|illegal/i)).toBeNull();
 expect(screen.getAllByRole('button',{name:'Inspect 1 cited rows'}).length).toBeGreaterThanOrEqual(2);
});

it('draws a contract overrun in ink from the decimal strings and says when no payments dataset is mapped',async()=>{
 const chain={agency:'City of San Diego',contractId:'H-24-1180',program:'Storage',vendor:'V-9',currency:'USD',baseAuthority:'1000000.00',amendments:'250000.00',totalAuthority:'1250000.00',payments:'1486200.10',remainingAuthority:'-236200.10',stages:{budget:['source'],contract:['source'],amendment:[],payment:['source'],outcome:[]},missingLinks:[],limitations:[]};
 const mapped:FinancialWorkspace={...workspace,mappings:[{role:'payment',snapshotId:'pay',columns:{},currency:'USD',agency:'City of San Diego'}]};
 bridge.invoke.mockImplementation((command:string)=>command==='get_financial_workspace'?Promise.resolve(mapped):Promise.resolve({...report,chains:[chain]}));
 await mounted();await run();fireEvent.click(screen.getByRole('button',{name:'Spending chains'}));
 expect(screen.getByText('1 of 1 contracts show posted payments above retrieved authority; largest: H-24-1180 by $236,200.10.')).toBeTruthy();
 expect(screen.getByText('+$236,200.10 over retrieved authority')).toBeTruthy();
 expect(document.body.innerHTML).not.toContain('--viz-counted');
 // A stage with no cited rows is a hollow node read as not linked; a linked one opens its rows.
 expect(screen.getByRole('img',{name:/Contract H-24-1180 · Storage, Amendment: not linked/})).toBeTruthy();
 fireEvent.click(screen.getByRole('img',{name:/Contract H-24-1180 · Storage, Payment: 1 cited row/}));
 expect(screen.getByRole('region',{name:'Financial source evidence'})).toBeTruthy();
 cleanup();
 bridge.invoke.mockImplementation((command:string)=>command==='get_financial_workspace'?Promise.resolve(workspace):Promise.resolve({...report,chains:[chain]}));
 await mounted();await run();fireEvent.click(screen.getByRole('button',{name:'Spending chains'}));
 expect(screen.getByText(/No payments dataset mapped, so none of 1 contract is compared/)).toBeTruthy();
 expect(screen.queryByText(/over retrieved authority/)).toBeNull();
 // With no payments dataset mapped, no payment link was looked for: the stage says 'not mapped', never 'not linked'.
 expect(screen.queryByRole('img',{name:/Payment: not linked/})).toBeNull();
 expect([...document.querySelectorAll('[data-absence="not-covered"] title')].some((title)=>/Payment: no dataset is mapped/.test(title.textContent??''))).toBe(true);
});
