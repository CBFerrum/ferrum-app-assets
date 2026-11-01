// Verifying a room: walk it, scan or tap what is there, and the register is told as you go. Each scan
// is answered at once by the server, in green or with its reason in red; what the register expects
// here and has not seen is listed, and what turns up in the wrong room can be moved with one tap. A
// hardware scanner types a tag and presses Enter, so the box needs no clicks; on a phone the same box
// sits above the list, and every row can also be ticked by hand.

import { type FormEvent, useEffect, useId, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Link, useSearchParams } from "react-router";
import { api, docPath, errorMessage, newKey, todayIso } from "@ferrum/client";
import { Button, confirmDialog, EmptyState, ErrorState, Field, Icon, Spinner, toast, useShellTitle } from "@ferrum/kit";
import { can, LinkInput, useEntityMeta, useLinkTitle } from "@ferrum/records";
import { create, list } from "./data";
import { Tabs } from "./Tabs";
import type { Asset } from "./types";
import { alreadySeen, type Candidate, clean, last, lookup, type NextCheck, nextCheck, progress, replace, type Scan, without } from "./walk";

const FIELDS = "number,asset_name,serial_number,location,custodian,company";
const LIMIT = 500;

const NEXT: { id: NextCheck; label: string }[] = [
  { id: "category", label: "When its category says" },
  { id: "3", label: "In 3 months" },
  { id: "6", label: "In 6 months" },
  { id: "12", label: "In a year" },
];

const title = (a: Candidate | null, code: string) => (a ? [a.number, a.asset_name].filter(Boolean).join(" · ") : code);

function Setup({ onStart }: { onStart: (place: string, next: NextCheck) => void }) {
  const [place, setPlace] = useState<string | null>(null);
  const [next, setNext] = useState<NextCheck>("category");
  const id = useId();
  return (
    <form
      className="card card-pad stack ast-setup"
      onSubmit={(e) => {
        e.preventDefault();
        if (place) onStart(place, next);
      }}
    >
      <div>
        <h2 className="ast-subhead">Where are you?</h2>
        <p className="muted">Pick the room. You will see what the register says is there; everything you scan or tap is verified in the register at once.</p>
      </div>
      <Field label="Room or place" htmlFor={`${id}-place`} required>
        <LinkInput to="Location" id={`${id}-place`} value={place} required linkFilters={{ is_group: false }} onChange={setPlace} />
      </Field>
      <Field label="Verify again" htmlFor={`${id}-next`} hint="Sets when each asset is next due. Categories with no schedule have none unless you choose one.">
        <select id={`${id}-next`} className="select" value={next} onChange={(e) => setNext(e.target.value as NextCheck)}>
          {NEXT.map((n) => (
            <option key={n.id} value={n.id}>
              {n.label}
            </option>
          ))}
        </select>
      </Field>
      <div className="row">
        <Button type="submit" variant="primary" disabled={!place}>
          Start
        </Button>
      </div>
    </form>
  );
}

const MARK: Record<Scan["status"], { icon: string; word: string }> = {
  saving: { icon: "refresh", word: "Checking" },
  verified: { icon: "check", word: "Verified" },
  elsewhere: { icon: "alert", word: "Not where the register has it" },
  missing: { icon: "alert", word: "Marked as not found" },
  again: { icon: "check", word: "Already seen on this walk" },
  refused: { icon: "alert", word: "Not accepted" },
  unknown: { icon: "alert", word: "No such asset" },
};

function Walk({ place, next, leave }: { place: string; next: NextCheck; leave: () => void }) {
  const qc = useQueryClient();
  // What the walk found is news to the overview: when the walk is left, it asks again.
  useEffect(() => () => void qc.invalidateQueries({ queryKey: ["assets"] }), [qc]);
  const today = todayIso();
  const here = useLinkTitle("Location", place) ?? "this place";
  const box = useRef<HTMLInputElement>(null);
  const [scans, setScans] = useState<Scan[]>([]);
  const latest = useRef<Scan[]>([]);
  const found = useRef(new Map<string, Candidate>());
  const [code, setCode] = useState("");
  const expected = useQuery({
    queryKey: ["assets", "expected", place],
    queryFn: ({ signal }) => list<Asset>("Asset", { filters: [["location", "=", place], ["docstatus", "=", 1], ["disposal", "is", "not set"]], fields: FIELDS, titles: true, sort: "number asc", limit: LIMIT, count: true }, signal),
  });
  const rows: Candidate[] = expected.data?.rows ?? [];
  const p = progress(rows, scans);
  const undoable = last(scans);

  const put = (change: (s: Scan[]) => Scan[]) => {
    latest.current = change(latest.current);
    setScans(latest.current);
  };
  const settle = (key: string, change: Partial<Scan>) => put((s) => replace(s, key, change));
  const refocus = () => box.current?.focus();

  /** Tells the register an asset was seen here, or looked for and not found. */
  const record = async (key: string, asset: Candidate, result: "Found" | "Not Found") => {
    const body = { asset: asset.id, verified_on: today, result, ...(result === "Found" ? { seen_at: place, next_verification_date: nextCheck(next, today) } : {}) };
    const made = await create<{ id: string }>("AssetVerification", body, newKey());
    settle(key, { asset, check: made.id, status: result === "Not Found" ? "missing" : asset.location === place ? "verified" : "elsewhere" });
  };

  const fetchOne = async (entered: string): Promise<Candidate | "many" | null> => {
    const ask = (filter: unknown[]) => list<Asset>("Asset", { filters: [filter, ["docstatus", "=", 1]], fields: FIELDS, titles: true, limit: 2 }).then((r) => r.rows);
    const hits = await ask(["number", "in", [entered, entered.toUpperCase()]]);
    const rest = hits.length ? hits : await ask(["serial_number", "=", entered]);
    return rest.length > 1 ? "many" : (rest[0] ?? null);
  };

  /** One thing seen (typed, scanned, or ticked by hand), or not found. */
  const handle = async (entered: string, known?: Candidate, result: "Found" | "Not Found" = "Found") => {
    const key = newKey();
    put((s) => [{ key, code: entered, asset: known ?? null, status: "saving" }, ...s]);
    try {
      let asset: Candidate | null | "many" = known ?? null;
      if (!asset) {
        const hit = lookup(entered, [...rows, ...found.current.values()]);
        asset = hit.ambiguous ? "many" : (hit.asset ?? (await fetchOne(entered)));
      }
      if (asset === "many") return settle(key, { status: "refused", reason: "More than one asset has that serial number. Scan its tag instead." });
      if (!asset) return settle(key, { status: "unknown", reason: `Nothing is registered under “${entered}”.` });
      found.current.set(asset.id, asset);
      if (alreadySeen(latest.current.filter((s) => s.key !== key), asset)) return settle(key, { asset, status: "again" });
      await record(key, asset, result);
    } catch (e) {
      settle(key, { status: "refused", reason: errorMessage(e) });
    }
  };

  const submit = (e: FormEvent) => {
    e.preventDefault();
    const entered = clean(code);
    setCode("");
    if (entered) void handle(entered);
    refocus();
  };

  const move = async (s: Scan) => {
    if (!s.asset) return;
    settle(s.key, { status: "saving" });
    try {
      await create("AssetMovement", { company: s.asset.company, purpose: "Transfer", transaction_date: today, assets: [{ asset: s.asset.id, target_location: place }] }, newKey());
      settle(s.key, { status: "verified", moved: true });
    } catch (e) {
      settle(s.key, { status: "elsewhere" });
      toast(errorMessage(e), "error");
    }
    refocus();
  };

  const undo = async (s: Scan) => {
    if (!s.check) return;
    try {
      await api.post(docPath("AssetVerification", s.check, "cancel"));
      put((all) => without(all, s.key));
    } catch (e) {
      toast(errorMessage(e), "error");
    }
    refocus();
  };

  const loseRest = async () => {
    const rest = p.waiting;
    const ok = await confirmDialog({
      title: `Mark ${rest.length} ${rest.length === 1 ? "asset" : "assets"} as not found?`,
      message: "They are listed as not found on the overview until someone finds them again.",
      confirmLabel: "Mark as not found",
      danger: true,
    });
    if (!ok) return;
    for (const a of rest) await handle(a.number ?? a.id, a, "Not Found");
    refocus();
  };

  return (
    <div className="ast-page ast-walk">
      <header className="ast-head">
        <div>
          <Tabs />
          <p className="page-sub">Verifying {here}</p>
        </div>
        <div className="ast-tools">
          <Button onClick={leave}>Change room</Button>
        </div>
      </header>

      <section className="ast-scanbar" aria-label="Scan">
        <div className="ast-count" aria-live="polite">
          <strong>
            {p.done} of {p.total}
          </strong>{" "}
          <span className="muted">expected here, verified</span>
          {p.missing > 0 && <span className="badge badge-red">{p.missing} not found</span>}
          {p.elsewhere.length > 0 && <span className="badge badge-amber">{p.elsewhere.length} from elsewhere</span>}
          <span className="ast-progress" aria-hidden="true">
            <span style={{ width: `${p.total ? (p.done / p.total) * 100 : 0}%` }} />
          </span>
        </div>
        <form className="ast-scan" onSubmit={submit}>
          <input ref={box} className="input" autoFocus autoComplete="off" autoCapitalize="off" spellCheck={false} aria-label="Scan or type a tag or serial number" placeholder="Scan or type a tag or serial number" value={code} onChange={(e) => setCode(e.target.value)} />
          <Button type="submit" variant="primary" disabled={!clean(code)}>
            Add
          </Button>
          <Button disabled={!undoable} onClick={() => undoable && void undo(undoable)}>
            Undo last
          </Button>
        </form>
      </section>

      <p className="sr-only" role="status">
        {scans[0] ? `${title(scans[0].asset, scans[0].code)}: ${scans[0].reason ?? MARK[scans[0].status].word}` : ""}
      </p>

      <div className="ast-walk-cols">
        <section className="card" aria-labelledby="ast-seen">
          <div className="card-head">
            <h2 id="ast-seen">Scanned</h2>
          </div>
          {scans.length === 0 ? (
            <p className="card-body muted">Nothing yet. Scan a tag, or tick things off in the list beside this.</p>
          ) : (
            <ul className="ast-scans">
              {scans.map((s) => {
                const mark = MARK[s.status];
                return (
                  <li key={s.key} data-status={s.status}>
                    <span className="ast-scan-mark" aria-hidden="true">
                      {s.status === "saving" ? <Spinner small /> : <Icon name={mark.icon} size={14} />}
                    </span>
                    <div className="ast-scan-body">
                      {s.asset ? <Link to={`/a/${s.asset.id}`}>{title(s.asset, s.code)}</Link> : <strong>{s.code}</strong>}
                      <div className="faint">
                        {s.reason ?? mark.word}
                        {s.moved && ", moved here"}
                        {s.status === "elsewhere" && s.asset?._titles?.location && `: recorded at ${s.asset._titles.location}`}
                      </div>
                    </div>
                    {s.status === "elsewhere" && (
                      <button type="button" className="btn btn-default btn-sm" onClick={() => void move(s)}>
                        Move here
                      </button>
                    )}
                    {s.check && (
                      <button type="button" className="btn btn-ghost btn-sm" onClick={() => void undo(s)} aria-label={`Undo ${title(s.asset, s.code)}`}>
                        Undo
                      </button>
                    )}
                  </li>
                );
              })}
            </ul>
          )}
        </section>

        <section className="card" aria-labelledby="ast-waiting">
          <div className="card-head">
            <h2 id="ast-waiting">Still to find{expected.data ? ` (${p.waiting.length})` : ""}</h2>
            <span className="spacer" />
            {p.waiting.length > 0 && !expected.isFetching && (
              <button type="button" className="link-btn" onClick={() => void loseRest()}>
                Mark the rest as not found
              </button>
            )}
          </div>
          {expected.error ? (
            <ErrorState error={expected.error} onRetry={() => void expected.refetch()} title="The register for this place could not be loaded" />
          ) : !expected.data ? (
            <div className="card-body">
              <Spinner label="Loading what is expected here" />
            </div>
          ) : rows.length === 0 ? (
            <EmptyState title="No assets are registered here" icon="file">
              <p className="muted">Anything you scan will be verified and can be moved here.</p>
            </EmptyState>
          ) : p.waiting.length === 0 ? (
            <EmptyState title="Everything expected here is accounted for" icon="check" />
          ) : (
            <ul className="ast-waiting">
              {p.waiting.map((a) => (
                <li key={a.id}>
                  <div className="ast-scan-body">
                    <Link to={`/a/${a.id}`}>{title(a, a.id)}</Link>
                    <div className="faint">{[a.serial_number && `serial ${a.serial_number}`, a.custodian].filter(Boolean).join(" · ")}</div>
                  </div>
                  <button type="button" className="btn btn-default btn-sm" onClick={() => void handle(a.number ?? a.id, a)} aria-label={`${title(a, a.id)} is here`}>
                    It is here
                  </button>
                </li>
              ))}
            </ul>
          )}
          {(expected.data?.total ?? 0) > LIMIT && <p className="card-body ast-note">Only the first {LIMIT} are listed. Anything else you scan is found all the same.</p>}
        </section>
      </div>
    </div>
  );
}

export function Verify() {
  useShellTitle("Verify");
  const [sp, setSp] = useSearchParams();
  const meta = useEntityMeta("AssetVerification");
  const place = sp.get("place");
  const next = (NEXT.find((n) => n.id === sp.get("next"))?.id ?? "category") as NextCheck;
  if (meta.error) return <ErrorState error={meta.error} onRetry={() => void meta.refetch()} />;
  if (!meta.data) return <Spinner label="Loading" />;
  if (!can(meta.data.permissions, "create")) {
    return (
      <EmptyState title="Verifying is for asset managers" icon="alert">
        <p className="muted">You can see the assets you hold, and when each was last verified, on their pages.</p>
        <Link to="/">Back to the overview</Link>
      </EmptyState>
    );
  }
  if (!place) {
    return (
      <div className="ast-page">
        <header className="ast-head">
          <div>
            <Tabs />
            <p className="page-sub">Walk a room and confirm what is there</p>
          </div>
        </header>
        <Setup onStart={(chosen, when) => setSp({ place: chosen, next: when }, { replace: false })} />
      </div>
    );
  }
  return <Walk key={`${place}-${next}`} place={place} next={next} leave={() => setSp({}, { replace: false })} />;
}
