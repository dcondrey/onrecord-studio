# Investigative product implementation roadmap

Requested September 26, 2026. This records the complete twenty priorities and their acceptance criteria. The earlier research, retention, reconstruction, source discovery and export work is the foundation. A feature is complete only when its production UI and native behavior work together and its meaningful checks pass; an attractive placeholder does not count.

| # | Capability | Required useful behavior | State |
|---|---|---|---|
| 1 | Trace spending to services and outcomes | Link cited appropriations, contracts, amendments, payments, deliverables and outcomes; expose missing links and avoid double counting. | Implemented and tested |
| 2 | Persistent investigations | Save case questions, hypotheses, next tests, owners and dispositions; revisit unresolved cases when evidence or actions change. | Implemented and tested |
| 3 | Requests that resolve evidence gaps | Draft and track specific requests, expected records/fields/periods, productions and unresolved omissions; prepare follow-up without sending automatically. | Implemented and tested |
| 4 | Independently challenge findings | Preserve counterevidence and falsification tests, recompute quantities, and require explicit review before promoting a lead to a supported conclusion. | Implemented and tested |
| 5 | Rank consequential leads | Explain ranking using consequence, evidence, novelty and urgency; link each lead to its source and useful next action. | Implemented and tested |
| 6 | Reconcile budget definitions | Distinguish adopted/revised/actual/encumbered amounts, fiscal periods, fund restrictions, account crosswalks, one-time funding and transfers. | Implemented and tested |
| 7 | Audit contracts and payments | Test duplicate invoices, amendments, authorization ceilings, missing deliverables and comparable unit prices; preserve innocent explanations. | Implemented and tested |
| 8 | Identify decision windows | Follow dated agendas, relevant supporting documents, proposed decisions, votes and follow-through; prioritize upcoming opportunities. | Implemented and tested |
| 9 | Track commitments and audit recommendations | Record what was promised, by whom, by when, and the evidence needed to establish completion; distinguish unknown completion from proven failure. | Implemented and tested |
| 10 | Acquire consequential missing records | Maintain case-linked coverage expectations by agency, record type and period; identify the acquisition/request that would unlock analysis. | Implemented and tested |
| 11 | Resolve public entities over time | Link programs, public roles, agencies, vendors and contracts with cited aliases and effective periods; never silently merge a shared name. | Implemented and tested |
| 12 | Detect substantive record changes | Preserve compact, cited version differences and flag changed money, dates, obligations and outcomes, including disappearance/replacement. | Implemented and tested |
| 13 | Compare meaningful service outcomes | Compare costs with consistent units, populations, periods and outcomes; distinguish missing measurements and incompatible denominators. | Implemented and tested |
| 14 | Map service access and repeated operations | Relate public facilities, service hours, verified observations and operations geographically and over time, with usable source links. | Implemented and tested |
| 15 | Evaluate explicit policy scenarios | Calculate transparent alternatives from cited unit costs and available funding; expose assumptions, restrictions and uncertainty. | Implemented and tested |
| 16 | Trace documented public influence | Follow cited relationships among officials, public business entities, contracts and disclosures with dates and uncertainty; shared names do not establish conflicts. | Implemented and tested |
| 17 | Resolve clues across documents | Compare complementary fragments, unredacted passages and dated public-role holders; preserve alternatives and exact source evidence. | Implemented and tested |
| 18 | Preserve and publish reviewable evidence | Produce reproducible case packets with claim-level citations, source identity, calculations, reviewed excerpts, compact retention and version context. | Implemented and tested |
| 19 | Collaborate with accountable review | Support portable shared cases, assigned reviews, provenance of contributions and explicit conflict handling; protect private work and never silently overwrite another review. | Implemented and tested |
| 20 | Evaluate investigative usefulness | Replay reference investigations, score supported discoveries and false leads, and support human pilot assessments of usefulness and time saved. Recruiting or contacting external pilot participants is a separate human action. | Implemented and tested |

## Implementation boundaries

- Public business records and public official roles are the focus. Reconstructed clues remain inferences, never replacements for original evidence.
- Missing is not zero; different jurisdictions, periods, definitions and units cannot silently be combined.
- Outbound requests, right-of-reply messages, publication and invitations remain user actions. No external messages are sent by this roadmap.
- Original files can be released after validated extraction/analysis. Compact derivatives retain provenance, links and hashes. The application must not pretend an original can be verified when only a derivative remains.
- Live multi-user hosting, a customer’s deployment environment, and human pilot judgments cannot be simulated as completed. Local collaboration and export behavior must be described precisely.

## Design references

- [Open Contracting Partnership procurement indicators](https://www.open-contracting.org/resources/red-flags-in-public-procurement-a-guide-to-using-data-to-detect-and-mitigate-risks/): measurable procurement risks and implementation evidence.
- [OCCRP Aleph / FollowTheMoney model](https://docs.aleph.occrp.org/developers/): consistent entities and relationships across heterogeneous sources.
- [MuckRock request workflow](https://help.muckrock.com/How-MuckRock-Works-19ef889269638140b169c8224a4c7c05): practical acquisition and request tracking.

## Delivered workflow

The dashboard now includes Cases and follow-through, Spending and outcomes, Coverage and changes, Evidence review, and Usefulness. See the [implementation and validation record](validation/investigative-workbench-2026-09-26.md) for behavior, integration, evidence, and explicit limits.
