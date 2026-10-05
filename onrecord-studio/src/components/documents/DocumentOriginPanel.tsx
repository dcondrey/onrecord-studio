import type {
  DocumentInternalMetadata,
  DocumentOriginReport,
  HeldComparison,
  HeldDocumentMatch,
  InternalMetadataUnreadReason,
  NotComparedReason,
} from '../../types';

const NOT_COMPARED: Record<NotComparedReason, string> = {
  FILE_DIGEST_NOT_RECORDED: 'no SHA-256 of this file is recorded yet',
  TEXT_NOT_EXTRACTED: 'no text has been extracted from this file',
  NOT_IN_SIMILARITY_INDEX: 'the similarity index does not hold this file’s text yet',
};

const NOT_READ: Record<InternalMetadataUnreadReason, string> = {
  FILE_UNAVAILABLE: 'Not read: the stored file is not available.',
  UNSUPPORTED_FORMAT: 'Not read: internal metadata is read from PDF and Office (docx, xlsx, pptx) files only.',
  UNREADABLE: 'Not read: the file could not be parsed as the format its name gives.',
};

function describeHeld(held: HeldDocumentMatch): string {
  const request = held.requestLabel ? `request ${held.requestLabel}` : `request ${held.requestId}`;
  return `${held.sourceName} · ${request} · ${held.fileName ?? 'unnamed file'} (document ${held.documentId})`;
}

function Comparison<T>({ title, comparison, comparedNoun, describe }: {
  title: string;
  comparison: HeldComparison<T>;
  comparedNoun: string;
  describe: (match: T) => string;
}) {
  if (comparison.state === 'NOT_COMPARED') {
    return <p><span className="font-medium text-fg-secondary">{title}:</span> not compared, {NOT_COMPARED[comparison.reason]}.</p>;
  }
  if (comparison.matches.length === 0) {
    return <p><span className="font-medium text-fg-secondary">{title}:</span> no match among {comparison.comparedCount} {comparedNoun}.</p>;
  }
  return (
    <div>
      <p className="font-medium text-fg-secondary">{title}</p>
      <ul className="ml-4 list-disc">
        {comparison.matches.map((match) => <li key={describe(match)}>{describe(match)}</li>)}
      </ul>
      {comparison.truncated && <p>More matched than are listed here.</p>}
    </div>
  );
}

function InternalMetadata({ metadata, failed }: { metadata: DocumentInternalMetadata | null; failed: boolean }) {
  let body;
  if (failed) body = <p role="alert" className="text-danger">The internal metadata could not be read.</p>;
  else if (!metadata) body = <p>Reading internal metadata…</p>;
  else if (metadata.state === 'NOT_READ') body = <p>{NOT_READ[metadata.reason]}</p>;
  else {
    body = (
      <dl className="grid grid-cols-[max-content_1fr] gap-x-3">
        {metadata.fields.map((field) => (
          <div key={field.name} className="contents">
            <dt className="font-mono">{field.name}</dt>
            <dd className={field.value === null ? 'text-muted' : 'break-all text-fg-secondary'}>{field.value ?? 'not recorded'}</dd>
          </div>
        ))}
      </dl>
    );
  }
  return (
    <div>
      <p className="font-medium text-fg-secondary">Internal metadata</p>
      <p>These fields are written by whoever made the file and can be set to anything. OnRecord reads them and verifies none of them.</p>
      {body}
    </div>
  );
}

/** Shown only for a file the user added by hand. */
export default function DocumentOriginPanel({ report, metadata, metadataFailed }: {
  report: DocumentOriginReport;
  metadata: DocumentInternalMetadata | null;
  metadataFailed: boolean;
}) {
  const relation = report.heldRelation;
  return (
    <section aria-label="File origin" className="space-y-2 border-b border-line px-4 py-3 text-caption text-muted">
      <p className="text-label font-semibold text-warning">
        Added by you{report.sourceKind === 'PASTED_CONTENT' ? ' as pasted text' : ' from a local file'}. Not downloaded from an agency source.
      </p>
      <p>
        OnRecord did not obtain this file from an agency, so it is not counted as an agency release and no finding about what an agency released, withheld or said is drawn from it.
        A match below says this file matches a file a source served. It does not say this file is authentic, and no match does not say it is fabricated.
      </p>
      {relation && (
        <>
          <Comparison title="Exact copies (same file SHA-256)" comparison={relation.exactCopies} comparedNoun="collected files with a recorded SHA-256" describe={describeHeld} />
          <Comparison title="Same extracted text, not an exact copy" comparison={relation.sameText} comparedNoun="collected files with extracted text" describe={describeHeld} />
          <Comparison
            title="Similar text"
            comparison={relation.nearDuplicates}
            comparedNoun="other documents in the similarity index"
            describe={(near) => `${describeHeld(near.document)} · fingerprint differs in ${near.simhashDistance} of 64 bits`}
          />
        </>
      )}
      <InternalMetadata metadata={metadata} failed={metadataFailed} />
    </section>
  );
}
