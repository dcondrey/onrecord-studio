// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import type { CaseworkCase, CaseworkImportPreview, CaseworkState } from "../types/casework";
const bridge=vi.hoisted(()=>({invoke:vi.fn()}));
vi.mock("../ipc",()=>({invokeCommand:bridge.invoke}));
vi.mock("@tauri-apps/api/core",()=>({isTauri:()=>true}));
import CaseworkPanel from "../components/dashboard/CaseworkPanel";
const caseA:CaseworkCase={id:"case-a",reportId:"report-a",topic:"homelessness",title:"Shelter contract",owner:"",disposition:"OPEN",revision:1,createdAt:"2026-09-26T12:00:00Z",updatedAt:"2026-09-26T12:00:00Z",questions:[{id:"q1",question:"Were bed nights delivered?",hypothesis:"Reports may exist separately.",nextTest:"Obtain the monthly reports.",owner:"",disposition:"OPEN",citationIds:["e1"],counterCitationIds:[]}],requests:[{id:"r1",title:"Monthly reports",custodian:"Housing Commission",gap:"Bed nights",resolves:"Delivered service volume",draft:"Please provide the reports.",state:"DRAFT",expectedRecordTypes:["monthly report"],expectedFields:["occupied bed nights"],expectedPeriod:"FY2025",submittedAt:null,responseDue:null,externalReference:"",productions:[],unresolvedOmissions:[],followUpDraft:"",citationIds:["e1"]}],decisions:[],commitments:[],evidence:[{id:"e1",reportId:"report-a",evidenceId:"e1",label:"Shelter contract",quote:"Provider shall report occupied bed nights.",sourceUrl:"https://www.sandiego.gov/contract.pdf",sourceSha256:"a",sourceReferences:[],portalKey:"city",documentId:1,characterOffset:0,verification:"SAVED_REPORT"}],notes:"Scope note"};
const caseB:CaseworkCase={...caseA,id:"case-b",reportId:"report-b",title:"Public bathrooms",topic:"bathrooms",notes:"Bathroom scope"};
const state:CaseworkState={cases:[caseA,caseB],events:[],leads:[],availableEvidence:[]};
const deferred=<T,>()=>{let resolve!:(value:T)=>void;const promise=new Promise<T>((r)=>{resolve=r;});return{promise,resolve};};
beforeEach(()=>{bridge.invoke.mockReset();bridge.invoke.mockResolvedValue(state);});
afterEach(cleanup);
async function openA(){render(<CaseworkPanel/>);await screen.findByRole("option",{name:"Shelter contract · r1"});fireEvent.change(screen.getByLabelText("Open saved case"),{target:{value:"case-a"}});fireEvent.change(screen.getByLabelText("Contributor name or team label"),{target:{value:"Reporter"}});}
describe("persistent casework",()=>{
  it("saves actual hypotheses and request expectations with optimistic revision identity",async()=>{
    await openA();fireEvent.change(screen.getByLabelText("Hypothesis 1"),{target:{value:"Reconcile invoice months with performance records."}});fireEvent.change(screen.getByLabelText("Expected fields 1 (one per line)"),{target:{value:"occupied bed nights\ninvoice month"}});
    bridge.invoke.mockImplementation(async(name:string,args:{case:CaseworkCase})=>name==="save_casework_case"?{...args.case,revision:2}:state);
    fireEvent.click(screen.getByRole("button",{name:"Save case revision"}));await screen.findByText("Saved revision 2.");expect(bridge.invoke).toHaveBeenCalledWith("save_casework_case",expect.objectContaining({expectedRevision:1,author:"Reporter",case:expect.objectContaining({questions:[expect.objectContaining({hypothesis:"Reconcile invoice months with performance records."})],requests:[expect.objectContaining({expectedFields:["occupied bed nights","invoice month"]})]})}));
  });
  it("discards a saved case response after switching to another investigation",async()=>{
    await openA();const pending=deferred<CaseworkCase>();bridge.invoke.mockReturnValue(pending.promise);fireEvent.click(screen.getByRole("button",{name:"Save case revision"}));fireEvent.change(screen.getByLabelText("Open saved case"),{target:{value:"case-b"}});
    await act(async()=>{pending.resolve({...caseA,revision:2,notes:"Late response from shelter case"});});
    expect(screen.getByLabelText("Case title")).toHaveProperty("value","Public bathrooms");expect(screen.getByLabelText("Case notes")).toHaveProperty("value","Bathroom scope");expect(screen.queryByText("Saved revision 2.")).toBeNull();
  });
  it("discards an import preview when its input packet has changed",async()=>{
    await openA();const pending=deferred<CaseworkImportPreview>();bridge.invoke.mockReturnValue(pending.promise);fireEvent.change(screen.getByLabelText("Incoming case JSON"),{target:{value:'{"old":"packet"}'}});fireEvent.click(screen.getByRole("button",{name:"Preview shared case"}));fireEvent.change(screen.getByLabelText("Incoming case JSON"),{target:{value:'{"new":"packet"}'}});
    await act(async()=>{pending.resolve({mode:"NEW",localRevision:null,packetSha256:"old",warnings:[],case:caseA,contributor:"Old sender"});});expect(screen.queryByText("Import review: new")).toBeNull();expect(screen.queryByRole("button",{name:"Apply reviewed case packet"})).toBeNull();
  });
  it("discards a file read when the incoming packet is replaced by newer text", async () => {
    await openA();
    const pending=deferred<string>();
    fireEvent.change(screen.getByLabelText("Open a shared case file"), { target: { files: [{name:"old-case.json",size:100,text:()=>pending.promise}] } });
    fireEvent.change(screen.getByLabelText("Incoming case JSON"), { target: { value: "newer packet text" } });
    await act(async()=>{pending.resolve("obsolete file contents");});
    expect(screen.getByLabelText("Incoming case JSON")).toHaveProperty("value","newer packet text");
  });
  it("rejects an oversized portable file before reading or previewing it", async () => {
    await openA();const read=vi.fn();
    fireEvent.change(screen.getByLabelText("Open a shared case file"), { target: { files: [{name:"large-case.json",size:2000001,text:read}] } });
    expect(screen.getByText("Case files must be at most 2 MB.")).toBeTruthy();expect(read).not.toHaveBeenCalled();
  });
  it("shows unresolved production gaps and does not equate a passed commitment date with failure",async()=>{
    bridge.invoke.mockResolvedValue({...state,cases:[{...caseA,commitments:[{id:"c1",promise:"Publish monthly reports",responsibleParty:"Department",dueDate:"2020-01-01",completionTest:"A public release contains the months",state:"UNKNOWN",assessment:"",citationIds:["e1"],completionCitationIds:[]}]},caseB]});await openA();
    expect(screen.getByText("Expected field not yet recorded as produced: occupied bed nights")).toBeTruthy();expect(screen.getByText("Requested period coverage remains unconfirmed: FY2025")).toBeTruthy();expect(screen.getByText("The stated due date has passed; completion remains unknown.")).toBeTruthy();expect(screen.getByLabelText("Commitment state 1")).toHaveProperty("value","UNKNOWN");
  });
  it("makes historical versions and the need to review conclusions visible while notes remain editable", async () => {
    const historical = structuredClone(caseA);
    historical.evidence[0].verification = "HISTORICAL_REPORT";
    historical.questions[0].disposition = "NEEDS_REVIEW";
    bridge.invoke.mockResolvedValue({ ...state, cases: [historical] });
    await openA();
    expect(screen.getByText(/Historical evidence: a public source has a newer version/)).toBeTruthy();
    expect(screen.getAllByText("Historical report · source has a newer version").length).toBeGreaterThan(0);
    expect(screen.getByLabelText("Question disposition 1")).toHaveProperty("value", "NEEDS_REVIEW");
    fireEvent.change(screen.getByLabelText("Case notes"), { target: { value: "Compare the preserved quotation to the new publication." } });
    bridge.invoke.mockImplementation(async (_name: string, args: { case: CaseworkCase }) => ({ ...args.case, revision: 2 }));
    fireEvent.click(screen.getByRole("button", { name: "Save case revision" }));
    await screen.findByText("Saved revision 2.");
    expect(bridge.invoke).toHaveBeenLastCalledWith("save_casework_case", expect.objectContaining({ case: expect.objectContaining({ notes: "Compare the preserved quotation to the new publication." }) }));
  });
  it("keeps a conflict review visible and offers comparison without an overwrite action",async()=>{
    await openA();bridge.invoke.mockResolvedValue({mode:"CONFLICT",localRevision:1,packetSha256:"packet",warnings:["Contributor is not authenticated."],case:{...caseA,notes:"Partner's divergent work"},contributor:"Partner"});fireEvent.change(screen.getByLabelText("Incoming case JSON"),{target:{value:"{}"}});fireEvent.click(screen.getByRole("button",{name:"Preview shared case"}));await screen.findByText("Import review: conflict");const changes=within(screen.getByRole("region",{name:"Changes in shared case"}));expect(changes.getByText("Partner's divergent work")).toBeTruthy();expect(changes.getByText("Scope note")).toBeTruthy();expect(changes.getByRole("heading",{name:"Case notes"})).toBeTruthy();expect(screen.queryByRole("button",{name:"Apply reviewed case packet"})).toBeNull();expect(screen.getByLabelText("Incoming case")).toHaveProperty("value",expect.stringContaining("Partner's divergent work"));expect(screen.getByLabelText("Local case")).toHaveProperty("value",expect.stringContaining("Scope note"));
  });
});
