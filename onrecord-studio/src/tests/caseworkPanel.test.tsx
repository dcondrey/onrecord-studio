// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import type { CaseworkCase, CaseworkImportPreview, CaseworkState, CompromisedImportWarning } from "../types/casework";
import type { CaseEvidencePackageResult } from "../types/investigation";
const bridge=vi.hoisted(()=>({invoke:vi.fn()}));
vi.mock("../ipc",()=>({invokeCommand:bridge.invoke}));
vi.mock("@tauri-apps/api/core",()=>({isTauri:()=>true}));
vi.mock("../components/dashboard/useContributors",()=>({
  groupKeyId:(id:string)=>id,
  useContributors:()=>({state:{status:"ready",contributors:[],identity:null},reload:()=>{},trust:async()=>{},untrust:async()=>{}}),
}));
vi.mock("@tauri-apps/plugin-dialog",()=>({open:()=>null}));
import CaseworkPanel from "../components/dashboard/CaseworkPanel";
const caseA:CaseworkCase={id:"case-a",reportId:"report-a",topic:"homelessness",title:"Shelter contract",owner:"",disposition:"OPEN",revision:1,createdAt:"2026-09-26T12:00:00Z",updatedAt:"2026-09-26T12:00:00Z",questions:[{id:"q1",question:"Were bed nights delivered?",hypothesis:"Reports may exist separately.",nextTest:"Obtain the monthly reports.",owner:"",disposition:"OPEN",citationIds:["e1"],counterCitationIds:[]}],requests:[{id:"r1",title:"Monthly reports",custodian:"Housing Commission",gap:"Bed nights",resolves:"Delivered service volume",draft:"Please provide the reports.",state:"DRAFT",expectedRecordTypes:["monthly report"],expectedFields:["occupied bed nights"],expectedPeriod:"FY2025",submittedAt:null,responseDue:null,externalReference:"",productions:[],unresolvedOmissions:[],followUpDraft:"",compiledDraft:"",filingPlan:null,citationIds:["e1"]}],decisions:[],commitments:[],evidence:[{id:"e1",reportId:"report-a",evidenceId:"e1",label:"Shelter contract",quote:"Provider shall report occupied bed nights.",sourceUrl:"https://www.sandiego.gov/contract.pdf",sourceSha256:"a",sourceReferences:[],portalKey:"city",documentId:1,characterOffset:0,verification:"SAVED_REPORT"}],notes:"Scope note"};
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
    await act(async()=>{pending.resolve({mode:"NEW",localRevision:null,packetSha256:"old",warnings:[],case:caseA,contributor:"Old sender",signerKeyId:null,commuting:[],contested:[]});});expect(screen.queryByText("Import review: new")).toBeNull();expect(screen.queryByRole("button",{name:"Apply reviewed case packet"})).toBeNull();
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
    fireEvent.change(screen.getByLabelText("Open a shared case file"), { target: { files: [{name:"large-case.json",size:8*1024*1024+1,text:read}] } });
    expect(screen.getByText(/Case files must be at most 8 MB/)).toBeTruthy();expect(read).not.toHaveBeenCalled();
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
  const custodyResult=(overrides:Partial<CaseEvidencePackageResult>={}):CaseEvidencePackageResult=>({caseId:"case-a",caseRevision:1,outputPath:"/reports/case-a-custody.json",packageSha256:"deadbeef",signatureAlgorithm:"Ed25519",signerKeyId:"abc123def456",generatedAt:"2026-09-28T00:00:00Z",verifiedCitationCount:1,unverifiedCitationLabels:["Imported claim (UNVERIFIED_PORTABLE)"],withheldCitationLabels:[],ledgerSequence:1,previousExportSha256:null,verifierScriptPath:"/reports/verify-evidence-package.mjs",verifierPagePath:"/reports/verify-evidence-package.html",...overrides});
  it("surfaces unverified citations and the signed revision inside a chain-of-custody export instead of hiding them",async()=>{
    await openA();
    bridge.invoke.mockResolvedValue(custodyResult());
    fireEvent.click(screen.getByRole("button",{name:"Export signed chain-of-custody package"}));
    await screen.findByText("Signed chain-of-custody package written to /reports/case-a-custody.json");
    expect(bridge.invoke).toHaveBeenCalledWith("export_case_evidence_package",{caseId:"case-a",targetPath:null,withheldCitations:[]});
    expect(screen.getByText(/Signed revision 1 —/)).toBeTruthy();
    expect(screen.getByText("1 citation could not be confirmed and remain marked unverified inside the signed package:")).toBeTruthy();
    expect(screen.getByText("Imported claim (UNVERIFIED_PORTABLE)")).toBeTruthy();
  });
  it("discards a chain-of-custody export response after switching to another investigation",async()=>{
    await openA();
    const pending=deferred<CaseEvidencePackageResult>();
    bridge.invoke.mockReturnValue(pending.promise);
    fireEvent.click(screen.getByRole("button",{name:"Export signed chain-of-custody package"}));
    fireEvent.change(screen.getByLabelText("Open saved case"),{target:{value:"case-b"}});
    await act(async()=>{pending.resolve(custodyResult({outputPath:"/reports/stale.json"}));});
    expect(screen.queryByText(/Signed chain-of-custody package written to/)).toBeNull();
  });
  it("keeps a conflict review visible and offers comparison without an overwrite action",async()=>{
    await openA();bridge.invoke.mockResolvedValue({mode:"CONFLICT",localRevision:1,packetSha256:"packet",warnings:["Contributor is not authenticated."],case:{...caseA,notes:"Partner's divergent work"},contributor:"Partner",signerKeyId:null,commuting:[],contested:[]});fireEvent.change(screen.getByLabelText("Incoming case JSON"),{target:{value:"{}"}});fireEvent.click(screen.getByRole("button",{name:"Preview shared case"}));await screen.findByText("Import review: conflict");const changes=within(screen.getByRole("region",{name:"Changes in shared case"}));expect(changes.getByText("Partner's divergent work")).toBeTruthy();expect(changes.getByText("Scope note")).toBeTruthy();expect(changes.getByRole("heading",{name:"Case notes"})).toBeTruthy();expect(screen.queryByRole("button",{name:"Apply reviewed case packet"})).toBeNull();expect(screen.getByLabelText("Incoming case")).toHaveProperty("value",expect.stringContaining("Partner's divergent work"));expect(screen.getByLabelText("Local case")).toHaveProperty("value",expect.stringContaining("Scope note"));
  });
  it("shows the verified signer key next to the claimed contributor, and omits it when absent",async()=>{
    await openA();bridge.invoke.mockResolvedValue({mode:"NEW",localRevision:null,packetSha256:"packet",warnings:[],case:caseA,contributor:"Partner",signerKeyId:"abcdef0123456789",commuting:[],contested:[]});fireEvent.change(screen.getByLabelText("Incoming case JSON"),{target:{value:"{}"}});fireEvent.click(screen.getByRole("button",{name:"Preview shared case"}));await screen.findByText("Import review: new");
    expect(screen.getByText(/signed by local key/).textContent).toContain("abcdef012345");
  });
  it("requires a reason before a withheld citation can export, and sends it in the payload",async()=>{
    await openA();
    const exportButton=()=>screen.getByRole("button",{name:"Export signed chain-of-custody package"}) as HTMLButtonElement;
    expect(exportButton().disabled).toBe(false);
    fireEvent.click(screen.getByRole("checkbox",{name:"Withhold Shelter contract"}));
    expect(exportButton().disabled).toBe(true);
    fireEvent.change(screen.getByLabelText("Reason for withholding Shelter contract"),{target:{value:"Ongoing personnel investigation."}});
    expect(exportButton().disabled).toBe(false);
    bridge.invoke.mockResolvedValue(custodyResult());
    fireEvent.click(exportButton());
    await screen.findByText("Signed chain-of-custody package written to /reports/case-a-custody.json");
    expect(bridge.invoke).toHaveBeenCalledWith("export_case_evidence_package",{caseId:"case-a",targetPath:null,withheldCitations:[{citationId:"e1",reason:"Ongoing personnel investigation."}]});
  });
  it("discards the export chain view after switching to another investigation",async()=>{
    await openA();
    const pending=deferred<import("../types/casework").CaseExportLedgerEntry[]>();
    bridge.invoke.mockReturnValue(pending.promise);
    fireEvent.click(screen.getByRole("button",{name:"View export chain"}));
    fireEvent.change(screen.getByLabelText("Open saved case"),{target:{value:"case-b"}});
    await act(async()=>{pending.resolve([{sequence:1,manifestSha256:"h1",previousManifestSha256:null,signerKeyId:"k",reservedAt:"2026-09-29T00:00:00Z",outcome:"WRITTEN",outcomeDetail:null,outcomeRecordedAt:null,previousLinkValid:true}]);});
    expect(screen.queryByText(/local export ledger/)).toBeNull();
  });
  it("discards a compromised-signer check fetched for a case the viewer has since left", async () => {
    const pendingA = deferred<CompromisedImportWarning[]>();
    bridge.invoke.mockImplementation(async (name: string, args?: { caseId?: string }) => {
      if (name === "compromised_case_imports") return args?.caseId === "case-a" ? pendingA.promise : [];
      return state;
    });
    render(<CaseworkPanel />);
    await screen.findByRole("option", { name: "Shelter contract · r1" });
    fireEvent.change(screen.getByLabelText("Open saved case"), { target: { value: "case-a" } });
    fireEvent.change(screen.getByLabelText("Open saved case"), { target: { value: "case-b" } });
    await act(async () => {
      pendingA.resolve([{ revision: 1, signerKeyId: "compromised-key", revokedAt: "2026-09-29T00:00:00Z", reason: "Leaked." }]);
    });
    expect(screen.queryByText(/previously imported revision/)).toBeNull();
  });
  it("refreshes the compromised-signer banner on whichever case is open when a mark-compromised write actually lands, not when it started", async () => {
    const pendingMark = deferred<null>();
    let compromisedCallsForB = 0;
    bridge.invoke.mockImplementation(async (name: string, args?: { caseId?: string }) => {
      if (name === "compromised_case_imports") {
        if (args?.caseId !== "case-b") return [];
        compromisedCallsForB += 1;
        return compromisedCallsForB === 1
          ? []
          : [{ revision: 1, signerKeyId: "key-1", revokedAt: "2026-09-29T00:00:00Z", reason: "Leaked." }];
      }
      if (name === "get_signing_key_revocations") {
        return { schemaVersion: "onrecord-signing-key-revocations-v1", entries: [{ keyId: "key-1", revokedAt: "2026-09-28T00:00:00Z", reason: "Routine.", status: "RETIRED" }] };
      }
      if (name === "mark_signing_key_compromised") return pendingMark.promise;
      return state;
    });
    await openA();
    fireEvent.click(screen.getByRole("button", { name: "View retired/compromised keys" }));
    await screen.findByRole("button", { name: "Mark compromised…" });
    fireEvent.click(screen.getByRole("button", { name: "Mark compromised…" }));
    fireEvent.change(screen.getByLabelText("Reason this key is compromised"), { target: { value: "Leaked." } });
    fireEvent.click(screen.getByRole("button", { name: "Confirm: this key is compromised" }));
    fireEvent.change(screen.getByLabelText("Open saved case"), { target: { value: "case-b" } });
    await act(async () => { pendingMark.resolve(null); });
    await screen.findByText(/previously imported revision/);
  });
  it("collapses a key with both a RETIRED and a COMPROMISED entry into one row read as compromised", async () => {
    bridge.invoke.mockImplementation(async (name: string) => {
      if (name === "get_signing_key_revocations") {
        return {
          schemaVersion: "onrecord-signing-key-revocations-v1",
          entries: [
            { keyId: "key-1", revokedAt: "2026-09-28T00:00:00Z", reason: "Routine.", status: "RETIRED" },
            { keyId: "key-1", revokedAt: "2026-09-29T00:00:00Z", reason: "Leaked.", status: "COMPROMISED" },
          ],
        };
      }
      if (name === "compromised_case_imports") return [];
      return state;
    });
    await openA();
    fireEvent.click(screen.getByRole("button", { name: "View retired/compromised keys" }));
    await screen.findByText("1 recorded key");
    const rows = screen.getAllByRole("listitem").filter((row) => row.textContent?.includes("Leaked."));
    expect(rows).toHaveLength(1);
    expect(rows[0].textContent).toContain("compromised");
    expect(screen.queryByRole("button", { name: "Mark compromised…" })).toBeNull();
  });
  it("does not carry a stale compromise warning onto a case opened by applying an import", async () => {
    const caseC: CaseworkCase = { ...caseA, id: "case-c", reportId: "report-c", title: "Utilities audit" };
    bridge.invoke.mockImplementation(async (name: string, args?: { caseId?: string }) => {
      if (name === "compromised_case_imports") {
        return args?.caseId === "case-a" ? [{ revision: 1, signerKeyId: "key-1", revokedAt: "2026-09-29T00:00:00Z", reason: "Leaked." }] : [];
      }
      if (name === "preview_casework_import") return { mode: "NEW", localRevision: null, packetSha256: "packet", warnings: [], case: caseC, contributor: "Partner", signerKeyId: null };
      if (name === "import_casework_packet") return caseC;
      return state;
    });
    await openA();
    await screen.findByText(/previously imported revision/);
    fireEvent.change(screen.getByLabelText("Incoming case JSON"), { target: { value: "{}" } });
    fireEvent.click(screen.getByRole("button", { name: "Preview shared case" }));
    await screen.findByText("Import review: new");
    fireEvent.click(screen.getByRole("button", { name: "Apply reviewed case packet" }));
    await screen.findByText("Saved revision 1.");
    expect(screen.queryByText(/previously imported revision/)).toBeNull();
    expect(bridge.invoke).toHaveBeenCalledWith("compromised_case_imports", { caseId: "case-c" });
  });
  it("requires explicit confirmation before a compromised rotation invokes anything, and cancel invokes nothing", async () => {
    await openA();
    fireEvent.change(screen.getByLabelText("Reason for rotation"), { target: { value: "Found on a leaked backup." } });
    fireEvent.click(screen.getByLabelText("This key is compromised, not just being retired"));
    fireEvent.click(screen.getByRole("button", { name: "Mark old key compromised…" }));
    expect(bridge.invoke).not.toHaveBeenCalledWith("rotate_export_signing_key", expect.anything());
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(bridge.invoke).not.toHaveBeenCalledWith("rotate_export_signing_key", expect.anything());
    expect(screen.queryByRole("button", { name: "Confirm: rotate and mark compromised" })).toBeNull();
  });
  it("sends compromised: false for a routine rotation, without requiring confirmation", async () => {
    await openA();
    fireEvent.change(screen.getByLabelText("Reason for rotation"), { target: { value: "Scheduled hygiene rotation." } });
    bridge.invoke.mockResolvedValue({ retiredKeyId: "old", newKeyId: "new", archivedKeyPath: "/x" });
    fireEvent.click(screen.getByRole("button", { name: "Rotate key (routine)" }));
    await screen.findByText("Signing key rotated.");
    expect(bridge.invoke).toHaveBeenCalledWith("rotate_export_signing_key", { reason: "Scheduled hygiene rotation.", compromised: false });
  });
});
