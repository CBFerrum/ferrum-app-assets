// An asset's whole story on one timeline: where it went and who had it, what it was worth when that
// changed, faults and repairs, maintenance, every time someone saw it, and how it left. Each entry
// opens the record behind it. Each kind of record is asked for on its own, so someone who may not
// read one (a custodian cannot read revaluations) still sees the rest.

import { useQuery } from "@tanstack/react-query";
import { Link } from "react-router";
import { api, ApiError, docPath, todayIso } from "@ferrum/client";
import { EmptyState, ErrorState, Icon, Spinner } from "@ferrum/kit";
import { list, report } from "./data";
import { dayText } from "./format";
import { type Adjustment, type Check, type Disposal, type Entry, type Kind, type Log, type MoveRow, type Repair, history } from "./history";
import type { Asset } from "./types";

const ICON: Record<Kind, string> = {
  bought: "file",
  inuse: "check",
  move: "timeline",
  handout: "user",
  takeback: "user",
  value: "chart",
  fault: "alert",
  repair: "check",
  service: "calendar",
  verified: "check",
  missing: "alert",
  disposal: "x",
};

/** A kind of record this person may not read is left out of the story, not made an error of it. */
const allowed = <T,>(request: Promise<T>, none: T): Promise<T> =>
  request.catch((e) => {
    if (e instanceof ApiError && (e.status === 403 || e.status === 404)) return none;
    throw e;
  });

const notCancelled = [["docstatus", "<", 2]];

export function Story({ asset, money }: { asset: Asset; money: (v: unknown) => string }) {
  const q = useQuery({
    queryKey: ["assets", "story", asset.id, asset.version],
    queryFn: async ({ signal }) => {
      const mine = ["asset", "=", asset.id];
      const today = todayIso();
      // The report wants its start no later than its end (an asset bought for a later date is rare, not unheard of).
      const from = asset.purchase_date && asset.purchase_date <= today ? asset.purchase_date : "1900-01-01";
      const [moves, adjustments, repairs, logs, checks, disposal] = await Promise.all([
        allowed(report<MoveRow>("asset_movement_history", { ...(asset.company ? { company: asset.company } : {}), asset: asset.id, from_date: from, to_date: today }, signal).then((r) => r.rows), []),
        allowed(list<Adjustment>("AssetValueAdjustment", { filters: [mine, ["docstatus", "=", 1]], fields: "adjustment_date,current_asset_value,new_asset_value,difference_amount,remarks", limit: 200 }, signal).then((r) => r.rows), []),
        allowed(list<Repair>("AssetRepair", { filters: [mine, ...notCancelled], fields: "failure_date,repair_status,completion_date,repair_cost,description,capitalize_repair_cost,docstatus", limit: 200 }, signal).then((r) => r.rows), []),
        allowed(list<Log>("AssetMaintenanceLog", { filters: [mine, ...notCancelled], fields: "task_name,due_date,maintenance_status,completion_date,actions_performed", limit: 200 }, signal).then((r) => r.rows), []),
        allowed(list<Check>("AssetVerification", { filters: [mine, ["docstatus", "=", 1]], fields: "verified_on,result,seen_at,note,verified_by", titles: true, limit: 200 }, signal).then((r) => r.rows), []),
        asset.disposal ? allowed(api.get<Disposal>(docPath("AssetDisposal", asset.disposal), undefined, signal), null) : Promise.resolve(null),
      ]);
      return { moves, adjustments, repairs, logs, checks, disposal };
    },
  });
  if (q.error) return <ErrorState error={q.error} onRetry={() => void q.refetch()} />;
  if (!q.data) return <Spinner label="Loading its history" />;
  const entries = history({ asset, ...q.data }, { money, date: dayText });
  if (!entries.length) return <EmptyState title="Nothing has happened yet" icon="timeline" />;
  return (
    <ol className="ast-story">
      {entries.map((e: Entry) => (
        <li key={e.key} data-tone={e.tone}>
          <span className="ast-story-icon" aria-hidden="true">
            <Icon name={ICON[e.kind]} size={13} />
          </span>
          <div className="ast-story-body">
            <div className="ast-story-line">
              {e.to ? <Link to={`/${e.to}`}>{e.title}</Link> : e.open ? <a href={`/open/${e.open.entity}/${e.open.id}`}>{e.title}</a> : <strong>{e.title}</strong>}
              <time className="faint" dateTime={e.at}>
                {dayText(e.at)}
              </time>
            </div>
            {e.detail && <p className="faint">{e.detail}</p>}
          </div>
        </li>
      ))}
    </ol>
  );
}
