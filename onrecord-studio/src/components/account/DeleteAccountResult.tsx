import type { DeleteAccountReport } from '../../types';

/** States what each part of deletion did. "Deleted" is claimed only for the parts that report it. */
export default function DeleteAccountResult({
  report,
  error,
  onDismiss,
}: {
  report: DeleteAccountReport | null;
  error: string | null;
  onDismiss: () => void;
}) {
  if (!report && !error) return null;
  const complete = report ? report.remoteDeleted && report.signedOut && report.localCleared : false;
  return (
    <div
      role={complete ? 'status' : 'alert'}
      className="mt-4 rounded-lg border border-slate-600 bg-slate-950/80 px-4 py-3 text-label leading-5 text-slate-200"
    >
      {report ? (
        <>
          <p className="font-semibold text-slate-100">
            {complete ? 'Account deletion finished.' : 'Account deletion did not fully finish.'}
          </p>
          {!complete && report.remoteDeleted ? (
            <p className="mt-1">
              Your account no longer exists on the server, but this Mac still holds part of your sign-in. Finish by
              signing out here.
            </p>
          ) : null}
          <ul className="mt-2 list-disc space-y-1 pl-4">
            <li>
              {report.remoteDeleted
                ? 'Done: your license account was deleted on the server.'
                : 'Not done: your license account was NOT deleted on the server.'}
            </li>
            <li>
              {report.signedOut
                ? 'Done: this Mac was signed out.'
                : 'Not done: this Mac was NOT signed out.'}
            </li>
            <li>
              {report.localCleared
                ? 'Done: sign-in data on this Mac was removed.'
                : 'Not done: sign-in data on this Mac was NOT fully removed.'}
            </li>
            <li>Not touched: local evidence on this Mac was not deleted.</li>
          </ul>
          {report.notDone.length > 0 ? (
            <div className="mt-2">
              <p className="font-medium">Reported as not done:</p>
              <ul className="list-disc pl-4">
                {report.notDone.map((item) => (
                  <li key={item}>{item}</li>
                ))}
              </ul>
            </div>
          ) : null}
        </>
      ) : (
        <p>Account deletion failed and nothing is confirmed as deleted. {error}</p>
      )}
      <button
        type="button"
        onClick={onDismiss}
        className="focus-ring mt-3 rounded-lg border border-slate-500 px-3 py-1 font-semibold text-slate-100 hover:bg-slate-800"
      >
        Dismiss
      </button>
    </div>
  );
}
