// The screens drawn from what the server would answer: each is rendered to markup with its queries
// already answered (nothing is fetched), so a screen that cannot draw what the server really sends
// fails here and not in front of a person. The answers are shaped like the server's (ids for links,
// money as strings, `_titles` for the words of links).

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderToString } from "react-dom/server";
import { MemoryRouter, Route, Routes } from "react-router";
import { todayIso } from "@ferrum/client";
import { describe, expect, it } from "vitest";
import { DetailsDialog, DisposeDialog, FaultDialog, MoveDialog, RevalueDialog, ServiceDialog, VerifyDialog } from "./Actions";
import { AssetRoute } from "./Asset";
import { Home } from "./Home";
import { Verify } from "./Verify";
import { monthEnd, monthStart, addMonths } from "./dates";
import { specs } from "./register";

const COMPANY = "co-1";
const ASSET = "as-1";
const today = todayIso();

const everything = { read: "all", write: "all", create: "all", delete: "all", submit: "all", cancel: "all", export: "all", import: "all" };
const meta = (name: string, permissions: object = everything) => ({
  name,
  label: name,
  label_plural: `${name}s`,
  kind: "document",
  submittable: true,
  list: { columns: [] },
  form: [],
  search_fields: [],
  rules: [],
  actions: [],
  fields: [],
  children: {},
  permissions,
});
const me = { tenant: "acme", user: { id: "u1", email: "manager@acme.test", full_name: "Mina", is_admin: false, roles: ["Asset Manager"] }, default_company: COMPANY, scopes: {}, fiscal_year_start_month: 4, packages: ["assets"] };

type Seed = [unknown[], unknown][];

function client(seed: Seed) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false, staleTime: Infinity, gcTime: Infinity } } });
  for (const [key, data] of seed) qc.setQueryData(key, data);
  return qc;
}

function draw(ui: React.ReactElement, seed: Seed, at = "/") {
  return renderToString(
    <QueryClientProvider client={client(seed)}>
      <MemoryRouter initialEntries={[at]}>{ui}</MemoryRouter>
    </QueryClientProvider>,
  ).replaceAll("<!-- -->", "");
}

const nothing = { read: "none", write: "none", create: "none", delete: "none", submit: "none", cancel: "none", export: "none", import: "none" };

const common: Seed = [
  [["me"], me],
  [["meta", "Asset"], meta("Asset")],
  [["meta", "AssetVerification"], meta("AssetVerification")],
  [["meta", "AssetMovement"], meta("AssetMovement")],
  [["meta", "AssetValueAdjustment"], meta("AssetValueAdjustment")],
  [["meta", "AssetDisposal"], meta("AssetDisposal")],
  [["meta", "AssetRepair"], meta("AssetRepair")],
  [["meta", "AssetMaintenanceLog"], meta("AssetMaintenanceLog")],
  [["meta", "AssetMaintenance"], meta("AssetMaintenance")],
];

describe("the overview", () => {
  const from = monthStart(today);
  const row = (id: string, number: string, name: string, more: object = {}) => ({ id, number, asset_name: name, _titles: { location: "HQ Floor 1" }, ...more });
  const attention = Object.fromEntries(
    specs(today).map((s) => [s.id, s.id === "overdue" ? { rows: [row("a1", "AST-1", "Laptop", { custodian: "bob@acme.test", expected_return_date: "2020-01-01" })], total: 5 } : { rows: [], total: 0 }]),
  );
  const loaded: Seed = [
    ...common,
    [["assets", "companies"], [{ id: COMPANY, company_name: "Acme" }]],
    [
      ["assets", "register", COMPANY, today, "Asset Category"],
      {
        rows: [
          { asset_category: "Laptops", assets: 3, cost: "3600", accumulated_depreciation: "1000", net_book_value: "2600" },
          { asset_category: "Machinery", assets: 1, cost: "50000", accumulated_depreciation: "7499.48", net_book_value: "42500.52" },
        ],
        totals: { cost: "53600", accumulated_depreciation: "8499.48", net_book_value: "45100.52" },
        meta: { company: "Acme Industries", currency: "INR" },
      },
    ],
    [["assets", "names", "asset_category"], new Map([["Laptops", "cat-1"]])],
    [
      ["assets", "plan", COMPANY, from],
      {
        rows: [
          { asset_id: "a1", asset: "AST-1", schedule_date: addMonths(from, 0), planned_amount: "100", booked_amount: "100", status: "Booked" },
          { asset_id: "a1", asset: "AST-1", schedule_date: addMonths(from, 1), planned_amount: "100", booked_amount: "0", status: "Planned" },
        ],
      },
    ],
    [["assets", "maintenance", COMPANY, today], { rows: [{ asset: "AST-1", asset_id: "a1", asset_name: "Laptop", asset_maintenance: "m1", task_name: "Clean the fans", next_due_date: "2020-01-01", days_overdue: 4, status: "Overdue" }] }],
    [["assets", "unbooked", COMPANY, today], { total: 1, items: [{ id: "a1", title: "AST-1 · Laptop", since: "2026-09-30", reason: "The period is closed" }] }],
    ...specs(today).map((s): [unknown[], unknown] => [["assets", "attention", s.id, COMPANY, today], attention[s.id]]),
  ];

  it("shows what the company owns, what is due and what needs a person", () => {
    const html = draw(<Home />, loaded);
    expect(html).toContain("Net book value");
    expect(html).toContain("₹45,100.52");
    expect(html).toContain("Laptops");
    expect(html).toContain("f.asset_category=cat-1");
    expect(html).toContain("94%");
    expect(html).toContain("Depreciation could not be booked");
    expect(html).toContain("The period is closed");
    expect(html).toContain("Past their return date");
    expect(html).toContain("and 4 more");
    expect(html).toContain("Maintenance overdue");
    expect(html).toContain("Clean the fans");
    expect(html).toContain("/a/a1?tab=maintenance");
    expect(html).toContain("Verify a room");
    expect(html).toContain(`from_date=${from}`);
    expect(html).toContain(`to_date=${monthEnd(from)}`);
  });

  it("says so when nothing needs attention, and when there are no assets", () => {
    const calm = loaded.map(([k, v]): [unknown[], unknown] => (k[1] === "attention" ? [k, { rows: [], total: 0 }] : k[1] === "unbooked" ? [k, { total: 0, items: [] }] : k[1] === "maintenance" ? [k, { rows: [] }] : [k, v]));
    expect(draw(<Home />, calm)).toContain("Nothing needs attention");
    const none = loaded.map(([k, v]): [unknown[], unknown] => (k[1] === "register" ? [k, { rows: [], totals: { cost: "0", accumulated_depreciation: "0", net_book_value: "0" }, meta: { currency: "INR" } }] : [k, v]));
    const html = draw(<Home />, none);
    expect(html).toContain("No assets yet");
    expect(html).toContain("Add an asset you own");
  });

  it("holds the page and offers no verifying to someone who may not", () => {
    const custodian: Seed = loaded.map(([k, v]) => (k[0] === "meta" && k[1] !== "Asset" ? [k, meta(String(k[1]), { ...nothing, read: "own" })] : [k, v]));
    const html = draw(<Home />, custodian);
    expect(html).not.toContain("Verify a room");
  });

  it("draws the first moments before anything has been answered", () => {
    expect(draw(<Home />, [])).toContain("Overview");
  });
});

describe("an asset's page", () => {
  const schedule = [
    { id: "s1", schedule_date: "2024-01-31", depreciation_amount: "100", booked: true, journal_entry: "je-1", accumulated_depreciation: "100" },
    { id: "s2", schedule_date: "2024-02-29", depreciation_amount: "100", booked: true, journal_entry: "je-2", accumulated_depreciation: "200" },
    { id: "s3", schedule_date: "2999-03-31", depreciation_amount: "300", booked: false, accumulated_depreciation: "500" },
    { id: "s4", schedule_date: "2999-04-30", depreciation_amount: "300", booked: false, accumulated_depreciation: "800" },
  ];
  const asset = {
    id: ASSET,
    version: 7,
    docstatus: 1,
    number: "AST-2024-00001",
    asset_name: "Dell XPS 13",
    company: COMPANY,
    location: "loc-1",
    custodian: "bob@acme.test",
    expected_return_date: "2020-04-01",
    status: "Partially Depreciated",
    serial_number: "SN-2",
    manufacturer: "Dell",
    model_number: "XPS 13",
    warranty_expiry_date: "2999-01-14",
    purchase_date: "2024-01-01",
    available_for_use_date: "2024-01-01",
    gross_purchase_amount: "1000",
    total_asset_cost: "800",
    value_adjustments: "-200",
    accumulated_depreciation: "200",
    value_after_depreciation: "600",
    calculate_depreciation: true,
    last_verified_date: "2024-05-01",
    next_verification_date: "2020-11-01",
    verification_result: "Found",
    schedule,
    _titles: { location: "HQ Floor 1", asset_category: "Laptops", currency: "INR" },
  };
  const page: Seed = [
    ...common,
    [["peek", "Asset", ASSET], asset],
    [["assets", "photo-of", ASSET], null],
    [["assets", "value-events", ASSET, 7], [{ date: "2024-03-05", amount: "-200", kind: "adjustment", label: "Revalued" }]],
    [
      ["assets", "story", ASSET, 7],
      {
        moves: [{ transaction_date: "2024-03-05", movement: "AMV-1", movement_id: "m1", purpose: "Issue", from_location: "HQ Floor 1", to_location: "HQ Floor 1", to_custodian: "bob@acme.test", due_back: "2020-04-01" }],
        adjustments: [{ id: "v1", adjustment_date: "2024-03-05", current_asset_value: "800", new_asset_value: "600", difference_amount: "-200" }],
        repairs: [{ id: "r1", failure_date: "2024-04-01", repair_status: "Pending", description: "No power" }],
        logs: [],
        checks: [{ id: "c1", verified_on: "2024-05-01", result: "Found", _titles: { seen_at: "HQ Floor 1" } }],
        disposal: null,
      },
    ],
    [["assets", "plan-of", ASSET], { id: "plan-1", tasks: [{ id: "t1", task_name: "Clean the fans", periodicity: "Monthly", last_completion_date: "2024-01-01", next_due_date: "2020-02-01" }] }],
    [["assets", "open-work", ASSET], { logs: [{ id: "l1", task: "t1", task_name: "Clean the fans", due_date: "2020-02-01", maintenance_status: "Planned" }], faults: [{ id: "r1", failure_date: "2024-04-01", repair_status: "Pending", description: "No power" }] }],
  ];
  const at = (tab = "") => `/a/${ASSET}${tab ? `?tab=${tab}` : ""}`;
  const route = (
    <Routes>
      <Route path="/a/:id" element={<AssetRoute />} />
    </Routes>
  );

  it("shows where it is, who has it, what it is worth and what can be done", () => {
    const html = draw(route, page, at());
    expect(html).toContain("Dell XPS 13");
    expect(html).toContain("₹600.00");
    expect(html).toContain("HQ Floor 1");
    expect(html).toContain("bob@acme.test");
    expect(html).toContain("overdue since Apr 1, 2020");
    expect(html).toContain("SN-2");
    expect(html).toContain("Move");
    expect(html).toContain("Maintain");
    expect(html).toContain("Revalue");
    expect(html).toContain("Dispose");
    expect(html).toContain("Verify now");
    expect(html).toContain("Book value over time");
    expect(html).toContain("Verify now");
  });

  it("raises the alarm for depreciation that is due and not booked, with the reason", () => {
    const stuck = asset.schedule.map((r) => (r.id === "s3" ? { ...r, schedule_date: "2020-03-31", posting_error: "The period is closed" } : r));
    const html = draw(route, page.map(([k, v]) => (k[0] === "peek" ? [k, { ...asset, schedule: stuck }] : [k, v])), at());
    expect(html).toContain("Depreciation could not be booked.");
    expect(html).toContain("The period is closed");
    expect(html).toContain("Could not be booked");
  });

  it("tells its depreciation period by period, the change of value among them", () => {
    const html = draw(route, page, at());
    expect(html).toContain("Revalued");
    expect(html).toContain("Booked");
    expect(html).toContain("Planned");
    expect(html).toContain("/open/JournalEntry/je-1");
    expect(html).toContain("₹1,000.00");
  });

  it("tells its history", () => {
    const html = draw(route, page, at("history"));
    expect(html).toContain("Handed to bob@acme.test");
    expect(html).toContain("Written down to ₹600.00");
    expect(html).toContain("Fault reported: No power");
    expect(html).toContain("Verified at HQ Floor 1");
  });

  it("tells its maintenance and its faults", () => {
    const html = draw(route, page, at("maintenance"));
    expect(html).toContain("Clean the fans");
    expect(html).toContain("Faults to repair");
    expect(html).toContain("Mark done");
    expect(html).toContain("Plan");
  });

  it("offers nothing to do to a draft, and says what it needs", () => {
    const draft: Seed = page.map(([k, v]) => (k[0] === "peek" ? [k, { ...asset, docstatus: 0, status: "Draft", custodian: null, expected_return_date: null }] : [k, v]));
    const html = draw(route, draft, at());
    expect(html).toContain("This asset is a draft");
    expect(html).toContain("Review and submit");
    expect(html).not.toContain("Dispose");
  });

  it("says how a sold asset left, and offers nothing more", () => {
    const sold: Seed = page.map(([k, v]) => (k[0] === "peek" ? [k, { ...asset, status: "Sold", disposal: "d1", disposal_date: "2024-06-01" }] : [k, v]));
    const html = draw(route, sold, at());
    expect(html).toContain("Sold on");
    expect(html).not.toContain("Dispose");
  });

  it("keeps a custodian to what they may do", () => {
    const own = { ...nothing, read: "own" };
    const custodian: Seed = page.map(([k, v]) => (k[0] === "meta" ? [k, meta(String(k[1]), k[1] === "AssetRepair" ? { ...own, create: "own" } : own)] : [k, v]));
    const html = draw(route, custodian, at());
    expect(html).not.toContain(">Dispose<");
    expect(html).not.toContain(">Revalue<");
    expect(html).not.toContain(">Move<");
    expect(html).toContain("Maintain");
  });
});

describe("verifying a room", () => {
  const room = {
    rows: [
      { id: "as-1", number: "AST-1", asset_name: "Laptop", serial_number: "SN-1", location: "loc-1", company: COMPANY },
      { id: "as-2", number: "AST-2", asset_name: "Chair", location: "loc-1", company: COMPANY },
    ],
    total: 2,
  };

  it("asks where you are first", () => {
    const html = draw(<Verify />, common, "/verify");
    expect(html).toContain("Where are you?");
    expect(html).toContain("Start");
  });

  it("lists what is expected in the room, with a way to scan", () => {
    const places = [["meta", "Location"], { ...meta("Location"), title_field: "location_name" }] as [unknown[], unknown];
    const html = draw(<Verify />, [...common, places, [["assets", "expected", "loc-1"], room], [["record-title", "Location", "loc-1"], { id: "loc-1", location_name: "HQ Floor 1" }]], "/verify?place=loc-1");
    expect(html).toContain("Verifying HQ Floor 1");
    expect(html).toContain("0 of 2");
    expect(html).toContain("Scan or type a tag or serial number");
    expect(html).toContain("AST-1 · Laptop");
    expect(html).toContain("It is here");
    expect(html).toContain("Mark the rest as not found");
  });

  it("is for those who may verify", () => {
    const html = draw(<Verify />, [...common.filter(([k]) => k[1] !== "AssetVerification"), [["meta", "AssetVerification"], meta("AssetVerification", { ...nothing, read: "all" })]], "/verify");
    expect(html).toContain("Verifying is for asset managers");
  });
});

describe("what is asked of a person before the server is", () => {
  const asset = { id: ASSET, version: 3, docstatus: 1, number: "AST-1", asset_name: "Laptop", company: COMPANY, location: "loc-1", custodian: "bob@acme.test", value_after_depreciation: "600", available_for_use_date: "2024-01-01", purchase_date: "2024-01-01", serial_number: "SN-1", _titles: { currency: "INR", location: "HQ Floor 1" } };
  const props = { asset, onClose: () => {}, onDone: () => {} };
  const dialog = (ui: React.ReactElement, seed: Seed = []) => draw(ui, [...common, ...seed]);

  it("asks where to, who to and until when, in the words of what is being done", () => {
    const move = dialog(<MoveDialog {...props} mode="transfer" />);
    expect(move).toContain("Move Laptop");
    expect(move).toContain("Where to");
    const out = dialog(<MoveDialog {...props} mode="handout" />);
    expect(out).toContain("Hand out Laptop");
    expect(out).toContain("Who gets it");
    expect(out).toContain("Back by");
    const back = dialog(<MoveDialog {...props} mode="takeback" />);
    expect(back).toContain("It is with bob@acme.test");
    expect(back).toContain("Put it back in");
  });

  it("says what a revaluation starts from and what it does", () => {
    const html = dialog(<RevalueDialog {...props} />);
    expect(html).toContain("Revalue Laptop");
    expect(html).toContain("₹600.00");
    expect(html).toContain("Book the difference to");
  });

  it("explains selling and scrapping before they are done", () => {
    const sell = dialog(<DisposeDialog {...props} mode="sell" />);
    expect(sell).toContain("Sell Laptop");
    expect(sell).toContain("Customer");
    expect(sell).toContain("Make the sales invoice");
    const scrap = dialog(<DisposeDialog {...props} mode="scrap" />);
    expect(scrap).toContain("₹600.00 is written off");
    expect(scrap).toContain("Scrap it");
  });

  it("asks what is wrong, and which task was done", () => {
    expect(dialog(<FaultDialog {...props} />)).toContain("What is wrong");
    const plan = { id: "plan-1", tasks: [{ id: "t1", task_name: "Clean the fans", periodicity: "Monthly", next_due_date: "2020-02-01" }] };
    const html = dialog(<ServiceDialog {...props} task="t1" />, [[["assets", "plan-of", ASSET], plan]]);
    expect(html).toContain("Clean the fans, monthly, due Feb 1, 2020");
    expect(html).toContain("What was done");
    expect(dialog(<ServiceDialog {...props} />, [[["assets", "plan-of", ASSET], null]])).toContain("This asset has no maintenance plan yet");
  });

  it("asks what was found, and what the asset says about itself", () => {
    const check = dialog(<VerifyDialog {...props} />);
    expect(check).toContain("It is here");
    expect(check).toContain("Not found");
    const details = dialog(<DetailsDialog {...props} />);
    expect(details).toContain("Serial number");
    expect(details).toContain('value="SN-1"');
    expect(details).toContain("End of life");
  });
});
