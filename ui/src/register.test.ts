import { describe, expect, it } from "vitest";
import { apiFilters, bars, depreciated, line, listPath, monthLabel, months, specs, stuck, tallest } from "./register";
import type { PlanRow } from "./types";

describe("the lists the figures open", () => {
  it("writes a list's filters as its address holds them, and as the API takes them", () => {
    const query = { status: "Fully Depreciated", expected_return_date: "..2026-10-31", warranty_expiry_date: "2026-11-01..2026-12-31" };
    expect(listPath(query)).toBe("/e/Asset?f.status=Fully+Depreciated&f.expected_return_date=..2026-10-31&f.warranty_expiry_date=2026-11-01..2026-12-31");
    expect(apiFilters(query)).toEqual([
      ["status", "=", "Fully Depreciated"],
      ["expected_return_date", "<=", "2026-10-31"],
      ["warranty_expiry_date", ">=", "2026-11-01"],
      ["warranty_expiry_date", "<=", "2026-12-31"],
    ]);
    expect(listPath({}, "AssetMovement")).toBe("/e/AssetMovement");
    expect(apiFilters({ next_verification_date: "2026-01-01.." })).toEqual([["next_verification_date", ">=", "2026-01-01"]]);
  });
});

describe("where the money is", () => {
  const rows = [
    { asset_category: "Laptops", assets: 3, cost: "3600", accumulated_depreciation: "1000", net_book_value: "2600" },
    { asset_category: "Machinery", assets: 1, cost: "50000", accumulated_depreciation: "7499.48", net_book_value: "42500.52" },
    { asset_category: "Furniture", assets: 10, cost: "1500", accumulated_depreciation: "1500", net_book_value: "0" },
    { asset_category: null, assets: 1, cost: "10", accumulated_depreciation: "0", net_book_value: "10" },
  ];

  it("lists the biggest first, with its length and its share of the whole", () => {
    const list = bars(rows, "asset_category", new Map([["Laptops", "cat-1"]]));
    expect(list.map((b) => [b.name, b.value, b.percent, b.id])).toEqual([
      ["Machinery", "42500.52", 94, undefined],
      ["Laptops", "2600", 6, "cat-1"],
      ["No category", "10", 0, undefined],
      ["Furniture", "0", 0, undefined],
    ]);
    expect(list[0].fraction).toBe(1);
    expect(list[1].fraction).toBeCloseTo(0.0612, 3);
    expect(list[3].fraction).toBe(0);
  });

  it("sums the small ones as a group once there are too many, but not for one", () => {
    const many = Array.from({ length: 5 }, (_, i) => ({ location: `Room ${i}`, assets: 1, cost: "10", accumulated_depreciation: "0", net_book_value: String(10 - i) }));
    const three = bars(many, "location", new Map(), 3);
    expect(three.map((b) => b.name)).toEqual(["Room 0", "Room 1", "Room 2", "2 more"]);
    expect(three[3]).toMatchObject({ assets: 2, value: "13", cost: "20" });
    expect(bars(many.slice(0, 4), "location", new Map(), 3).map((b) => b.name)).toEqual(["Room 0", "Room 1", "Room 2", "Room 3"]);
  });

  it("says how far through its cost the base is depreciated", () => {
    expect(depreciated("1000", "3600")).toBe(28);
    expect(depreciated("0", "0")).toBe(0);
  });
});

describe("depreciation by month", () => {
  const row = (date: string, status: string, planned: string, more: Partial<PlanRow> = {}): PlanRow => ({
    asset_id: "a",
    schedule_date: date,
    planned_amount: planned,
    booked_amount: status.startsWith("Booked") ? planned : "0",
    status,
    ...more,
  });

  it("sums what was booked and what is still to book, exactly, in the months asked for", () => {
    const list = months(
      [
        row("2026-10-31", "Booked", "100.10"),
        row("2026-11-30", "Booked", "33.33"),
        row("2026-11-30", "Booked at Disposal", "0.20"),
        row("2026-11-30", "Due", "10.10", { posting_error: "The period is closed" }),
        row("2026-12-31", "Planned", "33.34"),
        row("2026-12-31", "Planned", "0.01"),
        row("2027-12-31", "Planned", "999"),
      ],
      "2026-11-17",
      3,
    );
    expect(list.map((m) => m.start)).toEqual(["2026-11-01", "2026-12-01", "2027-01-01"]);
    expect(list.map((m) => [m.booked, m.waiting, m.total])).toEqual([
      ["33.53", "10.1", "43.63"],
      ["0", "33.35", "33.35"],
      ["0", "0", "0"],
    ]);
    expect(list.map((m) => [m.late, m.failed])).toEqual([[true, true], [false, false], [false, false]]);
    expect(tallest(list)).toBe("43.63");
  });

  it("labels a month, with the year when it is not this one", () => {
    expect(monthLabel("2026-11-01", "2026-11-17", "en")).toBe("Nov");
    expect(monthLabel("2027-01-01", "2026-11-17", "en")).toBe("Jan 27");
  });
});

describe("what needs a person", () => {
  const today = "2026-11-10";

  it("asks for exactly the assets its list will show", () => {
    const [missing, overdue, , verify, warranty] = specs(today);
    expect(listPath(missing.query)).toBe("/e/Asset?f.verification_result=Not+Found");
    expect(apiFilters(overdue.query)).toEqual([["expected_return_date", "<=", "2026-11-09"]]);
    expect(apiFilters(verify.query)).toEqual([["next_verification_date", "<=", "2026-11-10"]]);
    expect(apiFilters(warranty.query)).toEqual([["warranty_expiry_date", ">=", "2026-11-10"], ["warranty_expiry_date", "<=", "2027-01-09"]]);
    expect(warranty.title).toBe("Warranty ends within 60 days");
  });

  it("tells each asset by what is true of it there", () => {
    const [missing, overdue, broken, verify, warranty, old] = specs(today);
    const a = { id: "a", number: "AST-1", asset_name: "Laptop", custodian: "bob@acme.test", expected_return_date: "2026-11-07", last_verified_date: "2026-05-01", next_verification_date: "2026-11-10", warranty_expiry_date: "2026-11-22", _titles: { location: "Floor 2" } };
    const f = (d: string) => `on ${d}`;
    expect(line(overdue, a, today, f)).toEqual({ id: "a", title: "AST-1 · Laptop", detail: "bob@acme.test · due back 3 days ago" });
    expect(line(missing, a, today, f).detail).toBe("last checked on 2026-05-01");
    expect(line(missing, { id: "b" }, today, f)).toEqual({ id: "b", title: "Asset", detail: "last checked never" });
    expect(line(verify, a, today, f).detail).toBe("due today");
    expect(line(warranty, a, today, f).detail).toBe("ends in 12 days");
    expect(line(broken, a, today, f).detail).toBe("Floor 2");
    expect(line(old, a, today, f).detail).toBe("Floor 2 · bob@acme.test");
  });

  it("finds the assets whose depreciation is due and not booked, oldest first, with why", () => {
    const rows: PlanRow[] = [
      { asset_id: "a", asset: "AST-1", asset_name: "Laptop", schedule_date: "2026-10-31", planned_amount: "10", booked_amount: "0", status: "Due", posting_error: "The period is closed" },
      { asset_id: "a", asset: "AST-1", asset_name: "Laptop", schedule_date: "2026-09-30", planned_amount: "10", booked_amount: "0", status: "Due" },
      { asset_id: "b", asset: "AST-2", asset_name: "Chair", schedule_date: "2026-08-31", planned_amount: "5", booked_amount: "0", status: "Due" },
      { asset_id: "c", asset: "AST-3", schedule_date: "2026-12-31", planned_amount: "5", booked_amount: "0", status: "Planned" },
      { asset_id: "d", asset: "AST-4", schedule_date: "2026-07-31", planned_amount: "5", booked_amount: "5", status: "Booked" },
    ];
    expect(stuck(rows)).toEqual([
      { id: "b", title: "AST-2 · Chair", since: "2026-08-31", reason: "" },
      { id: "a", title: "AST-1 · Laptop", since: "2026-09-30", reason: "" },
    ]);
    expect(stuck([rows[1], rows[0]]).map((s) => s.since)).toEqual(["2026-09-30"]);
  });
});
