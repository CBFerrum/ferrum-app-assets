// What a person does to an asset from its page: move it, hand it out or take it back; revalue it;
// sell or scrap it; log maintenance done or report a fault; confirm it is there; correct what it says
// about itself. Each is a short question and the server's answer: the rules (who may, which dates,
// which accounts) are the server's, and a refusal is shown in its words, beside the field it names.

import { type FormEvent, useId, useState } from "react";
import { Link } from "react-router";
import { api, docPath, errorMessage, newKey, shell, todayIso } from "@ferrum/client";
import { Alert, Button, Dialog, Field, Spinner, toast } from "@ferrum/kit";
import { LinkInput, usePeople } from "@ferrum/records";
import { action, create, draft, list } from "./data";
import { dayText, moneyIn } from "./format";
import { useAct } from "./forms";
import { minus, units } from "./money";
import { usePlan } from "./plan";
import type { Asset } from "./types";

export type DialogKind = "transfer" | "handout" | "takeback" | "revalue" | "sell" | "scrap" | "service" | "fault" | "verify" | "details";

interface Props {
  asset: Asset;
  onClose: () => void;
  /** The work is done: the page asks again for what changed. */
  onDone: () => void;
}

const label = (asset: Asset) => asset.asset_name ?? asset.number ?? "this asset";

function Buttons({ busy, onClose, go, danger, disabled }: { busy: boolean; onClose: () => void; go: string; danger?: boolean; disabled?: boolean }) {
  return (
    <div className="row ast-dialog-buttons">
      <Button onClick={onClose}>Cancel</Button>
      <Button type="submit" variant={danger ? "danger" : "primary"} loading={busy} disabled={disabled}>
        {go}
      </Button>
    </div>
  );
}

/** A date no later than today, the way the server wants business dates. */
function DayInput({ id, value, onChange, min, max = todayIso(), required = true }: { id: string; value: string; onChange: (v: string) => void; min?: string; max?: string; required?: boolean }) {
  return <input id={id} className="input" type="date" required={required} min={min} max={max || undefined} value={value} onChange={(e) => onChange(e.target.value)} />;
}

/** Someone in the workspace, by name or address; the address is what the register keeps. */
function PersonInput({ id, value, onChange, invalid }: { id: string; value: string; onChange: (v: string) => void; invalid?: boolean }) {
  const people = usePeople().data?.users ?? [];
  const listId = useId();
  return (
    <>
      <input id={id} className="input" type="email" required list={listId} autoComplete="off" placeholder="Name or email address" aria-invalid={invalid || undefined} value={value} onChange={(e) => onChange(e.target.value)} />
      <datalist id={listId}>
        {people.map((p) => (
          <option key={p.id} value={p.email} label={p.full_name} />
        ))}
      </datalist>
    </>
  );
}

// ── move, hand out, take back ──

const PURPOSE = { transfer: "Transfer", handout: "Issue", takeback: "Receipt" } as const;

export function MoveDialog({ asset, mode, onClose, onDone }: Props & { mode: "transfer" | "handout" | "takeback" }) {
  const act = useAct();
  const today = todayIso();
  const [place, setPlace] = useState<string | null>(null);
  const [who, setWho] = useState("");
  const [due, setDue] = useState("");
  const [on, setOn] = useState(today);
  const [note, setNote] = useState("");
  const id = useId();
  const title = mode === "transfer" ? `Move ${label(asset)}` : mode === "handout" ? `Hand out ${label(asset)}` : `Take back ${label(asset)}`;
  const submit = async (e: FormEvent) => {
    e.preventDefault();
    const row: Record<string, unknown> = { asset: asset.id };
    if (place) row.target_location = place;
    if (mode === "handout") {
      row.to_custodian = who.trim().toLowerCase();
      if (due) row.expected_return_date = due;
    }
    const body = { company: asset.company, purpose: PURPOSE[mode], transaction_date: on, remarks: note.trim() || undefined, assets: [row] };
    if (await act.run(() => create("AssetMovement", body, newKey()))) {
      toast(mode === "transfer" ? "Moved" : mode === "handout" ? `Handed to ${who.trim()}` : "Taken back");
      onDone();
    }
  };
  const general = act.general(["target_location", "to_custodian", "expected_return_date", "transaction_date", "asset"]);
  return (
    <Dialog open onClose={onClose} title={title}>
      <form className="stack" onSubmit={submit}>
        {general && <Alert tone="error">{general}</Alert>}
        {mode === "takeback" && <p className="muted">It is with {asset.custodian}. Taking it back clears who has it.</p>}
        {mode === "handout" && (
          <Field label="Who gets it" htmlFor={`${id}-who`} required error={act.field("to_custodian")}>
            <PersonInput id={`${id}-who`} value={who} onChange={setWho} invalid={!!act.field("to_custodian")} />
          </Field>
        )}
        <Field
          label={mode === "takeback" ? "Put it back in" : mode === "handout" ? "Where it goes" : "Where to"}
          htmlFor={`${id}-place`}
          required={mode !== "handout"}
          error={act.field("target_location")}
          hint={mode === "handout" ? "Leave empty if it stays where it is." : undefined}
        >
          <LinkInput to="Location" id={`${id}-place`} value={place} required={mode !== "handout"} linkFilters={{ is_group: false }} onChange={setPlace} invalid={!!act.field("target_location")} />
        </Field>
        {mode === "handout" && (
          <Field label="Back by" htmlFor={`${id}-due`} error={act.field("expected_return_date")} hint="Optional. The overview lists what is not back by then.">
            <DayInput id={`${id}-due`} value={due} onChange={setDue} min={on} max="" required={false} />
          </Field>
        )}
        <Field label="Date" htmlFor={`${id}-on`} required error={act.field("transaction_date")}>
          <DayInput id={`${id}-on`} value={on} onChange={setOn} />
        </Field>
        <Field label="Note" htmlFor={`${id}-note`}>
          <textarea id={`${id}-note`} className="textarea" rows={2} value={note} onChange={(e) => setNote(e.target.value)} />
        </Field>
        <Buttons busy={act.busy} onClose={onClose} go={mode === "transfer" ? "Move" : mode === "handout" ? "Hand out" : "Take back"} disabled={mode === "handout" ? !who.trim() : !place} />
      </form>
    </Dialog>
  );
}

// ── revalue ──

export function RevalueDialog({ asset, onClose, onDone }: Props) {
  const act = useAct();
  const today = todayIso();
  const [value, setValue] = useState("");
  const [on, setOn] = useState(today);
  const [account, setAccount] = useState<string | null>(null);
  const [why, setWhy] = useState("");
  const id = useId();
  const money = moneyIn(asset._titles?.currency);
  const now = asset.value_after_depreciation ?? "0";
  const typed = units(value);
  const change = typed === null ? null : minus(value, now);
  const submit = async (e: FormEvent) => {
    e.preventDefault();
    const body = { asset: asset.id, adjustment_date: on, new_asset_value: value.trim(), difference_account: account, remarks: why.trim() || undefined };
    const made = await act.run(() => create<{ current_asset_value?: string; new_asset_value?: string }>("AssetValueAdjustment", body, newKey()).then((r) => toast(`Revalued from ${money(r.current_asset_value)} to ${money(r.new_asset_value)}`)));
    if (made) onDone();
  };
  const general = act.general(["new_asset_value", "adjustment_date", "difference_account", "asset"]);
  return (
    <Dialog open onClose={onClose} title={`Revalue ${label(asset)}`}>
      <form className="stack" onSubmit={submit}>
        {general && <Alert tone="error">{general}</Alert>}
        <p className="muted">
          Worth <strong>{money(now)}</strong> in the books now. Depreciation due by the date is booked first, and what is left is spread over the rest of its life.
        </p>
        <Field
          label="What it is worth"
          htmlFor={`${id}-value`}
          required
          error={act.field("new_asset_value") ?? (value.trim() && typed === null ? "Write a number, such as 1200.50" : undefined)}
          hint={change === null || change === "0" ? undefined : `A change of ${change.startsWith("-") ? "" : "+"}${money(change)} on today's book value.`}
        >
          <input id={`${id}-value`} className="input num" inputMode="decimal" required autoComplete="off" value={value} onChange={(e) => setValue(e.target.value)} aria-invalid={!!act.field("new_asset_value") || undefined} />
        </Field>
        <Field label="As of" htmlFor={`${id}-on`} required error={act.field("adjustment_date")}>
          <DayInput id={`${id}-on`} value={on} onChange={setOn} min={asset.available_for_use_date ?? undefined} />
        </Field>
        <Field
          label="Book the difference to"
          htmlFor={`${id}-account`}
          required
          error={act.field("difference_account")}
          hint={change?.startsWith("-") ? "An impairment loss account." : change && change !== "0" ? "A revaluation surplus account." : "An impairment loss for less, a revaluation surplus for more."}
        >
          <LinkInput to="Account" id={`${id}-account`} value={account} required linkFilters={{ is_group: false, ...(asset.company ? { company: asset.company } : {}) }} onChange={setAccount} invalid={!!act.field("difference_account")} />
        </Field>
        <Field label="Why" htmlFor={`${id}-why`}>
          <textarea id={`${id}-why`} className="textarea" rows={2} value={why} onChange={(e) => setWhy(e.target.value)} />
        </Field>
        <Buttons busy={act.busy} onClose={onClose} go="Revalue" disabled={typed === null || !account} />
      </form>
    </Dialog>
  );
}

// ── sell or scrap ──

export function DisposeDialog({ asset, mode, onClose, onDone }: Props & { mode: "sell" | "scrap" }) {
  const act = useAct();
  const today = todayIso();
  const [on, setOn] = useState(today);
  const [customer, setCustomer] = useState<string | null>(null);
  const [price, setPrice] = useState("");
  const [note, setNote] = useState("");
  const [saved, setSaved] = useState<string | null>(null);
  const id = useId();
  const money = moneyIn(asset._titles?.currency);
  const submit = async (e: FormEvent) => {
    e.preventDefault();
    const ok = await act.run(async () => {
      if (mode === "scrap") {
        const r = await create<{ net_book_value?: string }>("AssetDisposal", { asset: asset.id, disposal_type: "Scrap", disposal_date: on, remarks: note.trim() || undefined }, newKey());
        toast(`Scrapped: ${money(r.net_book_value)} written off`);
        return;
      }
      // A sale is finished by its sales invoice: the disposal is kept as a draft and the invoice made from it.
      const made = saved ? { id: saved } : await draft("AssetDisposal", { asset: asset.id, disposal_type: "Sell", disposal_date: on, customer, sale_amount: price.trim(), remarks: note.trim() || undefined }, newKey());
      setSaved(made.id);
      const invoice = await action<{ entity?: string; id?: string }>("AssetDisposal", made.id, "make_sales_invoice");
      toast("The sale is a draft sales invoice: submit it to complete the sale");
      if (invoice.id) shell().navigate({ entity: invoice.entity ?? "SalesInvoice", id: invoice.id });
    });
    if (ok) onDone();
  };
  const general = act.general(["disposal_date", "customer", "sale_amount", "asset"]);
  return (
    <Dialog open onClose={onClose} title={mode === "sell" ? `Sell ${label(asset)}` : `Scrap ${label(asset)}`}>
      <form className="stack" onSubmit={submit}>
        {general && <Alert tone="error">{general}</Alert>}
        {saved && (
          <Alert tone="info">
            The sale is saved as a draft. <Link to={`/e/AssetDisposal/${saved}`}>Open it</Link>
          </Alert>
        )}
        <p className="muted">
          {mode === "sell"
            ? "Depreciation up to the date is booked, then a sales invoice is made for you to check and submit. Submitting it completes the sale and books the gain or loss."
            : `Depreciation up to the date is booked, then ${money(asset.value_after_depreciation)} is written off. It can be undone by cancelling the disposal.`}
        </p>
        {mode === "sell" && (
          <>
            <Field label="Customer" htmlFor={`${id}-customer`} required error={act.field("customer")}>
              <LinkInput to="Customer" id={`${id}-customer`} value={customer} required onChange={setCustomer} invalid={!!act.field("customer")} />
            </Field>
            <Field label="Price" htmlFor={`${id}-price`} required error={act.field("sale_amount") ?? (price.trim() && units(price) === null ? "Write a number, such as 1200.50" : undefined)} hint="Before taxes, in the company's currency.">
              <input id={`${id}-price`} className="input num" inputMode="decimal" required autoComplete="off" value={price} onChange={(e) => setPrice(e.target.value)} aria-invalid={!!act.field("sale_amount") || undefined} />
            </Field>
          </>
        )}
        <Field label={mode === "sell" ? "Date of sale" : "Date"} htmlFor={`${id}-on`} required error={act.field("disposal_date")}>
          <DayInput id={`${id}-on`} value={on} onChange={setOn} min={asset.available_for_use_date ?? undefined} />
        </Field>
        <Field label="Note" htmlFor={`${id}-note`}>
          <textarea id={`${id}-note`} className="textarea" rows={2} value={note} onChange={(e) => setNote(e.target.value)} />
        </Field>
        <Buttons busy={act.busy} onClose={onClose} go={mode === "sell" ? "Make the sales invoice" : "Scrap it"} danger={mode === "scrap"} disabled={mode === "sell" && (!customer || units(price) === null)} />
      </form>
    </Dialog>
  );
}

// ── a fault, and maintenance done ──

export function FaultDialog({ asset, onClose, onDone }: Props) {
  const act = useAct();
  const [on, setOn] = useState(todayIso());
  const [what, setWhat] = useState("");
  const id = useId();
  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if (await act.run(() => draft("AssetRepair", { asset: asset.id, failure_date: on, description: what.trim() }, newKey()))) {
      toast("Reported: the asset is out of order until it is repaired");
      onDone();
    }
  };
  const general = act.general(["failure_date", "description", "asset"]);
  return (
    <Dialog open onClose={onClose} title={`Report a fault on ${label(asset)}`}>
      <form className="stack" onSubmit={submit}>
        {general && <Alert tone="error">{general}</Alert>}
        <Field label="What is wrong" htmlFor={`${id}-what`} required error={act.field("description")}>
          <textarea id={`${id}-what`} className="textarea" rows={3} required value={what} onChange={(e) => setWhat(e.target.value)} />
        </Field>
        <Field label="Since" htmlFor={`${id}-on`} required error={act.field("failure_date")}>
          <DayInput id={`${id}-on`} value={on} onChange={setOn} />
        </Field>
        <Buttons busy={act.busy} onClose={onClose} go="Report the fault" disabled={!what.trim()} />
      </form>
    </Dialog>
  );
}

export function ServiceDialog({ asset, task: first, onClose, onDone }: Props & { task?: string }) {
  const act = useAct();
  const [task, setTask] = useState(first ?? "");
  const [on, setOn] = useState(todayIso());
  const [done, setDone] = useState("");
  const id = useId();
  const plan = usePlan(asset.id);
  const chosen = task || plan.data?.tasks[0]?.id || "";
  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if (!plan.data) return;
    const ok = await act.run(async () => {
      // The upkeep opens a planned log when a task falls due: finish that one rather than make a second.
      const open = await list<{ id: string; version: number }>("AssetMaintenanceLog", {
        filters: [["asset_maintenance", "=", plan.data!.id], ["task", "=", chosen], ["docstatus", "=", 0], ["maintenance_status", "in", ["Planned", "In Progress"]]],
        fields: "version",
        limit: 1,
      });
      const finished = { maintenance_status: "Completed", completion_date: on, actions_performed: done.trim() || undefined };
      if (open.rows[0]) {
        await api.patch(docPath("AssetMaintenanceLog", open.rows[0].id), finished, { ifMatch: open.rows[0].version });
        await api.post(docPath("AssetMaintenanceLog", open.rows[0].id, "submit"));
      } else {
        await create("AssetMaintenanceLog", { asset_maintenance: plan.data!.id, task: chosen, ...finished }, newKey());
      }
    });
    if (ok) {
      toast("Maintenance logged");
      onDone();
    }
  };
  const general = act.general(["completion_date", "task", "actions_performed"]);
  return (
    <Dialog open onClose={onClose} title={`Log maintenance on ${label(asset)}`}>
      {plan.isLoading ? (
        <Spinner label="Loading the maintenance plan" />
      ) : plan.error ? (
        <Alert tone="error">{errorMessage(plan.error)}</Alert>
      ) : !plan.data ? (
        <div className="stack">
          <p className="muted">This asset has no maintenance plan yet. A plan lists its tasks and how often each falls due; the overview then shows what is coming up.</p>
          <div className="row ast-dialog-buttons">
            <Button onClick={onClose}>Cancel</Button>
            <Link className="btn btn-primary btn-md" to={`/e/AssetMaintenance/new?asset=${asset.id}`}>
              Plan maintenance
            </Link>
          </div>
        </div>
      ) : (
        <form className="stack" onSubmit={submit}>
          {general && <Alert tone="error">{general}</Alert>}
          <Field label="Task" htmlFor={`${id}-task`} required error={act.field("task")}>
            <select id={`${id}-task`} className="select" value={chosen} onChange={(e) => setTask(e.target.value)}>
              {plan.data.tasks.map((t) => (
                <option key={t.id} value={t.id}>
                  {t.task_name}
                  {t.periodicity ? `, ${t.periodicity.toLowerCase()}` : ""}
                  {t.next_due_date ? `, due ${dayText(t.next_due_date)}` : ""}
                </option>
              ))}
            </select>
          </Field>
          <Field label="Done on" htmlFor={`${id}-on`} required error={act.field("completion_date")}>
            <DayInput id={`${id}-on`} value={on} onChange={setOn} />
          </Field>
          <Field label="What was done" htmlFor={`${id}-done`} error={act.field("actions_performed")}>
            <textarea id={`${id}-done`} className="textarea" rows={3} value={done} onChange={(e) => setDone(e.target.value)} />
          </Field>
          <Buttons busy={act.busy} onClose={onClose} go="Log it" disabled={!chosen} />
        </form>
      )}
    </Dialog>
  );
}

// ── seen in person ──

export function VerifyDialog({ asset, onClose, onDone }: Props) {
  const act = useAct();
  const [found, setFound] = useState(true);
  const [place, setPlace] = useState<string | null>(asset.location ?? null);
  const [note, setNote] = useState("");
  const [on, setOn] = useState(todayIso());
  const id = useId();
  const submit = async (e: FormEvent) => {
    e.preventDefault();
    const body = { asset: asset.id, verified_on: on, result: found ? "Found" : "Not Found", seen_at: found ? place : undefined, note: note.trim() || undefined };
    if (await act.run(() => create("AssetVerification", body, newKey()))) {
      toast(found ? "Verified" : "Marked as not found");
      onDone();
    }
  };
  const general = act.general(["verified_on", "seen_at", "result", "asset"]);
  return (
    <Dialog open onClose={onClose} title={`Verify ${label(asset)}`}>
      <form className="stack" onSubmit={submit}>
        {general && <Alert tone="error">{general}</Alert>}
        <div className="segmented" role="group" aria-label="What you found">
          <button type="button" aria-pressed={found} onClick={() => setFound(true)}>
            It is here
          </button>
          <button type="button" aria-pressed={!found} onClick={() => setFound(false)}>
            Not found
          </button>
        </div>
        {found && (
          <Field label="Seen at" htmlFor={`${id}-place`} error={act.field("seen_at")} hint={place && place !== asset.location ? "Not where the register has it. Verify, then move it from its page." : undefined}>
            <LinkInput to="Location" id={`${id}-place`} value={place} title={place === asset.location ? asset._titles?.location : undefined} linkFilters={{ is_group: false }} onChange={setPlace} />
          </Field>
        )}
        <Field label="Date" htmlFor={`${id}-on`} required error={act.field("verified_on")}>
          <DayInput id={`${id}-on`} value={on} onChange={setOn} min={asset.purchase_date ?? undefined} />
        </Field>
        <Field label="Note" htmlFor={`${id}-note`}>
          <textarea id={`${id}-note`} className="textarea" rows={2} value={note} onChange={(e) => setNote(e.target.value)} />
        </Field>
        <Buttons busy={act.busy} onClose={onClose} go={found ? "Verify" : "Mark as not found"} danger={!found} />
      </form>
    </Dialog>
  );
}

// ── what it says about itself ──

export function DetailsDialog({ asset, onClose, onDone }: Props) {
  const act = useAct();
  const [v, setV] = useState({
    serial_number: asset.serial_number ?? "",
    manufacturer: asset.manufacturer ?? "",
    model_number: asset.model_number ?? "",
    warranty_expiry_date: asset.warranty_expiry_date ?? "",
    end_of_life_date: asset.end_of_life_date ?? "",
  });
  const id = useId();
  const set = (name: keyof typeof v) => (value: string) => setV((x) => ({ ...x, [name]: value }));
  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if (await act.run(() => action("Asset", asset.id, "update_details", v, asset.version))) {
      toast("Details saved");
      onDone();
    }
  };
  const general = act.general(Object.keys(v));
  return (
    <Dialog open onClose={onClose} title={`Details of ${label(asset)}`}>
      <form className="stack" onSubmit={submit}>
        {general && <Alert tone="error">{general}</Alert>}
        <Field label="Serial number" htmlFor={`${id}-serial`} error={act.field("serial_number")}>
          <input id={`${id}-serial`} className="input" autoComplete="off" maxLength={120} value={v.serial_number} onChange={(e) => set("serial_number")(e.target.value)} />
        </Field>
        <Field label="Manufacturer" htmlFor={`${id}-maker`} error={act.field("manufacturer")}>
          <input id={`${id}-maker`} className="input" autoComplete="off" maxLength={120} value={v.manufacturer} onChange={(e) => set("manufacturer")(e.target.value)} />
        </Field>
        <Field label="Model" htmlFor={`${id}-model`} error={act.field("model_number")}>
          <input id={`${id}-model`} className="input" autoComplete="off" maxLength={120} value={v.model_number} onChange={(e) => set("model_number")(e.target.value)} />
        </Field>
        <div className="ast-pair">
          <Field label="Warranty ends" htmlFor={`${id}-warranty`} error={act.field("warranty_expiry_date")}>
            <DayInput id={`${id}-warranty`} value={v.warranty_expiry_date} onChange={set("warranty_expiry_date")} max="" required={false} />
          </Field>
          <Field label="End of life" htmlFor={`${id}-eol`} error={act.field("end_of_life_date")} hint="When it is due to be replaced.">
            <DayInput id={`${id}-eol`} value={v.end_of_life_date} onChange={set("end_of_life_date")} max="" required={false} />
          </Field>
        </div>
        <Buttons busy={act.busy} onClose={onClose} go="Save" />
      </form>
    </Dialog>
  );
}
