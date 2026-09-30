import LadderDiagram from './LadderDiagram';

export function EvidenceLadderFigure() {
  return (
    <LadderDiagram
      id="evidence-ladder"
      title="Evidence ladder"
      description="Four rungs from weakest to strongest. A stand-in proxy, a pattern signal that is Unassessed, a pattern signal with measured confidence, and a legal claim that is a counted violation. A statement moves up only when a measurement supports it."
      topLabel="Strongest statement"
      bottomLabel="Weakest statement"
      rungs={[
        { label: 'Legal claim: counted violation', note: 'Counted against a statute or the agency’s own commitment', marker: 'diamond' },
        { label: 'Pattern signal, N% measured', note: 'Confidence derived by the evaluation', marker: 'ring' },
        { label: 'Pattern signal, Unassessed', note: 'Observed; no confidence measured (not 0%)', marker: 'dashed' },
        { label: 'Stand-in (proxy)', note: 'Used where the real event was not recorded', marker: 'square' },
      ]}
    />
  );
}

export function ProvenanceLadderFigure() {
  return (
    <LadderDiagram
      id="provenance-ladder"
      title="Provenance confidence ladder"
      description="Seven states from most to least trusted. Chain of custody verified, which this build never produces. Manifest signature verified. Local file hash verified. Recorded claim, unchecked. No recorded digest. Evidence file missing. Digest mismatch."
      topLabel="Most trusted"
      bottomLabel="Least trusted"
      rungs={[
        { label: 'Chain of custody verified', note: 'Never produced: no trust anchors configured', marker: 'diamond', unreachable: true },
        { label: 'Manifest signature verified', note: 'Signer not independently authenticated', marker: 'ring' },
        { label: 'Local file hash verified', note: 'Local integrity, not provenance', marker: 'ring' },
        { label: 'Recorded claim, unchecked', note: 'Metadata exists; nothing checked', marker: 'dashed' },
        { label: 'No recorded digest', note: 'Nothing about the stored bytes is known', marker: 'dashed' },
        { label: 'Evidence file missing', note: 'A custody gap, not an unrun check', marker: 'square' },
        { label: 'Digest mismatch', note: 'The file changed after ingestion', marker: 'triangle' },
      ]}
    />
  );
}
