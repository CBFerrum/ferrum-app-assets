// An asset on one page: what it is, where it is, who has it, what it is worth now and over time, its
// depreciation, its maintenance and its whole history, with what a person does to it (move it, hand
// it out, maintain it, revalue it, sell or scrap it, verify it) one step away. The server decides
// what is allowed; the page offers what the person's roles let them try, and shows its reasons.

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { type ChangeEvent, type ReactNode, useCallback, useEffect, useRef, useState } from "react";
import { Link, useParams, useSearchParams } from "react-router";
import { api, docPath, errorMessage, fetchBlob, todayIso, uploadFile } from "@ferrum/client";
import { Alert, Button, ErrorState, Icon, PageLoading, TabPanel, Tabs, toast, useLiveEntity, useShellTitle } from "@ferrum/kit";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger } from "@ferrum/kit/primitives";
import { can, CollabPanel, type EntityMeta, useEntityMeta, useRecord } from "@ferrum/records";
import { DetailsDialog, type DialogKind, DisposeDialog, FaultDialog, MoveDialog, RevalueDialog, ServiceDialog, VerifyDialog } from "./Actions";
import { Depreciation } from "./Depreciation";
import { Maintenance } from "./Maintenance";
import { Story } from "./Story";
import { phrase, urgency } from "./dates";
import { dayText, moneyIn } from "./format";
import { depreciated } from "./register";
import type { Asset } from "./types";

const TABS = [
  { id: "depreciation", label: "Depreciation" },
  { id: "history", label: "History" },
  { id: "maintenance", label: "Maintenance" },
  { id: "files", label: "Files and discussion" },
];

/** A status as a badge: what is fine is calm, what needs a hand is not. */
const STATUS_BADGE: Record<string, string> = {
  Submitted: "badge-blue",
  "Partially Depreciated": "badge-blue",
  "Fully Depreciated": "",
  "In Maintenance": "badge-amber",
  "Out of Order": "badge-red",
  Sold: "badge-green",
  Scrapped: "",
  Cancelled: "badge-red",
  Draft: "",
};

function useRights() {
  const ask = (entity: string) => can(useEntityMeta(entity).data?.permissions, "create");
  return { move: ask("AssetMovement"), revalue: ask("AssetValueAdjustment"), dispose: ask("AssetDisposal"), fault: ask("AssetRepair"), log: ask("AssetMaintenanceLog"), plan: ask("AssetMaintenance"), verify: ask("AssetVerification") };
}

/** The photo: the first picture attached to the asset, fetched with the person's own session. */
function Photo({ asset, canEdit }: { asset: Asset; canEdit: boolean }) {
  const qc = useQueryClient();
  const input = useRef<HTMLInputElement>(null);
  const [busy, setBusy] = useState(false);
  const [src, setSrc] = useState<string | null>(null);
  const picture = useQuery({
    queryKey: ["assets", "photo-of", asset.id],
    queryFn: ({ signal }) =>
      api.get<{ attachments: { id: string; url: string; content_type?: string }[] }>(docPath("Asset", asset.id, "attachments"), undefined, signal).then((r) => r.attachments.find((f) => (f.content_type ?? "").startsWith("image/")) ?? null),
  });
  const blob = useQuery({ queryKey: ["assets", "photo", picture.data?.id], enabled: !!picture.data, queryFn: () => fetchBlob(picture.data!.url), staleTime: Infinity });
  useEffect(() => {
    if (!blob.data) return setSrc(null);
    const url = URL.createObjectURL(blob.data);
    setSrc(url);
    return () => URL.revokeObjectURL(url);
  }, [blob.data]);
  const add = async (e: ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    e.target.value = "";
    if (!file) return;
    setBusy(true);
    try {
      await uploadFile(file, { entity: "Asset", id: asset.id });
      toast("Photo added");
      await qc.invalidateQueries({ queryKey: ["assets", "photo-of", asset.id] });
    } catch (err) {
      toast(errorMessage(err), "error");
    } finally {
      setBusy(false);
    }
  };
  return (
    <figure className="ast-photo">
      {src ? (
        <img src={src} alt={`Photo of ${asset.asset_name ?? "the asset"}`} />
      ) : (
        <div className="ast-photo-empty">
          <Icon name="file" size={26} />
          <span>{picture.isLoading || blob.isLoading ? "Loading the photo" : "No photo yet"}</span>
        </div>
      )}
      {canEdit && (
        <>
          <input ref={input} type="file" accept="image/*" className="sr-only" tabIndex={-1} aria-hidden="true" onChange={(e) => void add(e)} />
          <Button size="sm" icon="upload" loading={busy} onClick={() => input.current?.click()}>
            {src ? "Change the photo" : "Add a photo"}
          </Button>
        </>
      )}
    </figure>
  );
}

function Fact({ name, children }: { name: string; children: ReactNode }) {
  return (
    <div>
      <dt>{name}</dt>
      <dd>{children}</dd>
    </div>
  );
}

/** When a date is near or past, as a chip next to it. */
function When({ date, today, noun }: { date: string; today: string; noun: "back" | "due" | "ends" }) {
  const how = urgency(date, today);
  return (
    <span className={`badge ${how === "late" ? (noun === "ends" ? "" : "badge-red") : how === "soon" ? "badge-amber" : ""}`} title={dayText(date)}>
      {phrase(date, today, noun, dayText)}
    </span>
  );
}

function Facts({ asset, today, rights, canEdit, onDetails, onVerify }: { asset: Asset; today: string; rights: ReturnType<typeof useRights>; canEdit: boolean; onDetails: () => void; onVerify: () => void }) {
  const t = asset._titles ?? {};
  const live = asset.docstatus === 1 && !asset.disposal;
  const identity = [asset.serial_number, asset.manufacturer, asset.model_number, asset.warranty_expiry_date, asset.end_of_life_date].some(Boolean);
  return (
    <aside className="ast-side" aria-label="Facts">
      <Photo asset={asset} canEdit={canEdit} />
      <dl className="ast-facts">
        <Fact name="Where">{asset.location ? <Link to={`/e/Asset?f.location=${asset.location}`}>{t.location}</Link> : "—"}</Fact>
        <Fact name="Held by">
          {asset.custodian ? (
            <>
              {asset.custodian}
              {asset.expected_return_date && (
                <>
                  {" "}
                  <When date={asset.expected_return_date} today={today} noun="back" />
                </>
              )}
            </>
          ) : (
            <span className="faint">Nobody</span>
          )}
        </Fact>
        <Fact name="Category">{asset.asset_category ? <Link to={`/e/AssetCategory/${asset.asset_category}`}>{t.asset_category}</Link> : "—"}</Fact>
        <Fact name="Bought">
          {asset.purchase_date ? dayText(asset.purchase_date) : "—"}
          {asset.purchase_invoice && (
            <>
              {" "}
              <a className="faint" href={`/open/PurchaseInvoice/${asset.purchase_invoice}`}>
                {t.purchase_invoice ?? "invoice"}
              </a>
            </>
          )}
        </Fact>
        {asset.available_for_use_date && asset.available_for_use_date !== asset.purchase_date && <Fact name="In use from">{dayText(asset.available_for_use_date)}</Fact>}
      </dl>
      <div className="ast-section-head">
        <h2>Identity</h2>
        {canEdit && (
          <button type="button" className="link-btn" onClick={onDetails}>
            {identity ? "Edit" : "Add"}
          </button>
        )}
      </div>
      {identity ? (
        <dl className="ast-facts">
          {asset.serial_number && <Fact name="Serial number">{asset.serial_number}</Fact>}
          {asset.manufacturer && <Fact name="Made by">{asset.manufacturer}</Fact>}
          {asset.model_number && <Fact name="Model">{asset.model_number}</Fact>}
          {asset.warranty_expiry_date && (
            <Fact name="Warranty">
              {dayText(asset.warranty_expiry_date)} <When date={asset.warranty_expiry_date} today={today} noun="ends" />
            </Fact>
          )}
          {asset.end_of_life_date && <Fact name="End of life">{dayText(asset.end_of_life_date)}</Fact>}
        </dl>
      ) : (
        <p className="faint">No serial number, model or warranty recorded.</p>
      )}
      {(asset.last_verified_date || live) && (
        <>
          <div className="ast-section-head">
            <h2>Verification</h2>
            {live && rights.verify && (
              <button type="button" className="link-btn" onClick={onVerify}>
                Verify now
              </button>
            )}
          </div>
          <dl className="ast-facts">
            <Fact name="Last verified">
              {asset.last_verified_date ? (
                <>
                  {dayText(asset.last_verified_date)} {asset.verification_result === "Not Found" && <span className="badge badge-red">Not found</span>}
                </>
              ) : (
                <span className="faint">Never</span>
              )}
            </Fact>
            {asset.next_verification_date && (
              <Fact name="Next due">
                {dayText(asset.next_verification_date)} <When date={asset.next_verification_date} today={today} noun="due" />
              </Fact>
            )}
          </dl>
        </>
      )}
    </aside>
  );
}

/** What it is worth, what it cost, how much of that is gone, and what comes next. */
function Numbers({ asset, money }: { asset: Asset; money: (v: unknown) => string }) {
  const next = (asset.schedule ?? []).filter((r) => !r.booked).sort((a, b) => a.schedule_date.localeCompare(b.schedule_date))[0];
  const pct = depreciated(asset.accumulated_depreciation, asset.total_asset_cost);
  return (
    <dl className="ast-numbers">
      <div>
        <dt>Worth now</dt>
        <dd className="ast-worth">{money(asset.value_after_depreciation)}</dd>
      </div>
      <div>
        <dt>Cost</dt>
        <dd>{money(asset.total_asset_cost)}</dd>
        {asset.additional_asset_cost && Number(asset.additional_asset_cost) !== 0 && <dd className="faint">with {money(asset.additional_asset_cost)} of repairs</dd>}
      </div>
      <div>
        <dt>Depreciated</dt>
        <dd>
          {pct}% <span className="faint">({money(asset.accumulated_depreciation)})</span>
        </dd>
        <dd className="ast-meter" aria-hidden="true">
          <span style={{ width: `${pct}%` }} />
        </dd>
      </div>
      <div>
        <dt>Next depreciation</dt>
        <dd>{next ? money(next.depreciation_amount) : "—"}</dd>
        {next && <dd className="faint">{dayText(next.schedule_date)}</dd>}
      </div>
    </dl>
  );
}

function Menu({ label, primary, children }: { label: string; primary?: boolean; children: ReactNode }) {
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button type="button" className={`btn ${primary ? "btn-primary" : "btn-default"} btn-md`}>
          {label}
          <Icon name="chevronDown" size={14} />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">{children}</DropdownMenuContent>
    </DropdownMenu>
  );
}

function AssetPage({ meta, id }: { meta: EntityMeta; id: string }) {
  const qc = useQueryClient();
  const { record } = useRecord(meta, id);
  const asset = record.data as unknown as Asset | undefined;
  const rights = useRights();
  const [sp, setSp] = useSearchParams();
  const tab = TABS.find((t) => t.id === sp.get("tab"))?.id ?? "depreciation";
  const [dialog, setDialog] = useState<{ kind: DialogKind; task?: string } | null>(null);
  const today = todayIso();
  useShellTitle(asset ? (asset.asset_name ?? "Asset") : null);

  const refresh = useCallback(() => {
    void qc.invalidateQueries({ queryKey: ["peek", "Asset", id] });
    void qc.invalidateQueries({ queryKey: ["assets"] });
  }, [qc, id]);
  useLiveEntity("Asset", useCallback((e) => void (e.id === id && refresh()), [id, refresh]));
  const choose = (next: string) => {
    setSp((p) => { const n = new URLSearchParams(p); if (next === "depreciation") n.delete("tab"); else n.set("tab", next); return n; }, { replace: true });
    if (next === "files") void qc.invalidateQueries({ queryKey: ["assets", "photo-of", id] });
  };

  if (record.error) return <ErrorState error={record.error} onRetry={() => void record.refetch()} />;
  if (!asset) return <PageLoading />;
  const money = moneyIn(asset._titles?.currency);
  const live = asset.docstatus === 1 && !asset.disposal;
  const open = (kind: DialogKind, task?: string) => setDialog({ kind, task });
  const props = { asset, onClose: () => setDialog(null), onDone: () => { setDialog(null); refresh(); } };
  const status = asset.status ?? "";

  return (
    <div className="ast-page ast-asset">
      <div className="crumbs">
        <Link to="/">Assets</Link>
        <Icon name="chevronRight" size={12} />
      </div>
      <header className="ast-asset-head">
        <div className="ast-asset-title">
          <h1 className="page-title">{asset.asset_name ?? asset.number ?? "Asset"}</h1>
          <p className="page-sub">{[asset.number, asset._titles?.asset_category, asset.asset_quantity && asset.asset_quantity > 1 ? `${asset.asset_quantity} units` : null].filter(Boolean).join(" · ")}</p>
        </div>
        <div className="page-actions">
          {status && <span className={`badge ${STATUS_BADGE[status] ?? "badge-blue"}`}>{status}</span>}
          {live && rights.move && (
            <Menu label="Move" primary>
              <DropdownMenuItem onSelect={() => open("transfer")}>Move to another place…</DropdownMenuItem>
              <DropdownMenuItem onSelect={() => open("handout")}>Hand out to someone…</DropdownMenuItem>
              {asset.custodian && <DropdownMenuItem onSelect={() => open("takeback")}>Take back from {asset.custodian}…</DropdownMenuItem>}
            </Menu>
          )}
          {live && (rights.log || rights.fault) && (
            <Menu label="Maintain">
              {rights.log && <DropdownMenuItem onSelect={() => open("service")}>Log maintenance done…</DropdownMenuItem>}
              {rights.fault && <DropdownMenuItem onSelect={() => open("fault")}>Report a fault…</DropdownMenuItem>}
            </Menu>
          )}
          {live && rights.revalue && (
            <Button onClick={() => open("revalue")}>Revalue</Button>
          )}
          {live && rights.dispose && (
            <Menu label="Dispose">
              <DropdownMenuItem onSelect={() => open("sell")}>Sell…</DropdownMenuItem>
              <DropdownMenuSeparator />
              <DropdownMenuItem onSelect={() => open("scrap")}>Scrap…</DropdownMenuItem>
            </Menu>
          )}
          <Link className="icon-btn" to={`/e/Asset/${id}`} title="Open the full form" aria-label="Open the full form">
            <Icon name="expand" size={15} />
          </Link>
        </div>
      </header>
      {asset.docstatus === 0 && (
        <Alert
          tone="warning"
          action={
            <Link className="btn btn-primary btn-md" to={`/e/Asset/${id}`}>
              Review and submit
            </Link>
          }
        >
          This asset is a draft. It starts depreciating, and can be moved or maintained, once it is submitted.
        </Alert>
      )}
      {asset.docstatus === 2 && <Alert tone="error">This asset was cancelled. Its depreciation was reversed.</Alert>}
      {asset.disposal && (
        <Alert tone="info" action={<Link to={`/e/AssetDisposal/${asset.disposal}`}>Open the disposal</Link>}>
          {status === "Sold" ? "Sold" : "Scrapped"} on {dayText(asset.disposal_date)}.
        </Alert>
      )}
      {asset.verification_result === "Not Found" && live && (
        <Alert tone="error" action={rights.verify ? <Button size="sm" onClick={() => open("verify")}>Verify again</Button> : undefined}>
          It was not found at the last check, on {dayText(asset.last_verified_date)}.
        </Alert>
      )}
      <div className="ast-body">
        <Facts asset={asset} today={today} rights={rights} canEdit={asset.docstatus === 1 && can(meta.permissions, "write")} onDetails={() => open("details")} onVerify={() => open("verify")} />
        <section className="ast-main" aria-label="Asset details">
          <Numbers asset={asset} money={money} />
          <Tabs id="asset" tabs={TABS} value={tab} onChange={choose} />
          <TabPanel id="asset" tab={tab}>
            {tab === "depreciation" && <Depreciation asset={asset} today={today} money={money} />}
            {tab === "history" && <Story asset={asset} money={money} />}
            {tab === "maintenance" && <Maintenance asset={asset} today={today} can={{ log: live && rights.log, plan: live && rights.plan, fault: live && rights.fault }} onLog={(task) => open("service", task)} onFault={() => open("fault")} />}
            {tab === "files" && <CollabPanel entity="Asset" id={id} canWrite={can(meta.permissions, "write")} />}
          </TabPanel>
        </section>
      </div>
      {dialog && (dialog.kind === "transfer" || dialog.kind === "handout" || dialog.kind === "takeback") && <MoveDialog {...props} mode={dialog.kind} />}
      {dialog?.kind === "revalue" && <RevalueDialog {...props} />}
      {dialog && (dialog.kind === "sell" || dialog.kind === "scrap") && <DisposeDialog {...props} mode={dialog.kind} />}
      {dialog?.kind === "service" && <ServiceDialog {...props} task={dialog.task} />}
      {dialog?.kind === "fault" && <FaultDialog {...props} />}
      {dialog?.kind === "verify" && <VerifyDialog {...props} />}
      {dialog?.kind === "details" && <DetailsDialog {...props} />}
    </div>
  );
}

export function AssetRoute() {
  const { id = "" } = useParams();
  const meta = useEntityMeta("Asset");
  if (meta.error) return <ErrorState error={meta.error} onRetry={() => void meta.refetch()} />;
  if (!meta.data) return <PageLoading />;
  return <AssetPage key={id} meta={meta.data} id={id} />;
}
