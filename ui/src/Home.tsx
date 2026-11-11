// Assets' home: what the company owns and what it is worth, the depreciation due month by month, the
// maintenance coming up, and what needs a person (assets past a return date, not found at the last
// check, due to be verified, depreciation that could not be booked...). Every figure opens the
// matching list or report. The figures are the server's: its reports and lists, with the caller's
// own access, so someone who holds a few assets sees those.

import { useQueries, useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback, useMemo, type ReactNode } from "react";
import { Link, useSearchParams } from "react-router";
import { errorMessage, todayIso, t, isolate } from "@ferrum/client";
import { Icon, StatePanel, useLiveEntity, useShellTitle, placed } from "@ferrum/kit";
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
      {isolate(errorMessage(error))}{isolate(" ")}
      <button type="button" className="link-btn" onClick={retry}>
        {t("Try again")}
      </button>
    </p>
  );
}

function Skeleton({ rows = 3 }: { rows?: number }) {
  return (
    <div className="ast-skeleton" role="status" aria-label={t("Loading")}>
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
      <span className="ast-tile-label">{isolate(label)}</span>
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
        <h2 id="ast-owned">{t("What the company owns")}</h2>
        <span className="spacer" />
        <div className="segmented" role="group" aria-label={t("Group by")}>
          <button type="button" aria-pressed={by === "asset_category"} onClick={() => setBy("asset_category")}>
            {t("By category")}
          </button>
          <button type="button" aria-pressed={by === "location"} onClick={() => setBy("location")}>
            {t("By place")}
          </button>
        </div>
      </div>
      <div className="card-body">
        {error ? (
          <SectionError error={error} retry={retry} />
        ) : loading || !rows ? (
          <Skeleton rows={4} />
        ) : !shown.length ? (
          <p className="muted">{t("Nothing registered yet.")}</p>
        ) : (
          <ul className="ast-bars">
            {shown.map((b: Bar) => {
              const name = b.id ? <Link to={listPath({ ...query, [by]: b.id })}>{isolate(b.name)}</Link> : <span>{isolate(b.name)}</span>;
              return (
                <li key={b.name}>
                  <div className="ast-bar-line">
                    {name}
                    <span className="faint">{b.assets === 1 ? t("1 asset") : t("{assets} assets", { assets: b.assets })}</span>
                    <span className="spacer" />
                    <strong className="num">{isolate(money(b.value))}</strong>
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
          const said = t("{label}: {value} booked, {value2} still to book{value3}", { label, value: money(m.booked), value2: money(m.waiting), value3: m.failed ? ", some could not be booked" : m.late ? ", some is overdue" : "" });
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
                <span className="ast-month-name">{isolate(label)}</span>
              </Link>
            </li>
          );
        })}
      </ol>
      <p className="ast-legend faint">
        {placed(t("{p1} booked {p2} still to book{p3} overdue {p4} could not be booked"), { p1: <span className="ast-key ast-key-booked" aria-hidden="true" />, p2: <span className="ast-key ast-key-waiting" aria-hidden="true" />, p3: <span className="ast-key ast-key-late" aria-hidden="true" />, p4: <span className="ast-key ast-key-failed" aria-hidden="true" /> })}
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

const TONE_WORD: Record<Tone, string> = { get bad() { return t("Needs action"); }, get warn() { return t("Coming up"); }, get quiet() { return t("For information"); } };

function Attention({ groups, loading, error, retry }: { groups: Group[]; loading: boolean; error: unknown; retry: () => void }) {
  const shown = groups.filter((g) => g.total > 0);
  return (
    <section className="card ast-attention" aria-labelledby="ast-attention">
      <div className="card-head">
        <h2 id="ast-attention">{t("Needs attention")}</h2>
      </div>
      <div className="card-body">
        {error ? <SectionError error={error} retry={retry} /> : null}
        {loading && !shown.length ? (
          <Skeleton rows={4} />
        ) : !shown.length ? (
          error ? null : (
            <StatePanel kind="empty" className="pt-4 md:pt-4" title={t("Nothing needs attention")}>
              {t("Returns, checks, repairs and depreciation are all where they should be.")}
            </StatePanel>
          )
        ) : (
          <ul className="ast-groups">
            {shown.map((g) => (
              <li key={g.id} className="ast-group" data-tone={g.tone}>
                <div className="ast-group-head">
                  <span className="ast-group-mark" aria-hidden="true">
                    <Icon name={g.tone === "bad" ? "alert" : g.tone === "warn" ? "calendar" : "check"} size={14} />
                  </span>
                  <Link to={g.to}>{isolate(g.title)}</Link>
                  <span className={`badge ${g.tone === "bad" ? "badge-red" : g.tone === "warn" ? "badge-amber" : ""}`} title={TONE_WORD[g.tone]}>
                    {g.total}
                  </span>
                </div>
                <ul className="ast-lines">
                  {g.lines.map((l) => (
                    <li key={l.id}>
                      <Link to={l.to ?? `/a/${l.id}`}>{isolate(l.title)}</Link>
                      {l.detail && <span className="faint"> {isolate(l.detail)}</span>}
                    </li>
                  ))}
                  {g.total > g.lines.length && (
                    <li>
                      <Link className="faint" to={g.to}>
                        {t("and")} {g.total - g.lines.length} {t("more")}
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
  useShellTitle(t("Assets"));
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
        title: failing ? t("Depreciation could not be booked") : t("Depreciation due, not booked yet"),
        tone: "bad",
        total: behind.total,
        lines: behind.items.map((s) => ({ id: s.id, title: s.title, detail: s.reason ? t("since {value}: {reason}", { value: dayText(s.since), reason: s.reason }) : t("since {value}", { value: dayText(s.since) }) })),
        to: `/r/asset_depreciation_schedule?${new URLSearchParams({ ...params, from_date: behind.items[0]?.since ?? from, to_date: today, run: "1" })}`,
      });
    }
    const overdue = due.data?.rows.filter((r) => r.days_overdue > 0) ?? [];
    if (due.data) {
      found.set("maintenance", {
        id: "maintenance",
        title: t("Maintenance overdue"),
        tone: "bad",
        total: overdue.length,
        lines: overdue.slice(0, 3).map((r) => ({ id: r.asset_id, to: `/a/${r.asset_id}?tab=maintenance`, title: [r.asset, r.asset_name].filter(Boolean).join(" · "), detail: (r.days_overdue === 1 ? t("{task}, 1 day late", { task: r.task_name ?? t("Maintenance") }) : t("{task}, {days} days late", { task: r.task_name ?? t("Maintenance"), days: r.days_overdue })) })),
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
    <StatePanel kind="failed" error={byCategory.error} onRetry={() => void byCategory.refetch()} title={t("The register could not be loaded")} />
  ) : byCategory.data && count === 0 && !byCategory.isFetching && asOf === today ? (
    <StatePanel
      kind="empty"
      title={everything ? t("No assets yet") : t("No assets in your care")}
      action={
        everything && canAdd ? (
          <Link className="btn btn-primary btn-md" to="/e/Asset/new">
            <Icon name="plus" />
            {t("Add an asset you own")}
          </Link>
        ) : undefined
      }
    >
      {everything ? t("Assets are made when a purchase invoice buys a fixed asset item. Ones you already own are added by hand.") : t("Assets handed to you will appear here.")}
    </StatePanel>
  ) : (
    <>
      <section className="ast-tiles" aria-label={t("Totals")}>
        <Tile label={t("Net book value")} value={byCategory.data ? money(totals?.net_book_value) : <span className="skeleton ast-tile-skel" />} sub={byCategory.data ? `${count} ${count === 1 ? "asset" : "assets"}${asOf === today ? "" : t(", as of {asOf}", { asOf: dayText(asOf) })}` : undefined} to={registerLink} />
        <Tile label={t("Cost")} value={byCategory.data ? money(totals?.cost) : <span className="skeleton ast-tile-skel" />} sub={t("what was paid, with repairs and revaluations")} to={registerLink} />
        <Tile label={t("Depreciated")} value={byCategory.data ? `${depreciated(totals?.accumulated_depreciation, totals?.cost)}%` : <span className="skeleton ast-tile-skel" />} sub={byCategory.data ? t("{value} of the cost", { value: money(totals?.accumulated_depreciation) }) : undefined} to={registerLink}>
          <span className="ast-meter" aria-hidden="true">
            <span style={{ width: `${depreciated(totals?.accumulated_depreciation, totals?.cost)}%` }} />
          </span>
        </Tile>
        <Tile
          label={t("Depreciation this month")}
          value={thisMonth ? money(thisMonth.total) : plan.error ? "—" : <span className="skeleton ast-tile-skel" />}
          sub={thisMonth ? t("{value} booked so far", { value: money(thisMonth.booked) }) : undefined}
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
            <h2 id="ast-depreciation">{t("Depreciation, month by month")}</h2>
          </div>
          <div className="card-body">
            {plan.error ? (
              <SectionError error={plan.error} retry={() => void plan.refetch()} />
            ) : !plan.data ? (
              <Skeleton rows={4} />
            ) : (
              <>
                <Months items={upcoming} today={today} money={money} company={company.id} />
                {plan.data.meta?.truncated && <p className="ast-note">{t("There are more periods than a report holds, so the later months are partial. Open a month for its own report.")}</p>}
              </>
            )}
          </div>
        </section>
        <section className="card ast-maintenance" aria-labelledby="ast-maintenance">
          <div className="card-head">
            <h2 id="ast-maintenance">{t("Maintenance in the next 30 days")}</h2>
            <span className="spacer" />
            <Link to={`/r/asset_maintenance_due?${new URLSearchParams({ ...params, as_of: today, days: "90", run: "1" })}`}>{t("All maintenance")}</Link>
          </div>
          <div className="card-body">
            {due.error ? (
              <SectionError error={due.error} retry={() => void due.refetch()} />
            ) : !due.data ? (
              <Skeleton rows={3} />
            ) : !due.data.rows.length ? (
              <p className="muted">{t("No maintenance is due.")}</p>
            ) : (
              <ul className="ast-lines ast-due">
                {due.data.rows.slice(0, 6).map((r) => (
                  <li key={`${r.asset_maintenance}-${r.task_name}-${r.next_due_date}`}>
                    <span className={`badge ${r.days_overdue > 0 ? "badge-red" : "badge-amber"}`}>{r.days_overdue > 0 ? (r.days_overdue === 1 ? t("{days_overdue} day late", { days_overdue: r.days_overdue }) : t("{days_overdue} days late", { days_overdue: r.days_overdue })) : relative(r.next_due_date, today, dayText)}</span>
                    <span>
                      <strong>{r.task_name ?? t("Maintenance")}</strong>{isolate(" ")}
                      <Link className="faint" to={`/a/${r.asset_id}?tab=maintenance`}>
                        {isolate([r.asset, r.asset_name].filter(Boolean).join(" · "))}
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
            {byCategory.data ? `${count} ${count === 1 ? "asset" : "assets"}${byCategory.data.meta?.company ? t(" of {company}", { company: byCategory.data.meta.company }) : ""}` : " "}
          </p>
        </div>
        <div className="ast-tools">
          {many && (
            <select className="select ast-company" aria-label={t("Company")} value={company.id ?? ""} onChange={(e) => company.choose(e.target.value)}>
              {company.all.map((c) => (
                <option key={c.id} value={c.id}>
                  {c.company_name ?? c.id}
                </option>
              ))}
            </select>
          )}
          <label className="ast-asof">
            <span className="faint">{t("As of")}</span>
            <input className="input" type="date" max={today} value={asOf} onChange={(e) => setParam("as_of", isDay(e.target.value) && e.target.value !== today ? e.target.value : null)} />
          </label>
          {canVerify && (
            <Link className="btn btn-primary btn-md" to="/verify">
              <Icon name="check" />
              {t("Verify a room")}
            </Link>
          )}
          {canAdd && (
            <Link className="btn btn-default btn-md" to="/e/Asset/new">
              <Icon name="plus" />
              {t("New asset")}
            </Link>
          )}
        </div>
      </header>
      {body}
    </div>
  );
}
