# Persistent investigative casework

The **Cases** tab turns a saved research dossier into a durable investigation. Its questions, hypotheses, next tests, owner, disposition, requests, decisions and commitments live in the native database. Creating a case again from the same dossier opens its existing case. Evidence from later saved dossiers can be attached without changing the founding dossier.

A save creates a new compressed revision and contribution event. The submitted revision must still match the stored revision. A stale editor receives a conflict instead of overwriting another contributor's work. Exact local citations are checked against saved report evidence; query citations compare complete rows without converting decimal strings or missing values to numbers. When a public source is replaced, retained quotations and hashes remain usable as explicitly historical evidence. Classification is refreshed on load, export and save. Assessments supported only by that older version move to **needs review**, preserving their written explanations; case notes and requests stay editable. Attaching current evidence allows a researcher to review the conclusion. Removal or loss of public visibility still prevents reading, exporting or saving the affected case. Source purge removes dependent case revisions and archived imported packets.

## Work that changes the investigation

- **Questions:** record the hypothesis, its next test, owner, disposition, supporting sources and counter-evidence. A supported/refuted disposition requires a cited local report whose public source still matches. It remains an explicit researcher assessment, not proof inferred from a task checkbox.
- **Records requests:** begin with the dossier's specific custodian, gap, resolution goal and draft. Record expected record types, fields and period; actual submission and response dates; and each production's receipt date, contents, period and sources. The app compares recorded production coverage with the stated expectations. Unknown fields or period coverage remain omissions. Follow-up drafts identify those omissions and retain the agency request reference. No request or follow-up is sent.
- **Decision windows:** retain a date actually quoted in the cited agenda. Keep the proposed decision separate from its eventual outcome and vote. Recording a decided outcome requires local cited evidence; the agenda alone does not establish approval.
- **Commitments and audit recommendations:** record the cited promise, responsible party, due date and completion test. Completion and noncompletion require cited evidence plus a written assessment. A passed due date with no completion evidence remains unknown.

Active unresolved cases affect the autonomous research fingerprint. The planner receives a bounded selection of hypotheses, next tests, production omissions, dated decisions and unresolved commitments so later investigations can pursue those gaps. Private case notes, contributor history and question-owner labels are excluded from that planner context. Research settings disclose the casework information sent to the selected model provider.

## Priority is explained

The lead queue takes the latest saved dossier per topic and explains four ranking factors: public consequence, available cited sources, whether the finding already appeared, and a related dated decision opportunity. Original query-source snapshots are deduplicated when counting evidence. The score prioritizes review; it is not a confidence score or a finding of wrongdoing. Shared source evidence and changed wording have explicit limitations in their explanations.

## Portable collaboration

A saved case exports as a versioned JSON packet with its revision base, source identity, contribution trail and contributor label. Later exports carry received contribution events forward with their source packet hashes. These labels and imported histories are not authenticated identities. The export is Ed25519-signed by the exporting installation's local key, and import verifies that signature before anything is previewed, rejecting a packet whose bytes changed after signing or whose signer this installation has since recorded as compromised; a routine (non-compromising) key rotation does not block it. That proves the file's origin and integrity, not the contributor's real-world identity.

Open a packet file or paste its JSON, then preview it. The preview classifies the packet as new, identical, compatible with the unchanged local base, or conflicting. A readable comparison shows changed, added and removed questions, tests, request scopes and productions, outcomes, commitments and evidence. Applying is a separate action, and the native layer rechecks both the local revision and packet digest. Conflicts cannot overwrite local work; compare the versions and merge selected edits into a new local revision. Raw JSON is available as an advanced view.

A source absent from local saved reports is explicitly unverified portable evidence. Local private or removed records are rejected. Imported completion claims without locally supported evidence return to unknown/open states. The original incoming packet is retained in compressed form with its digest for the local audit trail.

## Bounds

Portable packets and stored case payloads are limited to 2 MB before compression. The case list returns the 200 most recently updated cases, source attachment offers a bounded selection of recent dossier evidence, and portable contribution histories are bounded to 1,000 events and 40 received packets per case. Export refuses over-limit histories instead of silently truncating contributors; local history remains intact. The research planner uses bounded active-case context and retrieves fresh primary evidence before making another finding. These are local casework and portable collaboration features; they do not create a hosted multi-user service or send invitations.

## Verification

Native regression checks cover the pre-casework database shape, saved-report case creation, revision conflicts, scheduler context and privacy, exact SQL values/nulls, production gaps and follow-up drafts, agenda dates, commitment completion evidence, portable conflicts and contribution continuity, historical public replacements, retained quotations, needs-review assessments, unavailable/private sources and purge behavior. Frontend checks cover the actual editing/import workflow and readable differences. Removing async result guards makes the stale-case, stale-preview and stale-file tests fail; restoring them makes those tests pass.
