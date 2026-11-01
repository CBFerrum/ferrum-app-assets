// The overview's figures: where the money is (the register by category or place), what depreciation
// is due month by month, and what needs a person (assets past a return date, not found at the last
// check, due to be verified, and so on), each with the filter that opens exactly those assets in
// the list. Pure, so it is tested without a browser.

import { addDays, monthOf, monthStart, relative } from "./dates";
import { compare, share, sum } from "./money";
import type { Asset, PlanRow, RegisterRow } from "./types";

// ── lists the figures open ──

/** A list's filters as its address holds them: a field and its value, `from..to` for a date range. */
export type Query = Record<string, string>;

/** The same filters as the API takes them. */
export function apiFilters(query: Query): [string, string, string][] {
  const out: [string, string, string][] = [];
  for (const [field, raw] of Object.entries(query)) {
    if (!raw.includes("..")) {
      out.push([field, "=", raw]);
      continue;
    }
    const [from, to] = raw.split("..");
    if (from) out.push([field, ">=", from]);
    if (to) out.push([field, "<=", to]);
  }
  return out;
}

/** Where the generic list of assets (or of another entity) opens with these filters (a path under the app). */
export function listPath(query: Query, entity = "Asset"): string {
  const sp = new URLSearchParams();
  for (const [field, value] of Object.entries(query)) sp.set(`f.${field}`, value);
  const q = sp.toString();
  return `/e/${entity}${q ? `?${q}` : ""}`;
}

// ── where the money is ──

export interface Bar {
  /** The category or place, as the register names it. */
  name: string;
  /** Its record, when the register's name could be matched to one: what the bar opens. */
  id?: string;
  assets: number;
  /** Net book value, exactly. */
  value: string;
  cost: string;
  /** Against the biggest bar, for its length. */
  fraction: number;
  /** Against everything, for the words beside it. */
  percent: number;
}

/** The register's groups, biggest first; past `limit` they are summed as "others". `ids` maps the
 *  register's names to records. */
export function bars(rows: RegisterRow[], key: "asset_category" | "location", ids: ReadonlyMap<string, string>, limit = 7): Bar[] {
  const all = rows
    .map((r) => ({ name: r[key] || (key === "location" ? "No place" : "No category"), assets: r.assets ?? 0, value: r.net_book_value, cost: r.cost }))
    .sort((a, b) => compare(b.value, a.value) || a.name.localeCompare(b.name));
  // One more than the limit is shown as itself: "1 more" would say less than its name.
  const shown = all.length > limit + 1 ? all.slice(0, limit) : all;
  const rest = all.slice(shown.length);
  if (rest.length) shown.push({ name: `${rest.length} more`, assets: rest.reduce((n, r) => n + r.assets, 0), value: sum(rest.map((r) => r.value)), cost: sum(rest.map((r) => r.cost)) });
  const total = sum(all.map((r) => r.value));
  const biggest = shown.reduce((m, r) => (compare(r.value, m) > 0 ? r.value : m), "0");
  return shown.map((r) => ({ ...r, id: ids.get(r.name), fraction: share(r.value, biggest), percent: Math.round(share(r.value, total) * 100) }));
}

// ── depreciation, month by month ──

export interface Month {
  /** First day of the month. */
  start: string;
  /** Depreciation booked in it, exactly. */
  booked: string;
  /** Depreciation still to book in it (planned, or due and not yet booked). */
  waiting: string;
  total: string;
  /** Some of it was due by now and is not booked. */
  late: boolean;
  /** Some of it could not be booked. */
  failed: boolean;
}

/** `count` months from the one `from` falls in, with the plan's rows summed into them. */
export function months(rows: PlanRow[], from: string, count: number): Month[] {
  const out: (Month & { b: string[]; w: string[] })[] = [];
  for (let i = 0; i < count; i++) out.push({ start: monthStart(from, i), booked: "0", waiting: "0", total: "0", late: false, failed: false, b: [], w: [] });
  const at = new Map(out.map((m) => [monthOf(m.start), m]));
  for (const r of rows) {
    const m = at.get(monthOf(r.schedule_date));
    if (!m) continue;
    if (r.status.startsWith("Booked")) m.b.push(r.booked_amount);
    else {
      m.w.push(r.planned_amount);
      m.late ||= r.status === "Due";
      m.failed ||= !!r.posting_error;
    }
  }
  return out.map(({ b, w, ...m }) => {
    const [booked, waiting] = [sum(b), sum(w)];
    return { ...m, booked, waiting, total: sum([booked, waiting]) };
  });
}

/** The tallest month's total, which sets the chart's height. */
export const tallest = (list: Month[]) => list.reduce((m, x) => (compare(x.total, m) > 0 ? x.total : m), "0");

// ── what needs a person ──

export type Tone = "bad" | "warn" | "quiet";

export interface Spec {
  id: string;
  title: string;
  tone: Tone;
  /** Which assets: the list's own filters, so the list opens on exactly these. */
  query: Query;
  /** The date field the line tells about. */
  date?: "expected_return_date" | "last_verified_date" | "next_verification_date" | "warranty_expiry_date";
}

export function specs(today: string, warrantyDays = 60): Spec[] {
  return [
    { id: "missing", title: "Not found at the last check", tone: "bad", query: { verification_result: "Not Found" }, date: "last_verified_date" },
    { id: "overdue", title: "Past their return date", tone: "bad", query: { expected_return_date: `..${addDays(today, -1)}` }, date: "expected_return_date" },
    { id: "broken", title: "Out of order", tone: "bad", query: { status: "Out of Order" } },
    { id: "verify", title: "Due to be verified", tone: "warn", query: { next_verification_date: `..${today}` }, date: "next_verification_date" },
    { id: "warranty", title: `Warranty ends within ${warrantyDays} days`, tone: "warn", query: { warranty_expiry_date: `${today}..${addDays(today, warrantyDays)}` }, date: "warranty_expiry_date" },
    { id: "depreciated", title: "Fully depreciated, still in use", tone: "quiet", query: { status: "Fully Depreciated" } },
  ];
}

export interface Line {
  id: string;
  title: string;
  detail: string;
  /** Where it opens, when not on the asset's own page. */
  to?: string;
}

/** An asset as one line under a heading of `specs`: its name, and what is true of it there. */
export function line(spec: Spec, a: Asset, today: string, format: (day: string) => string): Line {
  const day = spec.date && a[spec.date];
  const when = day ? relative(day, today, format) : "";
  const detail =
    spec.id === "overdue"
      ? [a.custodian, `due back ${when}`]
      : spec.id === "missing"
        ? [`last checked ${when || "never"}`]
        : spec.id === "verify"
          ? [`due ${when}`]
          : spec.id === "warranty"
            ? [`ends ${when}`]
            : spec.id === "broken"
              ? [a._titles?.location]
              : [a._titles?.location, a.custodian];
  return { id: a.id, title: [a.number, a.asset_name].filter(Boolean).join(" · ") || "Asset", detail: detail.filter(Boolean).join(" · ") };
}

// ── depreciation that was due and is not booked ──

export interface Stuck {
  id: string;
  title: string;
  since: string;
  /** Why the last attempt failed; empty while it waits for the next round. */
  reason: string;
}

/** The assets with depreciation due and not booked, with the oldest period of each. */
export function stuck(rows: PlanRow[]): Stuck[] {
  const first = new Map<string, PlanRow>();
  for (const r of rows) {
    if (r.status !== "Due") continue;
    const seen = first.get(r.asset_id);
    if (!seen || r.schedule_date < seen.schedule_date) first.set(r.asset_id, r);
  }
  return [...first.values()]
    .sort((a, b) => a.schedule_date.localeCompare(b.schedule_date) || (a.asset ?? "").localeCompare(b.asset ?? ""))
    .map((r) => ({ id: r.asset_id, title: [r.asset, r.asset_name].filter(Boolean).join(" · "), since: r.schedule_date, reason: r.posting_error ?? "" }));
}

/** How far through its cost an asset base is depreciated, in whole percent. */
export const depreciated = (accumulated: unknown, cost: unknown) => Math.round(share(accumulated, cost) * 100);

/** The month's name for a chart's label: "Nov", with the year when it is not this one. */
export function monthLabel(start: string, today: string, locale?: string): string {
  const d = new Date(`${start}T00:00:00Z`);
  const short = d.toLocaleDateString(locale, { month: "short", timeZone: "UTC" });
  return monthOf(start).slice(0, 4) === monthOf(today).slice(0, 4) ? short : `${short} ${start.slice(2, 4)}`;
}
