// An asset's depreciation: its book value over time (what was booked solid, what is planned dashed,
// a period that could not be booked in red with the reason), and every period in a table. Pointing
// at the line, or the arrow keys on it, reads one period: its amount, what the asset was worth after
// it, and the journal entry that booked it.

import { useQuery } from "@tanstack/react-query";
import { type KeyboardEvent, type PointerEvent, useEffect, useMemo, useRef, useState } from "react";
import { EmptyState } from "@ferrum/kit";
import { daysBetween } from "./dates";
import { list } from "./data";
import { compact, dayText } from "./format";
import { plot } from "./money";
import type { Asset } from "./types";
import { type Point, type PointKind, type ValueEvent, valueSeries } from "./value";

const H = 232;
/** The tooltip's width in pixels, so it can be kept inside the drawing. */
const TIP = 208;
const M = { top: 14, right: 16, bottom: 30, left: 58 };

const TIME = (iso: string) => Date.parse(`${iso}T00:00:00Z`);
const monthYear = (t: number) => new Date(t).toLocaleDateString(undefined, { month: "short", year: "numeric", timeZone: "UTC" });

const WORDS: Record<PointKind, string> = {
  start: "Put in use",
  booked: "Booked",
  due: "Due",
  planned: "Planned",
  failed: "Could not be booked",
  adjustment: "Revalued",
  repair: "Repair added",
  disposal: "Left the books",
};

/** What changed value besides depreciation, so the line can show its steps. */
function useValueEvents(asset: Asset) {
  return useQuery({
    queryKey: ["assets", "value-events", asset.id, asset.version],
    queryFn: async ({ signal }): Promise<ValueEvent[]> => {
      const [adjustments, repairs] = await Promise.all([
        list<{ adjustment_date: string; difference_amount: string }>("AssetValueAdjustment", { filters: [["asset", "=", asset.id], ["docstatus", "=", 1]], fields: "adjustment_date,difference_amount", limit: 200 }, signal)
          .then((r) => r.rows)
          // Not everyone may read these: the line is then only as exact as the rest allows (see `exact`).
          .catch(() => []),
        list<{ completion_date: string; repair_cost: string }>("AssetRepair", { filters: [["asset", "=", asset.id], ["docstatus", "=", 1], ["capitalize_repair_cost", "=", true]], fields: "completion_date,repair_cost", limit: 200 }, signal)
          .then((r) => r.rows)
          .catch(() => []),
      ]);
      return [
        ...adjustments.map((a): ValueEvent => ({ date: a.adjustment_date, amount: a.difference_amount, kind: "adjustment", label: "Revalued" })),
        ...repairs.filter((r) => r.completion_date).map((r): ValueEvent => ({ date: r.completion_date, amount: r.repair_cost, kind: "repair", label: "Repair added to its value" })),
      ];
    },
  });
}

/** What a point changed, in words. */
function changeText(p: Point, money: (v: unknown) => string): string {
  const amount = money(p.change.replace("-", ""));
  if (p.kind === "disposal") return "Taken off the books";
  if (p.kind === "adjustment") return plot(p.change) < 0 ? `Written down by ${amount}` : `Written up by ${amount}`;
  if (p.kind === "repair") return `Added ${amount}`;
  return `Depreciation ${amount}`;
}

const titleOf = (p: Point) => (p.kind === "adjustment" || p.kind === "repair" ? p.label : WORDS[p.kind]);

function Chart({ points, today, money }: { points: Point[]; today: string; money: (v: unknown) => string }) {
  const [active, setActive] = useState<number | null>(null);
  // The drawing is as wide as its box, so a unit is a pixel and its words keep their size on a phone.
  const box = useRef<HTMLDivElement>(null);
  const [W, setWidth] = useState(640);
  useEffect(() => {
    const el = box.current;
    if (!el || typeof ResizeObserver === "undefined") return;
    const watch = new ResizeObserver(([entry]) => setWidth(Math.max(260, Math.round(entry.contentRect.width))));
    watch.observe(el);
    return () => watch.disconnect();
  }, []);
  const geometry = useMemo(() => {
    const times = points.map((p) => TIME(p.date));
    const t0 = Math.min(...times);
    const t1 = Math.max(...times, t0 + 86_400_000);
    const top = Math.max(1, ...points.map((p) => plot(p.value)));
    const x = (iso: string) => M.left + ((TIME(iso) - t0) / (t1 - t0)) * (W - M.left - M.right);
    const y = (v: number) => M.top + (1 - v / top) * (H - M.top - M.bottom);
    const at = points.map((p) => ({ x: x(p.date), y: y(plot(p.value)) }));
    const booked = points.map((p, i) => (p.booked ? i : -1)).filter((i) => i >= 0);
    const solid = booked.map((i, n) => `${n ? "L" : "M"}${at[i].x.toFixed(1)} ${at[i].y.toFixed(1)}`).join(" ");
    const lastBooked = booked.length ? booked[booked.length - 1] : 0;
    const planned = points.map((_, i) => i).filter((i) => i >= lastBooked && (i === lastBooked || !points[i].booked));
    const dashed = planned.map((i, n) => `${n ? "L" : "M"}${at[i].x.toFixed(1)} ${at[i].y.toFixed(1)}`).join(" ");
    const now = TIME(today) >= t0 && TIME(today) <= t1 ? x(today) : null;
    return { t0, t1, top, at, solid, dashed, now };
  }, [points, today, W]);

  const nearest = (e: PointerEvent<SVGSVGElement>) => {
    const box = e.currentTarget.getBoundingClientRect();
    const px = ((e.clientX - box.left) / box.width) * W;
    let best = 0;
    geometry.at.forEach((p, i) => {
      if (Math.abs(p.x - px) < Math.abs(geometry.at[best].x - px)) best = i;
    });
    setActive(best);
  };
  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const last = points.length - 1;
    const next = e.key === "ArrowRight" ? Math.min(last, (active ?? -1) + 1) : e.key === "ArrowLeft" ? Math.max(0, (active ?? 1) - 1) : e.key === "Home" ? 0 : e.key === "End" ? last : null;
    if (e.key === "Escape") setActive(null);
    if (next === null) return;
    e.preventDefault();
    setActive(next);
  };
  const p = active === null ? null : points[active];
  const ticks = [0, 0.5, 1];
  const many = points.length > 60;

  return (
    <div ref={box} className="ast-chart" tabIndex={0} role="group" aria-label="Book value over time. Use the left and right arrow keys to read each period." onKeyDown={onKeyDown}>
      <svg viewBox={`0 0 ${W} ${H}`} role="presentation" onPointerMove={nearest} onPointerDown={nearest}>
        {ticks.map((f) => {
          const yy = M.top + (1 - f) * (H - M.top - M.bottom);
          return (
            <g key={f}>
              <line className="ast-grid-line" x1={M.left} x2={W - M.right} y1={yy} y2={yy} />
              <text className="ast-axis" x={M.left - 8} y={yy + 4} textAnchor="end">
                {compact(geometry.top * f)}
              </text>
            </g>
          );
        })}
        <text className="ast-axis" x={M.left} y={H - 8}>
          {monthYear(geometry.t0)}
        </text>
        <text className="ast-axis" x={W - M.right} y={H - 8} textAnchor="end">
          {monthYear(geometry.t1)}
        </text>
        {geometry.now !== null && (
          <g>
            <line className="ast-now" x1={geometry.now} x2={geometry.now} y1={M.top} y2={H - M.bottom} />
            <text className="ast-axis" x={geometry.now} y={H - 8} textAnchor="middle">
              Today
            </text>
          </g>
        )}
        <path className="ast-line-planned" d={geometry.dashed} fill="none" />
        <path className="ast-line-booked" d={geometry.solid} fill="none" />
        {points.map((pt, i) =>
          many && !["adjustment", "repair", "failed", "disposal"].includes(pt.kind) && i !== active ? null : (
            <circle key={pt.key} className={`ast-point ast-point-${pt.kind}`} data-active={i === active || undefined} cx={geometry.at[i].x} cy={geometry.at[i].y} r={i === active ? 5 : 3.5} />
          ),
        )}
      </svg>
      {p && active !== null && (
        <div
          className="ast-tip"
          data-below={geometry.at[active].y < H * 0.5 || undefined}
          style={{ left: `${Math.min(Math.max(geometry.at[active].x, TIP / 2 + 4), Math.max(W - TIP / 2 - 4, TIP / 2 + 4))}px`, top: `${geometry.at[active].y}px`, width: TIP }}
        >
          <strong>
            {titleOf(p)}, {dayText(p.date)}
          </strong>
          {p.kind !== "start" && <div>{changeText(p, money)}</div>}
          <div>Worth {money(p.value)} after</div>
          {p.entry && (
            <a href={`/open/JournalEntry/${p.entry}`} target="_blank" rel="noreferrer">
              Journal entry
            </a>
          )}
          {p.error && <div className="ast-tip-error">{p.error}</div>}
        </div>
      )}
      <p className="sr-only" aria-live="polite">
        {p ? `${titleOf(p)}, ${dayText(p.date)}: ${p.kind === "start" ? "" : `${changeText(p, money)}; `}worth ${money(p.value)} after.` : ""}
      </p>
    </div>
  );
}

export function Depreciation({ asset, today, money }: { asset: Asset; today: string; money: (v: unknown) => string }) {
  const events = useValueEvents(asset);
  const series = useMemo(() => valueSeries(asset, events.data ?? [], today), [asset, events.data, today]);
  const { points } = series;

  if (asset.calculate_depreciation === false) {
    return (
      <EmptyState title="This asset does not depreciate" icon="chart">
        <p className="muted">It stays on the books at {money(asset.value_after_depreciation)}.</p>
      </EmptyState>
    );
  }
  const waiting = points.filter((p) => !p.booked && p.kind !== "start");
  const next = waiting[0];
  const stuck = waiting.filter((p) => p.kind === "failed" || p.kind === "due");
  const last = points.filter((p) => p.booked && p.kind === "booked").at(-1);

  return (
    <div className="stack ast-depr">
      {stuck.length > 0 && (
        <div className="alert alert-error" role="alert">
          <div className="alert-body">
            <strong>{stuck.some((p) => p.error) ? "Depreciation could not be booked." : "Depreciation is due and not booked yet."}</strong>{" "}
            {stuck[0].error ? stuck[0].error : `Due since ${dayText(stuck[0].date)}; the books are updated every minute.`}
          </div>
        </div>
      )}
      <p className="muted ast-summary">
        {last ? `Booked up to ${dayText(last.date)}. ` : "Nothing booked yet. "}
        {next ? `Next: ${money(next.change.replace("-", ""))} ${daysBetween(today, next.date) < 0 ? "was due" : "on"} ${dayText(next.date)}.` : "Nothing more is planned."}
      </p>
      {points.length < 2 ? (
        <p className="muted">No depreciation is planned for this asset.</p>
      ) : (
        <Chart points={points} today={today} money={money} />
      )}
      {!series.exact && events.isSuccess && <p className="ast-note">Revaluations and repairs are not all shown to you, so the early part of the line is only as exact as what you can see. The present and the plan are the books'.</p>}
      <div className="table-wrap">
        <table className="table ast-schedule">
          <caption className="sr-only">Every period of the depreciation schedule</caption>
          <thead>
            <tr>
              <th scope="col">Date</th>
              <th scope="col">What</th>
              <th scope="col" className="num">
                Change
              </th>
              <th scope="col" className="num">
                Worth after
              </th>
            </tr>
          </thead>
          <tbody>
            {points.map((pt) => (
              <tr key={pt.key} data-kind={pt.kind}>
                <td className="nowrap">{dayText(pt.date)}</td>
                <td>
                  <span className={`badge ${pt.kind === "booked" ? "badge-green" : pt.kind === "failed" ? "badge-red" : pt.kind === "due" ? "badge-amber" : pt.kind === "planned" ? "" : "badge-blue"}`}>{WORDS[pt.kind]}</span>
                  {pt.entry && (
                    <>
                      {" "}
                      <a className="faint" href={`/open/JournalEntry/${pt.entry}`} target="_blank" rel="noreferrer">
                        entry
                      </a>
                    </>
                  )}
                  {pt.error && <div className="ast-reason">{pt.error}</div>}
                </td>
                <td className="num">{pt.kind === "start" ? "" : money(pt.change)}</td>
                <td className="num">{money(pt.value)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}
