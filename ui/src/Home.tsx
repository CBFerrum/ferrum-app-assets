// Assets' home: what the company owns and what it is worth, the depreciation due month by month, the
// maintenance coming up, and what needs a person (assets past a return date, not found at the last
// check, due to be verified, depreciation that could not be booked...). Every figure opens the
// matching list or report. The figures are the server's: its reports and lists, with the caller's
// own access, so someone who holds a few assets sees those.

import { useQueries, useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback, useMemo, type ReactNode } from "react";
import { Link, useSearchParams } from "react-router";
import { errorMessage, todayIso } from "@ferrum/client";
import { EmptyState, ErrorState, Icon, useLiveEntity, useShellTitle } from "@ferrum/kit";
import { can, useEntityMeta, useMe } from "@ferrum/records";
import { addMonths, monthEnd, monthStart, relative } from "./dates";
import { list, report } from "./data";
import { compact, dayText, moneyIn } from "./format";
import { plot } from "./money";
import { apiFilters, bars, depreciated, line, listPath, monthLabel, months, specs, stuck, tallest, type Bar, type Line, type Month, type Query, type Tone } from "./register";
import { Tabs } from "./Tabs";
import type { Asset, DueRow, PlanRow, RegisterRow } from "./types";

const FIELDS = "number,asset_name,custodian,location,expected_return_date,last_verified_date,next_verification_date,warranty_expiry_date";

const isDay = (v: string | null) => (v && /^\d{4}-\d{2}-\d{2}$/.test(v) ? v : null);

/** A section that could not load: what the server said, and a way to ask again. */
function SectionError({ error, retry }: { error: unknown; retry: () => void }) {
  return (
    <p className="ast-note" role="alert">
      {errorMessage(error)}{" "}
      <button type="button" className="link-btn" onClick={retry}>
        Try again
      </button>
    </p>
  );
}

function Skeleton({ rows = 3 }: { rows?: number }) {
  return (
    <div className="ast-skeleton" role="status" aria-label="Loading">
      {Array.from({ length: rows }, (_, i) => (
        <div key={i} className="skeleton" />
      ))}
    </div>
  );
}

/** The company the figures are for: the one asked for in the address, else the person's own. */
function useCompany(ready: boolean) {
  const me = useMe().data;
  const [sp, setSp] = useSearchParams();
  const all = useQuery({
    queryKey: ["assets", "companies"],
    queryFn: ({ signal }) => list<{ id: string; company_name?: string }>("Company", { fields: "company_name", limit: 50 }, signal).then((r) => r.rows),
    enabled: ready,
    staleTime: 5 * 60_000,
  });
  const wanted = sp.get("company");
  const id = all.data?.find((c) => c.id === wanted)?.id ?? me?.default_company ?? all.data?.[0]?.id ?? undefined;
  const choose = (company: string) => setSp((p) => { const n = new URLSearchParams(p); n.set("company", company); return n; }, { replace: true });
  return { id, all: all.data ?? [], settled: ready && !all.isLoading, choose };
}

// ── the totals ──

function Tile({ label, value, sub, to, children }: { label: string; value: ReactNode; sub?: ReactNode; to?: string; children?: ReactNode }) {
  const body = (
    <>
      <span className="ast-tile-label">{label}</span>
      <strong className="ast-tile-value">{value}</strong>
      {children}
      {sub && <span className="ast-tile-sub">{sub}</span>}
    </>
  );
  return to ? (
    <Link className="ast-tile" to={to}>
      {body}
    </Link>
  ) : (
    <div className="ast-tile">{body}</div>
  );
}

// ── where the money is ──

function Owned({ by, setBy, rows, loading, error, retry, money, query }: {
  by: "asset_category" | "location";
  setBy: (by: "asset_category" | "location") => void;
  rows: RegisterRow[] | undefined;
  loading: boolean;
  error: unknown;
  retry: () => void;
  money: (v: unknown) => string;
  query: Query;
}) {
  const ids = useQuery({
    queryKey: ["assets", "names", by],
    queryFn: ({ signal }) =>
      by === "location"
        ? list<{ id: string; location_name: string }>("Location", { fields: "location_name", limit: 500 }, signal).then((r) => new Map(r.rows.map((x) => [x.location_name, x.id])))
        : list<{ id: string; category_name: string }>("AssetCategory", { fields: "category_name", limit: 500 }, signal).then((r) => new Map(r.rows.map((x) => [x.category_name, x.id]))),
    staleTime: 5 * 60_000,
  });
  const shown = useMemo(() => (rows ? bars(rows, by, ids.data ?? new Map()) : []), [rows, by, ids.data]);
  return (
    <section className="card ast-owned" aria-labelledby="ast-owned">
      <div className="card-head">
        <h2 id="ast-owned">What the company owns</h2>
        <span className="spacer" />
        <div className="segmented" role="group" aria-label="Group by">
          <button type="button" aria-pressed={by === "asset_category"} onClick={() => setBy("asset_category")}>
            By category
          </button>
          <button type="button" aria-pressed={by === "location"} onClick={() => setBy("location")}>
            By place
          </button>
        </div>
      </div>
      <div className="card-body">
        {error ? (
          <SectionError error={error} retry={retry} />
        ) : loading || !rows ? (
          <Skeleton rows={4} />
        ) : !shown.length ? (
          <p className="muted">Nothing registered yet.</p>
        ) : (
          <ul className="ast-bars">
            {shown.map((b: Bar) => {
              const name = b.id ? <Link to={listPath({ ...query, [by]: b.id })}>{b.name}</Link> : <span>{b.name}</span>;
              return (
                <li key={b.name}>
                  <div className="ast-bar-line">
                    {name}
                    <span className="faint">{b.assets === 1 ? "1 asset" : `${b.assets} assets`}</span>
                    <span className="spacer" />
                    <strong className="num">{money(b.value)}</strong>
                    <span className="faint ast-percent">{b.percent}%</span>
                  </div>
                  <div className="ast-bar" aria-hidden="true">
                    <span style={{ width: `${Math.max(b.fraction * 100, b.fraction > 0 ? 1.5 : 0)}%` }} />
                  </div>
                </li>
              );
            })}
          </ul>
        )}
      </div>
    </section>
  );
}

// ── depreciation by month ──

function Months({ items, today, money, company }: { items: Month[]; today: string; money: (v: unknown) => string; company?: string }) {
  const top = plot(tallest(items));
  return (
    <>
      <ol className="ast-months">
        {items.map((m) => {
          const [booked, waiting] = [plot(m.booked), plot(m.waiting)];
          const label = monthLabel(m.start, today);
          const to = `/r/asset_depreciation_schedule?${new URLSearchParams({ ...(company ? { company } : {}), from_date: m.start, to_date: monthEnd(m.start), run: "1" })}`;
          const said = `${label}: ${money(m.booked)} booked, ${money(m.waiting)} still to book${m.failed ? ", some could not be booked" : m.late ? ", some is overdue" : ""}`;
          return (
            <li key={m.start}>
              <Link className="ast-month" to={to} title={said} aria-label={said} data-now={m.start === monthStart(today) || undefined}>
                <span className="ast-month-flag" data-state={m.failed ? "failed" : undefined} aria-hidden="true">
                  {m.failed || m.late ? "!" : ""}
                </span>
                <span className="ast-month-amount faint">{top > 0 && booked + waiting > 0 ? compact(booked + waiting) : ""}</span>
                <span className="ast-month-bar">
                  {booked > 0 && <span className="ast-month-booked" style={{ height: `${(booked / top) * 100}%` }} />}
                  {waiting > 0 && <span className="ast-month-waiting" data-state={m.failed ? "failed" : m.late ? "late" : undefined} style={{ height: `${(waiting / top) * 100}%` }} />}
                </span>
                <span className="ast-month-name">{label}</span>
              </Link>
            </li>
          );
        })}
      </ol>
      <p className="ast-legend faint">
        <span className="ast-key ast-key-booked" aria-hidden="true" /> booked <span className="ast-key ast-key-waiting" aria-hidden="true" /> still to book
        <span className="ast-key ast-key-late" aria-hidden="true" /> overdue <span className="ast-key ast-key-failed" aria-hidden="true" /> could not be booked
      </p>
    </>
  );
}

// ── what needs a person ──

interface Group {
  id: string;
  title: string;
  tone: Tone;
  total: number;
  lines: Line[];
  /** Where all of them open, under the app. */
  to: string;
}

const TONE_WORD: Record<Tone, string> = { bad: "Needs action", warn: "Coming up", quiet: "For information" };

function Attention({ groups, loading, error, retry }: { groups: Group[]; loading: boolean; error: unknown; retry: () => void }) {
  const shown = groups.filter((g) => g.total > 0);
  return (
    <section className="card ast-attention" aria-labelledby="ast-attention">
      <div className="card-head">
        <h2 id="ast-attention">Needs attention</h2>
      </div>
      <div className="card-body">
        {error ? <SectionError error={error} retry={retry} /> : null}
        {loading && !shown.length ? (
          <Skeleton rows={4} />
        ) : !shown.length ? (
          error ? null : (
            <EmptyState title="Nothing needs attention" icon="check">
              Returns, checks, repairs and depreciation are all where they should be.
            </EmptyState>
          )
        ) : (
          <ul className="ast-groups">
            {shown.map((g) => (
              <li key={g.id} className="ast-group" data-tone={g.tone}>
                <div className="ast-group-head">
                  <span className="ast-group-mark" aria-hidden="true">
                    <Icon name={g.tone === "bad" ? "alert" : g.tone === "warn" ? "calendar" : "check"} size={14} />
                  </span>
                  <Link to={g.to}>{g.title}</Link>
                  <span className={`badge ${g.tone === "bad" ? "badge-red" : g.tone === "warn" ? "badge-amber" : ""}`} title={TONE_WORD[g.tone]}>
                    {g.total}
                  </span>
                </div>
                <ul className="ast-lines">
                  {g.lines.map((l) => (
                    <li key={l.id}>
                      <Link to={l.to ?? `/a/${l.id}`}>{l.title}</Link>
                      {l.detail && <span className="faint"> {l.detail}</span>}
                    </li>
                  ))}
                  {g.total > g.lines.length && (
                    <li>
                      <Link className="faint" to={g.to}>
                        and {g.total - g.lines.length} more
                      </Link>
                    </li>
                  )}
                </ul>
              </li>
            ))}
          </ul>
        )}
      </div>
    </section>
  );
}

// ── the page ──

export function Home() {
  useShellTitle("Assets");
  const qc = useQueryClient();
  const who = useMe();
  const assetMeta = useEntityMeta("Asset").data;
  const checkMeta = useEntityMeta("AssetVerification").data;
  const [sp, setSp] = useSearchParams();
  const today = todayIso();
  const asOf = isDay(sp.get("as_of")) ?? today;
  const by = sp.get("by") === "location" ? "location" : "asset_category";
  const company = useCompany(!who.isLoading);
  const ready = company.settled;
  const many = company.all.length > 1;
  const scope: Query = many && company.id ? { company: company.id } : {};
  const params: Record<string, string> = company.id ? { company: company.id } : {};

  useLiveEntity("Asset", useCallback(() => void qc.invalidateQueries({ queryKey: ["assets"] }), [qc]));
  useLiveEntity("AssetMaintenanceLog", useCallback(() => void qc.invalidateQueries({ queryKey: ["assets", "maintenance"] }), [qc]));

  const register = (group: "Asset Category" | "Location", enabled: boolean) => ({
    queryKey: ["assets", "register", company.id, asOf, group],
    queryFn: ({ signal }: { signal: AbortSignal }) => report<RegisterRow>("fixed_asset_register", { ...params, as_of: asOf, group_by: group }, signal),
    enabled: ready && enabled,
  });
  const [byCategory, byPlace] = useQueries({ queries: [register("Asset Category", true), register("Location", by === "location")] });
  const totals = byCategory.data?.totals;
  const money = moneyIn(byCategory.data?.meta?.currency);

  const from = monthStart(today);
  const plan = useQuery({
    queryKey: ["assets", "plan", company.id, from],
    queryFn: ({ signal }) => report<PlanRow>("asset_depreciation_schedule", { ...params, from_date: from, to_date: monthEnd(addMonths(from, 11)) }, signal),
    enabled: ready,
  });
  const upcoming = useMemo(() => (plan.data ? months(plan.data.rows, from, 12) : []), [plan.data, from]);

  const due = useQuery({
    queryKey: ["assets", "maintenance", company.id, today],
    queryFn: ({ signal }) => report<DueRow>("asset_maintenance_due", { ...params, as_of: today, days: "30" }, signal),
    enabled: ready,
  });

  // Depreciation that was due and is not booked: the assets, then why for the first few.
  const unbooked = useQuery({
    queryKey: ["assets", "unbooked", company.id, today],
    queryFn: async ({ signal }) => {
      const behind = await list<Asset>(
        "Asset",
        { filters: [["next_depreciation_date", "<=", today], ["disposal", "is", "not set"], ...(company.id ? [["company", "=", company.id]] : [])], fields: "number,asset_name,next_depreciation_date", sort: "next_depreciation_date asc", limit: 4, count: true },
        signal,
      );
      const why = await Promise.all(
        behind.rows.map((a) => report<PlanRow>("asset_depreciation_schedule", { ...params, asset: a.id, from_date: a.next_depreciation_date ?? today, to_date: today }, signal).then((r) => r.rows).catch(() => [] as PlanRow[])),
      );
      return { total: behind.total ?? behind.rows.length, items: stuck(why.flat()) };
    },
    enabled: ready,
  });

  const wanted = useMemo(() => specs(today), [today]);
  const lists = useQueries({
    queries: wanted.map((spec) => ({
      queryKey: ["assets", "attention", spec.id, company.id, today],
      queryFn: ({ signal }: { signal: AbortSignal }) =>
        list<Asset>("Asset", { filters: [...apiFilters(spec.query), ...(many && company.id ? [["company", "=", company.id]] : [])], fields: FIELDS, titles: true, sort: spec.date ? `${spec.date} asc` : undefined, limit: 3, count: true }, signal),
      enabled: ready,
    })),
  });

  const groups: Group[] = useMemo(() => {
    const found = new Map<string, Group>();
    wanted.forEach((spec, i) => {
      const r = lists[i]?.data;
      if (r) found.set(spec.id, { id: spec.id, title: spec.title, tone: spec.tone, total: r.total ?? r.rows.length, lines: r.rows.map((a) => line(spec, a, today, dayText)), to: listPath({ ...spec.query, ...scope }) });
    });
    const behind = unbooked.data;
    if (behind) {
      const failing = behind.items.some((s) => s.reason);
      found.set("posting", {
        id: "posting",
        title: failing ? "Depreciation could not be booked" : "Depreciation due, not booked yet",
        tone: "bad",
        total: behind.total,
        lines: behind.items.map((s) => ({ id: s.id, title: s.title, detail: s.reason ? `since ${dayText(s.since)}: ${s.reason}` : `since ${dayText(s.since)}` })),
        to: `/r/asset_depreciation_schedule?${new URLSearchParams({ ...params, from_date: behind.items[0]?.since ?? from, to_date: today, run: "1" })}`,
      });
    }
    const overdue = due.data?.rows.filter((r) => r.days_overdue > 0) ?? [];
    if (due.data) {
      found.set("maintenance", {
        id: "maintenance",
        title: "Maintenance overdue",
        tone: "bad",
        total: overdue.length,
        lines: overdue.slice(0, 3).map((r) => ({ id: r.asset_id, to: `/a/${r.asset_id}?tab=maintenance`, title: [r.asset, r.asset_name].filter(Boolean).join(" · "), detail: `${r.task_name ?? "Maintenance"}, ${r.days_overdue} ${r.days_overdue === 1 ? "day" : "days"} late` })),
        to: `/r/asset_maintenance_due?${new URLSearchParams({ ...params, as_of: today, days: "0", run: "1" })}`,
      });
    }
    const order = ["posting", "missing", "overdue", "broken", "maintenance", "verify", "warranty", "depreciated"];
    return order.flatMap((id) => found.get(id) ?? []);
  }, [wanted, lists, unbooked.data, due.data, today, scope, params, from]);
  const settled = lists.every((l) => !l.isLoading) && !unbooked.isLoading && !due.isLoading;
  const failed = lists.find((l) => l.error)?.error ?? unbooked.error ?? due.error;
  const retryAttention = () => {
    for (const l of lists) if (l.error) void l.refetch();
    void unbooked.refetch();
    void due.refetch();
  };

  const rows = (by === "location" ? byPlace : byCategory).data?.rows;
  const count = byCategory.data?.rows.reduce((n, r) => n + (r.assets ?? 0), 0) ?? 0;
  const thisMonth = upcoming[0];
  const canAdd = can(assetMeta?.permissions, "create");
  const canVerify = can(checkMeta?.permissions, "create");
  const everything = assetMeta?.permissions.read === "all";
  const registerLink = `/r/fixed_asset_register?${new URLSearchParams({ ...params, as_of: asOf, run: "1" })}`;
  const setParam = (name: string, value: string | null) => setSp((p) => { const n = new URLSearchParams(p); if (value) n.set(name, value); else n.delete(name); return n; }, { replace: true });

  const body = byCategory.error ? (
    <ErrorState error={byCategory.error} onRetry={() => void byCategory.refetch()} title="The register could not be loaded" />
  ) : byCategory.data && count === 0 && !byCategory.isFetching && asOf === today ? (
    <EmptyState title={everything ? "No assets yet" : "No assets in your care"} icon="file">
      {everything ? (
        <>
          <p className="muted">Assets are made when a purchase invoice buys a fixed asset item. Ones you already own are added by hand.</p>
          {canAdd && (
            <Link className="btn btn-primary btn-md" to="/e/Asset/new">
              <Icon name="plus" />
              Add an asset you own
            </Link>
          )}
        </>
      ) : (
        <p className="muted">Assets handed to you will appear here.</p>
      )}
    </EmptyState>
  ) : (
    <>
      <section className="ast-tiles" aria-label="Totals">
        <Tile label="Net book value" value={byCategory.data ? money(totals?.net_book_value) : <span className="skeleton ast-tile-skel" />} sub={byCategory.data ? `${count} ${count === 1 ? "asset" : "assets"}${asOf === today ? "" : `, as of ${dayText(asOf)}`}` : undefined} to={registerLink} />
        <Tile label="Cost" value={byCategory.data ? money(totals?.cost) : <span className="skeleton ast-tile-skel" />} sub="what was paid, with repairs and revaluations" to={registerLink} />
        <Tile label="Depreciated" value={byCategory.data ? `${depreciated(totals?.accumulated_depreciation, totals?.cost)}%` : <span className="skeleton ast-tile-skel" />} sub={byCategory.data ? `${money(totals?.accumulated_depreciation)} of the cost` : undefined} to={registerLink}>
          <span className="ast-meter" aria-hidden="true">
            <span style={{ width: `${depreciated(totals?.accumulated_depreciation, totals?.cost)}%` }} />
          </span>
        </Tile>
        <Tile
          label="Depreciation this month"
          value={thisMonth ? money(thisMonth.total) : plan.error ? "—" : <span className="skeleton ast-tile-skel" />}
          sub={thisMonth ? `${money(thisMonth.booked)} booked so far` : undefined}
          to={`/r/asset_depreciation_schedule?${new URLSearchParams({ ...params, from_date: from, to_date: monthEnd(from), run: "1" })}`}
        />
      </section>
      <div className="ast-grid">
        <Owned
          by={by}
          setBy={(next) => setParam("by", next === "location" ? "location" : null)}
          rows={rows}
          loading={(by === "location" ? byPlace : byCategory).isLoading}
          error={(by === "location" ? byPlace : byCategory).error}
          retry={() => void (by === "location" ? byPlace : byCategory).refetch()}
          money={money}
          query={scope}
        />
        <Attention groups={groups} loading={!settled} error={failed} retry={retryAttention} />
        <section className="card ast-depreciation" aria-labelledby="ast-depreciation">
          <div className="card-head">
            <h2 id="ast-depreciation">Depreciation, month by month</h2>
          </div>
          <div className="card-body">
            {plan.error ? (
              <SectionError error={plan.error} retry={() => void plan.refetch()} />
            ) : !plan.data ? (
              <Skeleton rows={4} />
            ) : (
              <>
                <Months items={upcoming} today={today} money={money} company={company.id} />
                {plan.data.meta?.truncated && <p className="ast-note">There are more periods than a report holds, so the later months are partial. Open a month for its own report.</p>}
              </>
            )}
          </div>
        </section>
        <section className="card ast-maintenance" aria-labelledby="ast-maintenance">
          <div className="card-head">
            <h2 id="ast-maintenance">Maintenance in the next 30 days</h2>
            <span className="spacer" />
            <Link to={`/r/asset_maintenance_due?${new URLSearchParams({ ...params, as_of: today, days: "90", run: "1" })}`}>All maintenance</Link>
          </div>
          <div className="card-body">
            {due.error ? (
              <SectionError error={due.error} retry={() => void due.refetch()} />
            ) : !due.data ? (
              <Skeleton rows={3} />
            ) : !due.data.rows.length ? (
              <p className="muted">No maintenance is due.</p>
            ) : (
              <ul className="ast-lines ast-due">
                {due.data.rows.slice(0, 6).map((r) => (
                  <li key={`${r.asset_maintenance}-${r.task_name}-${r.next_due_date}`}>
                    <span className={`badge ${r.days_overdue > 0 ? "badge-red" : "badge-amber"}`}>{r.days_overdue > 0 ? `${r.days_overdue} ${r.days_overdue === 1 ? "day" : "days"} late` : relative(r.next_due_date, today, dayText)}</span>
                    <span>
                      <strong>{r.task_name ?? "Maintenance"}</strong>{" "}
                      <Link className="faint" to={`/a/${r.asset_id}?tab=maintenance`}>
                        {[r.asset, r.asset_name].filter(Boolean).join(" · ")}
                      </Link>
                    </span>
                  </li>
                ))}
              </ul>
            )}
          </div>
        </section>
      </div>
    </>
  );

  return (
    <div className="ast-page">
      <header className="ast-head">
        <div>
          <Tabs />
          <p className="page-sub" aria-live="polite">
            {byCategory.data ? `${count} ${count === 1 ? "asset" : "assets"}${byCategory.data.meta?.company ? ` of ${byCategory.data.meta.company}` : ""}` : " "}
          </p>
        </div>
        <div className="ast-tools">
          {many && (
            <select className="select ast-company" aria-label="Company" value={company.id ?? ""} onChange={(e) => company.choose(e.target.value)}>
              {company.all.map((c) => (
                <option key={c.id} value={c.id}>
                  {c.company_name ?? c.id}
                </option>
              ))}
            </select>
          )}
          <label className="ast-asof">
            <span className="faint">As of</span>
            <input className="input" type="date" max={today} value={asOf} onChange={(e) => setParam("as_of", isDay(e.target.value) && e.target.value !== today ? e.target.value : null)} />
          </label>
          {canVerify && (
            <Link className="btn btn-primary btn-md" to="/verify">
              <Icon name="check" />
              Verify a room
            </Link>
          )}
          {canAdd && (
            <Link className="btn btn-default btn-md" to="/e/Asset/new">
              <Icon name="plus" />
              New asset
            </Link>
          )}
        </div>
      </header>
      {body}
    </div>
  );
}
