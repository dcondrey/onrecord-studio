# Deobfuscation validation and cross-record reconstruction

The existing corpus was audited read-only: **91 files, including 60 PDFs and 1,336 PDF pages**. Every file's bytes were checked before and after analysis; no source file changed. Ninety files had recorded source digests, which matched; the remaining pending image had no recorded digest. Native geometry, residual-evidence and historical-version collectors ran without tool errors. This does not establish complete forensic coverage.

## What the existing files actually demonstrated

The original geometry implementation reported 76 intersections in three PDFs, across eight pages:

| Original record | Pages flagged | Intersections | Rendered review |
| --- | --- | ---: | --- |
| [CPP July 1 agenda](https://www.sandiego.gov/sites/default/files/2026-06/cpp-agenda-260701.pdf) | 17, 18, 22, 23, 24 | 42 | Ordinary slide backgrounds and readable design elements; no concealed information established. |
| [CPP Community Outreach June 4 agenda](https://www.sandiego.gov/sites/default/files/2026-06/cpp-community-outreach-committee-agenda-260604.pdf) | 47, 52 | 3 | Ordinary readable slides. |
| [CPP Recruitment May 7 agenda](https://www.sandiego.gov/sites/default/files/2026-05/cpp-recruitment-committee-agenda-260507.pdf) | 27 | 31 | A cover/title composition; the extracted title resembles visible cover content. No useful concealed information established. |

All eight flagged pages were rendered and visually inspected. **These flags are not evidence of successful unredaction.** The viewer now explains that backgrounds and other layout elements can trigger intersections. Ten PDFs had incomplete geometry coverage in the initial audit. The final pass explicitly reports unsupported CID font mappings and records 10 PDFs with incomplete coverage. That is an honest accounting of previously unreported limits, not evidence of concealed text.

Five PDFs yielded candidate historical text. Comparing with the current document using the same native extractor found three exact duplicates: `DisruptingChurnReport.pdf`, `HSEEGovernment.pdf`, and `cpp-agenda-260819.pdf`. The toolkit differed in character count by four characters; that difference was not established as concealed content. The Opportunity Zones report's historical extraction was truncated and cannot support a complete comparison. Extra EOF markers are candidate boundaries, not proof of a substantive earlier revision.

Machine-readable evidence: [initial corpus results](forensic-corpus-baseline.json), [final audit](forensic-corpus-final.json), [reviewed page geometry](forensic-flagged-pages.json), [revision comparisons](forensic-revision-comparisons.json). These are test artifacts, not copies of the underlying documents.

## Implemented improvements

1. **Actual PDF font mappings.** Positioned text now follows font selection, inherited Form resources, `/ToUnicode`, and graphics-state restoration. The former implementation interpreted encoded glyph bytes without the active font. Regression fixtures cover different mappings for the same bytes. CID glyph indices without a Unicode map are now treated as unsupported rather than being displayed as plausible-looking letters. Embedded-font cmap recovery remains outside this bounded pass.
2. **Public-role linkage.** Masked sender/recipient headers are connected to public roles elsewhere in the same extraction and explicitly named role holders in related records. Organization and dated tenure are checked separately. An incompatible tenure is displayed as contradicted; a current title does not establish a historic sender. Assistants, delegation and shared mailboxes remain possible.
3. **Complementary numeric fragments.** Public contract, grant, invoice, payment and similar fields are compared only with matching nearby record keys and no conflicting shared keys. Visible digits are constraints; no missing digit is fabricated. Unknown mask widths stay unknown. The analyst can explicitly supply a verified one-character-per-mask convention. Conflicting complete values remain competing candidates.
4. **Parallel passages.** The engine finds unmasked text between matching left/right context, including later in the same document. Multiple completions remain ambiguous; reused templates and amendments are explicit alternative explanations.
5. **Reviewable evidence and follow-through.** Each candidate includes source links, hashes, extraction character locators, quoted passages, supporting constraints, limitations and a targeted records-request draft. Search uses retained text and works after originals have been purged. Byte-level PDF audits still require an original. The research pipeline can pursue a bounded set of reconstruction leads, while only the actual source quotations become citable evidence.

## Validation and limits

The new constraint-engine regression tests exercise complementary and disjoint masks, unknown lengths, contradictory digits, conflicting record keys, ambiguous completions, duplicate releases, historical office holders, unrelated organizations, within-document matches, Unicode character locators and invented context rejection. These are controlled fixtures, **not claims that those scenarios were found in the existing corpus**.

The document viewer has identity guards for old results and old failures. Removing those guards made both race tests fail; the guards were restored. Storage tests cover retained-only records and source eligibility/identity checks.

Automatic detection currently relies on explicit masking markers and recognizable public-role/field formats. Image-only bars require usable OCR or related extracted text. Retrieval is bounded; lack of a match is not proof that no unredacted copy exists. Similar prose or record keys can still describe different versions or transactions, so assembled text is labeled as an inference and never substituted into the original record.

## Research basis

- [PyMuPDF's redaction documentation](https://pymupdf.readthedocs.io/en/latest/page.html#Page.apply_redactions) distinguishes a redaction annotation from removing the underlying content. Correctly removed content cannot be recovered from that saved text layer.
- [NIST SP 800-188](https://nvlpubs.nist.gov/nistpubs/SpecialPublications/NIST.SP.800-188.pdf) describes how auxiliary information and combinations of attributes affect identifiability. Here, that informs the separate treatment of title, organization and time constraints.
- [Composition Attacks and Auxiliary Information in Data Privacy](https://arxiv.org/abs/0803.0032) studies how combining releases changes what can be inferred. Here, independently visible fragments are compared without assuming that a plausible alignment is proven.

The implemented scope is public-business evidence and public official roles, not reconstruction of private contact, banking, patient or beneficiary identifiers.

Final regression verification: 390 native tests and 153 frontend tests passed; both new UI race guards and the storage source-identity guard were mutation-proven. The final all-corpus audit passed again in 72.71 seconds, with all source bytes unchanged.
