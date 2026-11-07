// An asset's maintenance: the plan's tasks with when each falls due next, the work planned or under
// way, and the faults not yet repaired. Logging work done is one step from the task it is for.

import { Link } from "react-router";
import { Button, EmptyState, ErrorState, Spinner } from "@ferrum/kit";
import { phrase, urgency } from "./dates";
import { dayText } from "./format";
import { type Task, useOpenWork, usePlan } from "./plan";
import type { Asset } from "./types";
import { t as tr } from "@ferrum/client";

/** When a task falls due next, as a chip: red once late, amber within a month. */
function Due({ date, today }: { date?: string | null; today: string }) {
  if (!date) return <span className="faint">{tr("no date")}</span>;
  const how = urgency(date, today);
  return (
    <span className={`badge ${how === "late" ? "badge-red" : how === "soon" ? "badge-amber" : ""}`} title={dayText(date)}>
      {phrase(date, today, "due", dayText)}
    </span>
  );
}

export function Maintenance({ asset, today, can, onLog, onFault }: { asset: Asset; today: string; can: { log: boolean; plan: boolean; fault: boolean }; onLog: (task?: string) => void; onFault: () => void }) {
  const plan = usePlan(asset.id);
  const work = useOpenWork(asset.id);
  if (plan.error) return <ErrorState error={plan.error} onRetry={() => void plan.refetch()} />;
  if (plan.isLoading || work.isLoading) return <Spinner label={tr("Loading its maintenance")} />;
  const tasks: Task[] = plan.data?.tasks ?? [];
  const named = new Map(tasks.map((t) => [t.id, t.task_name]));
  const logs = work.data?.logs ?? [];
  const faults = work.data?.faults ?? [];

  return (
    <div className="stack ast-service">
      {faults.length > 0 && (
        <section aria-label={tr("Faults")}>
          <h3 className="ast-subhead">{tr("Faults to repair")}</h3>
          <ul className="ast-rows">
            {faults.map((f) => (
              <li key={f.id}>
                <span>
                  <Link to={`/e/AssetRepair/${f.id}`}>{f.description || "Fault"}</Link> <span className="faint">since {dayText(f.failure_date)}</span>
                </span>
                <span className="badge badge-red">{tr("Out of order")}</span>
              </li>
            ))}
          </ul>
          <p className="faint">{tr("Open a fault to say when it was repaired and what it cost.")}</p>
        </section>
      )}
      <section aria-label={tr("Plan")}>
        <div className="ast-subhead-row">
          <h3 className="ast-subhead">{tr("Plan")}</h3>
          <span className="spacer" />
          {plan.data && can.log && (
            <Button size="sm" icon="check" onClick={() => onLog()}>
              {tr("Log work done")}
            </Button>
          )}
          {can.fault && (
            <Button size="sm" icon="alert" onClick={onFault}>
              {tr("Report a fault")}
            </Button>
          )}
        </div>
        {!plan.data ? (
          <EmptyState title={tr("No maintenance plan")} icon="calendar">
            <p className="muted">{tr("A plan lists what this asset needs and how often. The overview then shows what falls due.")}</p>
            {can.plan && (
              <Link className="btn btn-primary btn-md" to={`/e/AssetMaintenance/new?asset=${asset.id}`}>
                {tr("Plan maintenance")}
              </Link>
            )}
          </EmptyState>
        ) : (
          <div className="table-wrap">
            <table className="table">
              <thead>
                <tr>
                  <th scope="col">{tr("Task")}</th>
                  <th scope="col">{tr("Every")}</th>
                  <th scope="col">{tr("Last done")}</th>
                  <th scope="col">{tr("Next due")}</th>
                  <th scope="col">
                    <span className="sr-only">{tr("Actions")}</span>
                  </th>
                </tr>
              </thead>
              <tbody>
                {tasks.map((t) => (
                  <tr key={t.id}>
                    <td>
                      <strong>{t.task_name}</strong>
                      {(t.maintenance_type || t.assign_to) && <div className="faint">{[t.maintenance_type, t.assign_to].filter(Boolean).join(" · ")}</div>}
                    </td>
                    <td>{t.periodicity?.toLowerCase()}</td>
                    <td className="nowrap">{t.last_completion_date ? dayText(t.last_completion_date) : <span className="faint">never</span>}</td>
                    <td>
                      <Due date={t.next_due_date} today={today} />
                    </td>
                    <td className="num">
                      {can.log && (
                        <button type="button" className="btn btn-ghost btn-sm" onClick={() => onLog(t.id)}>
                          {tr("Log done")}
                        </button>
                      )}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </section>
      {logs.length > 0 && (
        <section aria-label={tr("Work open")}>
          <h3 className="ast-subhead">{tr("Work planned or under way")}</h3>
          <ul className="ast-rows">
            {logs.map((l) => (
              <li key={l.id}>
                <span>
                  <Link to={`/e/AssetMaintenanceLog/${l.id}`}>{l.task_name ?? named.get(l.task ?? "") ?? "Maintenance"}</Link> <span className="faint">{l.maintenance_status.toLowerCase()}</span>
                </span>
                <span className="row">
                  <Due date={l.due_date} today={today} />
                  {can.log && l.task && (
                    <button type="button" className="btn btn-ghost btn-sm" onClick={() => onLog(l.task ?? undefined)}>
                      {tr("Mark done")}
                    </button>
                  )}
                </span>
              </li>
            ))}
          </ul>
        </section>
      )}
    </div>
  );
}
