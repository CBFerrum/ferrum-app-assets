// One asset's whole story, newest first, from the records that hold it: where it went and who had
// it, what it was worth when that changed, what broke and what was done about it, when someone saw
// it, and how it left. Pure, so it is tested without a browser.

import { compare } from "./money";
import type { Asset } from "./types";

export interface MoveRow {
  transaction_date: string;
  movement: string;
  movement_id: string;
  purpose: string;
  from_location?: string | null;
  to_location?: string | null;
  from_custodian?: string | null;
  to_custodian?: string | null;
  due_back?: string | null;
}

export interface Adjustment {
  id: string;
  adjustment_date: string;
  current_asset_value?: string | null;
  new_asset_value?: string | null;
  difference_amount?: string | null;
  remarks?: string | null;
}

export interface Repair {
  id: string;
  failure_date: string;
  repair_status: string;
  completion_date?: string | null;
  repair_cost?: string | null;
  description?: string | null;
  capitalize_repair_cost?: boolean;
  docstatus?: number;
}

export interface Log {
  id: string;
  task_name?: string | null;
  due_date?: string | null;
  maintenance_status: string;
  completion_date?: string | null;
  actions_performed?: string | null;
}

export interface Check {
  id: string;
  verified_on: string;
  result: string;
  seen_at?: string | null;
  note?: string | null;
  verified_by?: string | null;
  _titles?: Record<string, string>;
}

export interface Disposal {
  id: string;
  disposal_type: string;
  disposal_date: string;
  proceeds?: string | null;
  gain_or_loss?: string | null;
}

export type Kind = "bought" | "inuse" | "move" | "handout" | "takeback" | "value" | "fault" | "repair" | "service" | "verified" | "missing" | "disposal";

export interface Entry {
  key: string;
  at: string;
  kind: Kind;
  title: string;
  detail?: string;
  tone?: "good" | "warn" | "bad";
  /** A record of this app: its path under the app. */
  to?: string;
  /** A record of another app, which opens where the workspace shows it. */
  open?: { entity: string; id: string };
}

export interface Format {
  money: (amount: string) => string;
  date: (day: string) => string;
}

export interface Story {
  asset: Asset;
  moves: MoveRow[];
  adjustments: Adjustment[];
  repairs: Repair[];
  logs: Log[];
  checks: Check[];
  disposal: Disposal | null;
}

/** Later in an asset's life, later in the list: what shares a day is told in this order. */
const RANK: Record<Kind, number> = { bought: 0, inuse: 1, move: 2, handout: 2, takeback: 2, verified: 3, missing: 3, service: 4, fault: 5, repair: 6, value: 7, disposal: 8 };

const join = (...parts: (string | false | null | undefined)[]) => parts.filter(Boolean).join(" · ") || undefined;

export function history({ asset, moves, adjustments, repairs, logs, checks, disposal }: Story, f: Format): Entry[] {
  const out: Entry[] = [];

  if (asset.purchase_date) {
    out.push({
      key: "bought",
      at: asset.purchase_date,
      kind: "bought",
      title: asset.purchase_invoice ? `Bought for ${f.money(asset.gross_purchase_amount ?? "0")}` : `Registered at ${f.money(asset.gross_purchase_amount ?? "0")}, already owned`,
      open: asset.purchase_invoice ? { entity: "PurchaseInvoice", id: asset.purchase_invoice } : undefined,
    });
  }
  if (asset.available_for_use_date && asset.available_for_use_date !== asset.purchase_date) {
    out.push({ key: "inuse", at: asset.available_for_use_date, kind: "inuse", title: "Put in use", detail: "Depreciation starts" });
  }

  for (const m of moves) {
    const base = { key: `m-${m.movement_id}-${m.transaction_date}`, at: m.transaction_date, to: `e/AssetMovement/${m.movement_id}` };
    if (m.purpose === "Issue") {
      out.push({
        ...base,
        kind: "handout",
        title: `Handed to ${m.to_custodian ?? "a custodian"}`,
        detail: join(m.due_back ? `due back ${f.date(m.due_back)}` : "no return date", m.to_location && m.to_location !== m.from_location && `at ${m.to_location}`),
      });
    } else if (m.purpose === "Receipt") {
      out.push({ ...base, kind: "takeback", title: `Taken back from ${m.from_custodian ?? "its custodian"}`, detail: m.to_location ? `into ${m.to_location}` : undefined });
    } else {
      const handedOn = m.to_custodian && m.to_custodian.toLowerCase() !== (m.from_custodian ?? "").toLowerCase();
      out.push({
        ...base,
        kind: "move",
        title: `Moved to ${m.to_location ?? "another place"}`,
        detail: join(m.from_location && `from ${m.from_location}`, handedOn && `now with ${m.to_custodian}`),
      });
    }
  }

  for (const a of adjustments) {
    const down = compare(a.difference_amount, "0") < 0;
    out.push({
      key: `v-${a.id}`,
      at: a.adjustment_date,
      kind: "value",
      title: `${down ? "Written down" : "Written up"} to ${f.money(a.new_asset_value ?? "0")}`,
      detail: join(a.current_asset_value != null && `from ${f.money(a.current_asset_value)}`, a.remarks),
      tone: down ? "warn" : "good",
      to: `e/AssetValueAdjustment/${a.id}`,
    });
  }

  for (const r of repairs) {
    if (r.repair_status === "Cancelled" || r.docstatus === 2) continue;
    const cost = compare(r.repair_cost, "0") > 0 ? `cost ${f.money(r.repair_cost ?? "0")}${r.capitalize_repair_cost ? ", added to its value" : ""}` : undefined;
    if (r.repair_status === "Completed") {
      out.push({ key: `r-${r.id}`, at: r.completion_date ?? r.failure_date, kind: "repair", title: `Repaired${r.description ? `: ${r.description}` : ""}`, detail: join(`fault reported ${f.date(r.failure_date)}`, cost), tone: "good", to: `e/AssetRepair/${r.id}` });
    } else {
      out.push({ key: `r-${r.id}`, at: r.failure_date, kind: "fault", title: `Fault reported${r.description ? `: ${r.description}` : ""}`, detail: "out of order until it is repaired", tone: "bad", to: `e/AssetRepair/${r.id}` });
    }
  }

  for (const l of logs) {
    if (l.maintenance_status === "Completed") {
      out.push({ key: `l-${l.id}`, at: l.completion_date ?? l.due_date ?? "", kind: "service", title: `${l.task_name ?? "Maintenance"} done`, detail: l.actions_performed ?? undefined, tone: "good", to: `e/AssetMaintenanceLog/${l.id}` });
    } else if (l.maintenance_status === "In Progress") {
      out.push({ key: `l-${l.id}`, at: l.due_date ?? "", kind: "service", title: `${l.task_name ?? "Maintenance"} under way`, tone: "warn", to: `e/AssetMaintenanceLog/${l.id}` });
    }
  }

  for (const c of checks) {
    const found = c.result !== "Not Found";
    const where = c._titles?.seen_at;
    out.push({
      key: `c-${c.id}`,
      at: c.verified_on,
      kind: found ? "verified" : "missing",
      title: found ? `Verified${where ? ` at ${where}` : ""}` : "Not found",
      detail: join(c.note, c.verified_by && `by ${c.verified_by}`),
      tone: found ? "good" : "bad",
      to: `e/AssetVerification/${c.id}`,
    });
  }

  if (disposal) {
    const sold = disposal.disposal_type === "Sell";
    const gain = compare(disposal.gain_or_loss, "0");
    out.push({
      key: `d-${disposal.id}`,
      at: disposal.disposal_date,
      kind: "disposal",
      title: sold ? `Sold for ${f.money(disposal.proceeds ?? "0")}` : "Scrapped",
      detail: gain === 0 ? undefined : `${gain > 0 ? "gain" : "loss"} of ${f.money((disposal.gain_or_loss ?? "0").replace("-", ""))}`,
      tone: gain < 0 ? "bad" : undefined,
      to: `e/AssetDisposal/${disposal.id}`,
    });
  }

  return out.filter((e) => e.at).sort((a, b) => b.at.localeCompare(a.at) || RANK[b.kind] - RANK[a.kind] || a.key.localeCompare(b.key));
}
