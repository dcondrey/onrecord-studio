import type { ResearchReport } from "../../types";
import { localDateInput } from "../../utils/briefing";
import { researchAgenda } from "../../utils/researchAgenda";
import { Citations } from "./ResearchEvidence";

export default function ResearchAgenda({
  reports,
  states,
  onOpen,
  meetingsOnly = false,
}: {
  reports: ResearchReport[];
  states: Record<string, string>;
  onOpen: (reportId: string, section: "requests" | "meetings", itemIndex?: number) => void;
  meetingsOnly?: boolean;
}) {
  const today = localDateInput(new Date());
  const actions = researchAgenda(reports, states, today).filter((action) => !meetingsOnly || action.section === "meetings");
  if (!actions.length) return null;
  return (
    <section
      aria-label="Investigation action queue"
      className="app-panel p-5 sm:p-6"
    >
      <div className="app-eyebrow text-amber-200">Follow the public decision</div>
      <h3 className="mt-2 text-xl font-semibold text-slate-100">
        {meetingsOnly ? "Dated actions in the record" : "Move the cases forward"}
      </h3>
      <p className="mt-2 text-label leading-relaxed text-slate-400">
        Dates below appear in cited, retained agenda text. They are not confirmed
        participation deadlines or proof that a decision occurred.
      </p>
      <div className="mt-4 grid gap-3 lg:grid-cols-2">
        {actions.slice(0, meetingsOnly ? 2 : 6).map((action) => (
          <article
            key={action.key}
            className="rounded-xl border border-slate-700 p-4"
          >
            <div className="flex flex-wrap gap-2 text-label leading-4 text-cyan-300">
              <span>
                {action.date
                  ? `${action.date} · ${action.date < today ? "Check the outcome" : "Verify current agenda"}`
                  : "Obtain missing evidence"}
              </span>
              {action.state === "IN_PROGRESS" && (
                <span className="text-amber-200">In progress</span>
              )}
            </div>
            <h4 className="mt-2 text-sm font-semibold text-slate-100">
              {action.title}
            </h4>
            <p className="mt-2 text-label leading-relaxed text-slate-300">
              {action.purpose}
            </p>
            {action.section === "meetings" && (() => {
              const report = reports.find((item) => item.id === action.reportId);
              const meeting = report?.synthesis.meetings[action.itemIndex];
              return report && meeting && <details className="mt-3 text-label leading-4"><summary className="cursor-pointer text-cyan-300">Read the dated agenda evidence</summary><div className="mt-2"><Citations citations={meeting.citations} report={report} /></div></details>;
            })()}
            <button
              type="button"
              className="analysis-button mt-3"
              onClick={() => onOpen(action.reportId, action.section, action.itemIndex)}
            >
              Open evidence and{" "}
              {action.section === "meetings"
                ? "meeting action"
                : "request draft"}
            </button>
          </article>
        ))}
      </div>
    </section>
  );
}
