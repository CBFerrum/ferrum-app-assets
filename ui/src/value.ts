// An asset's book value over time: what it was worth after each period booked, each revaluation and
// capitalized repair, and what the plan says it will be worth after each period still to come.
//
// It is worked out from where the asset stands now (the server's net book value) in both directions,
// so it ends exactly where the server says whatever happened: back through the booked periods and
// the changes of value (the server books depreciation due by a change's date first), forward through
// the planned ones. Pure, so it is tested without a browser.

import { decimal, minus, units } from "./money";
import type { Asset, ScheduleRow } from "./types";

export type PointKind = "start" | "booked" | "due" | "planned" | "failed" | "adjustment" | "repair" | "disposal";

export interface Point {
  key: string;
  date: string;
  /** The value after this point, exactly. */
  value: string;
  /** What it changed (negative = the asset lost value), exactly. */
  change: string;
  kind: PointKind;
  /** In the books: booked depreciation, a change of value, the start or the disposal. */
  booked: boolean;
  label: string;
  /** The journal entry that booked a period. */
  entry?: string | null;
  /** Why a period could not be booked. */
  error?: string | null;
}

/** A change of value other than depreciation: a revaluation or a capitalized repair. */
export interface ValueEvent {
  date: string;
  /** Signed: a write-down is negative. */
  amount: string;
  kind: "adjustment" | "repair";
  label: string;
}

export interface Series {
  points: Point[];
  /** The line starts at what the asset cost; false when changes of value the person may not read
   *  could be missing, so the early part is only as good as what is known. */
  exact: boolean;
}

type Input = Pick<
  Asset,
  "schedule" | "value_after_depreciation" | "gross_purchase_amount" | "opening_accumulated_depreciation" | "available_for_use_date" | "purchase_date" | "disposal" | "disposal_date" | "status"
>;

const day = (iso: string | null | undefined) => (iso ?? "").slice(0, 10);

const byDate = (a: ScheduleRow, b: ScheduleRow) => a.schedule_date.localeCompare(b.schedule_date) || Number(!!b.booked) - Number(!!a.booked);

export function valueSeries(asset: Input, events: ValueEvent[], today: string): Series {
  const now = units(asset.value_after_depreciation) ?? 0n;
  const rows = [...(asset.schedule ?? [])].sort(byDate);

  // What already happened, oldest first; at one date depreciation comes before a change of value.
  interface Step {
    key: string;
    date: string;
    change: bigint;
    kind: PointKind;
    label: string;
    order: number;
    entry?: string | null;
  }
  const past: Step[] = [
    ...rows
      .filter((r) => r.booked)
      .map((r): Step => ({
        key: `r-${r.id}`,
        date: day(r.schedule_date),
        change: -(units(r.depreciation_amount) ?? 0n),
        kind: "booked",
        label: r.disposal ? "Depreciation up to the disposal" : "Depreciation booked",
        order: 0,
        entry: r.journal_entry,
      })),
    ...events.map((e, i): Step => ({ key: `e-${i}`, date: day(e.date), change: units(e.amount) ?? 0n, kind: e.kind, label: e.label, order: 1 })),
  ].sort((a, b) => a.date.localeCompare(b.date) || a.order - b.order);

  // Back from now: the value after each step is the value after the next one, less the next one's change.
  const after: bigint[] = new Array(past.length);
  let value = now;
  for (let i = past.length - 1; i >= 0; i--) {
    after[i] = value;
    value -= past[i].change;
  }
  const startDate = [day(asset.available_for_use_date) || day(asset.purchase_date), past[0]?.date].filter(Boolean).sort()[0] ?? today;
  const cost = (units(asset.gross_purchase_amount) ?? 0n) - (units(asset.opening_accumulated_depreciation) ?? 0n);

  const points: Point[] = [{ key: "start", date: startDate, value: decimal(value), change: "0", kind: "start", booked: true, label: "Put in use" }];
  past.forEach((s, i) => points.push({ key: s.key, date: s.date, value: decimal(after[i]), change: decimal(s.change), kind: s.kind, booked: true, label: s.label, entry: s.entry }));

  // Forward from now through the plan.
  let run = now;
  for (const r of rows.filter((x) => !x.booked)) {
    const amount = units(r.depreciation_amount) ?? 0n;
    run -= amount;
    const date = day(r.schedule_date);
    const kind: PointKind = r.posting_error ? "failed" : date <= today ? "due" : "planned";
    const label = kind === "failed" ? "Could not be booked" : kind === "due" ? "Due to be booked" : "Planned";
    points.push({ key: `r-${r.id}`, date, value: decimal(run), change: decimal(-amount), kind, booked: false, label, error: r.posting_error });
  }

  // A sold or scrapped asset leaves the books at its book value.
  if (asset.disposal && asset.disposal_date) {
    const last = points[points.length - 1];
    points.push({
      key: "disposal",
      date: day(asset.disposal_date),
      value: "0",
      change: minus("0", last.value),
      kind: "disposal",
      booked: true,
      label: asset.status === "Sold" ? "Sold" : "Written off",
    });
  }
  return { points, exact: value === cost };
}
