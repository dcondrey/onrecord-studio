export interface ReportedFailure {
  message: string;
  reportedAt: number;
}

/** Reports a changed failure immediately and an unchanged one only at a bounded reminder rate. */
export function shouldReportFailure(
  previous: ReportedFailure | null,
  message: string,
  now: number,
  reminderMilliseconds: number,
): boolean {
  return (
    previous === null ||
    previous.message !== message ||
    now - previous.reportedAt >= reminderMilliseconds
  );
}
