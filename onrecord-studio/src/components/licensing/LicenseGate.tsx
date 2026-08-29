import type { LicenseStatus } from '../../types';

interface LicenseGateProps {
  status: LicenseStatus | null;
  refreshing: boolean;
  error: string | null;
  onRefresh: () => void;
}

function headingFor(status: LicenseStatus | null) {
  switch (status?.state) {
    case 'REVOKED':
      return 'Purchase no longer active';
    case 'UNVERIFIED':
    case 'BUNDLE_MISMATCH':
      return 'This copy could not be verified';
    case 'UNAVAILABLE':
      return 'App Store verification needed';
    default:
      return 'Verifying App Store purchase';
  }
}

export default function LicenseGate({ status, refreshing, error, onRefresh }: LicenseGateProps) {
  const loading = status === null;
  return (
    <main className="app-theme flex h-screen w-screen items-center justify-center bg-slate-950 px-6 text-slate-100">
      <section className="w-full max-w-lg rounded-2xl border border-slate-700 bg-slate-900/95 p-8 shadow-2xl" aria-live="polite">
        <p className="text-xs font-semibold uppercase tracking-[0.24em] text-cyan-400">OnRecord Studio</p>
        <h1 className="mt-3 text-2xl font-semibold">{headingFor(status)}</h1>
        <p className="mt-4 text-sm leading-6 text-slate-300">
          {status?.message ?? 'Checking Apple’s signed app transaction before opening the evidence workspace.'}
        </p>
        {error ? (
          <p className="mt-4 rounded-lg border border-rose-800 bg-rose-950/60 px-4 py-3 text-sm text-rose-200">
            {error}
          </p>
        ) : null}
        {!loading ? (
          <div className="mt-5 space-y-2 rounded-lg bg-slate-950/70 px-4 py-3 text-xs leading-5 text-slate-400">
            <p>Open the Mac App Store and sign in with the Apple Account that purchased this app.</p>
            <p>This is a one-time paid app; no subscription or separate license key is required.</p>
          </div>
        ) : null}
        <button
          type="button"
          disabled={loading || refreshing}
          onClick={onRefresh}
          className="mt-6 w-full rounded-lg bg-cyan-500 px-4 py-3 text-sm font-semibold text-slate-950 transition hover:bg-cyan-400 disabled:cursor-wait disabled:opacity-50"
        >
          {loading || refreshing ? 'Checking purchase…' : 'Verify purchase again'}
        </button>
        <p className="mt-4 text-center text-[11px] leading-4 text-slate-400">
          Workspace commands and local evidence remain locked until StoreKit verifies the paid app transaction.
        </p>
      </section>
    </main>
  );
}
