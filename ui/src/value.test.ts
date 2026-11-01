import { describe, expect, it } from "vitest";
import { valueSeries } from "./value";

const row = (id: string, date: string, amount: string, more: object = {}) => ({ id, schedule_date: date, depreciation_amount: amount, ...more });

// Cost 1000. January and February depreciate 100 each; on 5 March the asset is written down by 200
// (to 600), and the two periods left are replanned at 300 each.
const asset = {
  gross_purchase_amount: "1000",
  opening_accumulated_depreciation: "0",
  available_for_use_date: "2024-01-01",
  purchase_date: "2024-01-01",
  value_after_depreciation: "600",
  schedule: [
    row("1", "2024-01-31", "100", { booked: true, journal_entry: "je-1" }),
    row("2", "2024-02-29", "100", { booked: true, journal_entry: "je-2" }),
    row("3", "2024-03-31", "300"),
    row("4", "2024-04-30", "300"),
  ],
};
const writeDown = { date: "2024-03-05", amount: "-200", kind: "adjustment" as const, label: "Revalued" };

describe("an asset's value over time", () => {
  it("walks from the cost through every change to nothing", () => {
    const { points, exact } = valueSeries(asset, [writeDown], "2024-03-10");
    expect(points.map((p) => p.value)).toEqual(["1000", "900", "800", "600", "300", "0"]);
    expect(points.map((p) => p.kind)).toEqual(["start", "booked", "booked", "adjustment", "planned", "planned"]);
    expect(points.map((p) => p.booked)).toEqual([true, true, true, true, false, false]);
    expect(points[1].entry).toBe("je-1");
    expect(points[3].change).toBe("-200");
    expect(exact).toBe(true);
  });

  it("ends where the server says the asset stands, whatever it does not know", () => {
    const { points, exact } = valueSeries(asset, [], "2024-03-10");
    // Without the write-down the early part cannot be told; the present and the future are still exact.
    expect(points.map((p) => p.value).slice(-3)).toEqual(["600", "300", "0"]);
    expect(points[points.length - 3]).toMatchObject({ kind: "booked", date: "2024-02-29" });
    expect(exact).toBe(false);
  });

  it("tells a period that is due from one that could not be booked", () => {
    const due = valueSeries(asset, [writeDown], "2024-04-02").points;
    expect(due.slice(4).map((p) => p.kind)).toEqual(["due", "planned"]);
    const stuck = { ...asset, schedule: asset.schedule.map((r) => (r.id === "3" ? { ...r, posting_error: "The period is closed" } : r)) };
    const failed = valueSeries(stuck, [writeDown], "2024-04-02").points;
    expect(failed[4]).toMatchObject({ kind: "failed", error: "The period is closed", booked: false });
  });

  it("puts a change of value after the depreciation booked the same day", () => {
    const repair = { date: "2024-02-29", amount: "50", kind: "repair" as const, label: "Repair" };
    const booked = { ...asset, value_after_depreciation: "850", schedule: asset.schedule.slice(0, 2) };
    const { points } = valueSeries(booked, [repair], "2024-03-01");
    expect(points.map((p) => [p.kind, p.value])).toEqual([
      ["start", "1000"],
      ["booked", "900"],
      ["booked", "800"],
      ["repair", "850"],
    ]);
  });

  it("takes a sold asset out of the books at its disposal", () => {
    const sold = { ...asset, value_after_depreciation: "800", schedule: asset.schedule.slice(0, 2), disposal: "d-1", disposal_date: "2024-03-15", status: "Sold" };
    const { points } = valueSeries(sold, [], "2024-04-01");
    expect(points[points.length - 1]).toMatchObject({ kind: "disposal", value: "0", change: "-800", label: "Sold", date: "2024-03-15" });
  });

  it("shows an asset that does not depreciate as one flat value", () => {
    const land = { gross_purchase_amount: "5000", value_after_depreciation: "5000", purchase_date: "2020-01-01", schedule: [] };
    const { points, exact } = valueSeries(land, [], "2024-04-01");
    expect(points.map((p) => p.value)).toEqual(["5000"]);
    expect(exact).toBe(true);
  });

  it("counts the depreciation booked before it was registered", () => {
    const old = { gross_purchase_amount: "1000", opening_accumulated_depreciation: "400", value_after_depreciation: "500", purchase_date: "2020-01-01", schedule: [row("1", "2024-01-31", "100", { booked: true })] };
    const { points, exact } = valueSeries(old, [], "2024-04-01");
    expect(points.map((p) => p.value)).toEqual(["600", "500"]);
    expect(exact).toBe(true);
  });
});
