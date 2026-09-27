import { describe, expect, it } from "vitest";
import type { CaseworkCase, CaseworkRequest } from "../types/casework";
import { caseworkDifferences } from "../utils/caseworkDiff";
const base:CaseworkCase={id:"case-a",reportId:"report-a",topic:"homelessness",title:"Shelter investigation",owner:"Desk",disposition:"OPEN",revision:1,createdAt:"2026-09-26",updatedAt:"2026-09-26",questions:[],requests:[],decisions:[],commitments:[],evidence:[],notes:"Local notes"};
const request:CaseworkRequest={id:"r1",title:"Performance records",custodian:"Housing Commission",gap:"Occupied bed nights",resolves:"Delivered volume",draft:"Please provide monthly reports.",state:"SUBMITTED",expectedRecordTypes:["monthly report"],expectedFields:["occupied bed nights"],expectedPeriod:"FY2025",submittedAt:"2026-09-01",responseDue:null,externalReference:"PR-42",productions:[],unresolvedOmissions:[],followUpDraft:"",citationIds:[]};
describe("human-readable shared case comparison",()=>{
  it("identifies consequential edits, removals and newly recorded productions without merging them",()=>{
    const local={...base,requests:[request],questions:[{id:"q1",question:"Was the service delivered?",hypothesis:"Reports may exist elsewhere.",nextTest:"Obtain monthly records.",owner:"Reporter",disposition:"OPEN",citationIds:[],counterCitationIds:[]}]};
    const incoming={...base,owner:"Partner",questions:[],requests:[{...request,state:"PARTIAL",productions:[{id:"p1",receivedAt:"2026-09-20",description:"One month provided",recordTypes:["monthly report"],fields:["invoice month"],period:"July 2024",notes:"Bed nights missing",citationIds:[]}]}]};
    const diff=caseworkDifferences(local,incoming);
    expect(diff).toContainEqual(expect.objectContaining({label:"Case owner",local:"Desk",incoming:"Partner",kind:"changed"}));
    expect(diff).toContainEqual(expect.objectContaining({section:"Research questions",kind:"removed",label:"Was the service delivered?",incoming:"Removed by incoming version"}));
    expect(diff).toContainEqual(expect.objectContaining({label:"Performance records · State",local:"SUBMITTED",incoming:"PARTIAL"}));
    expect(diff).toContainEqual(expect.objectContaining({section:"Productions · Performance records",kind:"added",incoming:expect.stringContaining("Bed nights missing")}));
    expect(local.questions).toHaveLength(1);expect(local.requests[0].productions).toHaveLength(0);
  });
  it("shows an unknown-to-complete commitment claim together with its changed assessment",()=>{
    const commitment={id:"c1",promise:"Publish the reports",responsibleParty:"Department",dueDate:null,completionTest:"Dated release includes all months",state:"UNKNOWN",assessment:"No completion evidence yet",citationIds:[],completionCitationIds:[]};
    const diff=caseworkDifferences({...base,commitments:[commitment]},{...base,commitments:[{...commitment,state:"COMPLETE",assessment:"Partner reports a dated release."}]});
    expect(diff).toContainEqual(expect.objectContaining({label:"Publish the reports · Assessment state",local:"UNKNOWN",incoming:"COMPLETE"}));
    expect(diff).toContainEqual(expect.objectContaining({label:"Publish the reports · Assessment explanation",local:"No completion evidence yet",incoming:"Partner reports a dated release."}));
    expect(diff.some((d)=>d.label.endsWith("Due date"))).toBe(false);
  });
  it("does not present timestamp-only changes as changed research and shows absent values explicitly",()=>{
    expect(caseworkDifferences(base,{...base,revision:4,updatedAt:"2026-10-01"})).toEqual([]);
    expect(caseworkDifferences(undefined,base)).toContainEqual(expect.objectContaining({label:"Case owner",local:"Not recorded",incoming:"Desk"}));
  });
});
