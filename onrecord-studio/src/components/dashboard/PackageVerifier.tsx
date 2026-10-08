import { useRef, useState } from 'react';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import type { PackageVerificationResult } from '../../types/investigation';
import Button from '../ui/Button';
import InlineError from '../ui/InlineError';

export default function PackageVerifier() {
  const gate = useLatestRequestGate();
  const [result, setResult] = useState<PackageVerificationResult | null>(null);
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const [fileName, setFileName] = useState('');
  const [chosenName, setChosenName] = useState('');
  const inputRef = useRef<HTMLInputElement>(null);

  async function verify(file: File) {
    const token = gate.begin();
    setBusy(true);
    setError('');
    setResult(null);
    try {
      const text = await file.text();
      const value = await invokeCommand('verify_evidence_package', { payload: text });
      if (gate.isCurrent(token)) {
        setResult(value);
        setFileName(file.name);
      }
    } catch (reason) {
      if (gate.isCurrent(token)) setError(formatNativeError(reason));
    } finally {
      if (gate.isCurrent(token)) setBusy(false);
    }
  }

  return (
    <section aria-labelledby="package-verifier-title" className="rounded-lg border border-line p-4">
      <h4 id="package-verifier-title" className="text-body font-medium text-fg">Verify a signed chain-of-custody or evidence package</h4>
      <p className="mt-1 max-w-prose text-caption text-muted">
        Confirms the file hasn’t changed since whoever exported it signed it. It doesn’t authenticate
        the signer’s real-world identity, and it isn’t a legal determination of admissibility.
      </p>
      <div className="mt-3 flex min-w-0 items-center gap-3">
        <input
          ref={inputRef}
          type="file"
          accept=".json,application/json"
          aria-label="Open a signed package (.json)"
          className="sr-only"
          tabIndex={-1}
          disabled={busy}
          onChange={(event) => {
            const file = event.target.files?.[0];
            if (!file) return;
            setChosenName(file.name);
            if (file.size > 64 * 1024 * 1024) {
              gate.invalidate();
              setBusy(false);
              setError('Package files must be at most 64 MB.');
              return;
            }
            void verify(file);
          }}
        />
        <Button size="sm" disabled={busy} onClick={() => inputRef.current?.click()}>{busy ? 'Verifying…' : 'Choose package…'}</Button>
        {chosenName && !result && <span className="min-w-0 truncate font-mono text-code text-muted" title={chosenName}>{chosenName}</span>}
      </div>
      {error && (error.startsWith('Package files must') ? <p role="alert" className="mt-3 text-body text-danger">{error}</p> : <InlineError className="mt-3" summary="The package couldn’t be verified." detail={error} />)}
      {result && (
        <div className="mt-3 rounded-lg border border-line bg-canvas p-3 text-body">
          <p className="font-mono text-code text-fg">{fileName}</p>
          {result.parseError ? (
            <p className="mt-2 text-danger">{result.parseError}</p>
          ) : (
            <>
              <ul className="mt-2 space-y-1">
                <li className={result.canonicalBytesHashMatches ? 'text-success' : 'text-danger'}>
                  {result.canonicalBytesHashMatches ? 'PASS' : 'FAIL'} — canonical bytes hash to the recorded manifest hash
                </li>
                <li className={result.signatureValid ? 'text-success' : 'text-danger'}>
                  {result.signatureValid ? 'PASS' : 'FAIL'} — Ed25519 signature verifies against the embedded public key
                </li>
                <li className={result.manifestMatchesCanonicalBytes ? 'text-success' : 'text-danger'}>
                  {result.manifestMatchesCanonicalBytes ? 'PASS' : 'FAIL'} — the readable manifest matches the signed bytes
                </li>
              </ul>
              <p className="mt-2 text-muted">Schema: {result.schemaVersion ?? 'unknown'}</p>
              <p className="mt-1 text-muted">Identity assurance: {result.identityAssurance ?? 'unknown'} — a self-managed local key proves only that whoever held it signed these bytes, not a notarized identity.</p>
              {result.unverifiedCitationLabels.length > 0 && (
                <div className="mt-2 rounded-md border border-warning/30 bg-warning/10 p-2 text-warning">
                  <p className="font-medium">{result.unverifiedCitationLabels.length} citation{result.unverifiedCitationLabels.length === 1 ? '' : 's'} inside this package are themselves marked unverified:</p>
                  <ul className="mt-1 list-disc pl-4">
                    {result.unverifiedCitationLabels.map((label) => <li key={label}>{label}</li>)}
                  </ul>
                </div>
              )}
              {result.withheldCitationLabels.length > 0 && (
                <div className="mt-2 rounded-md border border-danger/30 bg-danger/10 p-2 text-danger">
                  <p className="font-medium">{result.withheldCitationLabels.length} citation{result.withheldCitationLabels.length === 1 ? '' : 's'} withheld from this package's evidence, by label and reason:</p>
                  <ul className="mt-1 list-disc pl-4">
                    {result.withheldCitationLabels.map((label) => <li key={label}>{label}</li>)}
                  </ul>
                </div>
              )}
              {result.revokedSigner && result.revokedSigner.status === 'RETIRED' && (
                <div className="mt-2 rounded-md border border-line bg-raised p-2 text-fg-secondary">
                  <p className="font-medium">Signer key retired by this installation on {result.revokedSigner.revokedAt} ({result.revokedSigner.reason}).</p>
                  <p className="mt-1">This was a routine key rotation, not a compromise. Packages this key already signed, including this one, remain trusted.</p>
                </div>
              )}
              {result.revokedSigner && result.revokedSigner.status !== 'RETIRED' && (
                <div className="mt-2 rounded-md border border-danger/30 bg-danger/10 p-2 text-danger">
                  <p className="font-medium">Signer key revoked as compromised by this installation on {result.revokedSigner.revokedAt} ({result.revokedSigner.reason}).</p>
                  <p className="mt-1">The signature over these bytes is still mathematically valid — that check above is unaffected — but this signer is no longer trusted; treat the package as untrusted regardless of the other checks.</p>
                </div>
              )}
              <p className={`mt-3 font-medium ${result.overallPassed ? 'text-success' : 'text-danger'}`}>
                {result.overallPassed ? 'Overall: PASS — internally consistent and unaltered since signing.' : 'Overall: FAIL — treat this package as unverified.'}
              </p>
            </>
          )}
        </div>
      )}
    </section>
  );
}
