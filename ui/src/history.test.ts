import { describe, expect, it } from "vitest";
import { history, type Story } from "./history";

const f = { money: (a: string) => `$${a}`, date: (d: string) => d };
const story = (more: Partial<Story> = {}): Story => ({
  asset: { id: "a", purchase_date: "2023-01-15", available_for_use_date: "2023-01-15", gross_purchase_amount: "1200", purchase_invoice: "pi-1" },
  moves: [],
  adjustments: [],
  repairs: [],
  logs: [],
  checks: [],
  disposal: null,
  ...more,
});

describe("an asset's history", () => {
  it("starts with how it came, and says so differently for what was already owned", () => {
    expect(history(story(), f)).toEqual([{ key: "bought", at: "2023-01-15", kind: "bought", title: "Bought for $1200", open: { entity: "PurchaseInvoice", id: "pi-1" } }]);
    const old = history(story({ asset: { id: "a", purchase_date: "2019-01-01", gross_purchase_amount: "10000", available_for_use_date: "2019-02-01" } }), f);
    expect(old.map((e) => e.title)).toEqual(["Put in use", "Registered at $10000, already owned"]);
  });

  it("tells moves, hand-outs and take-backs in the words people use", () => {
    const moves = [
      { transaction_date: "2024-02-01", movement: "AMV-1", movement_id: "m1", purpose: "Transfer", from_location: "Floor 1", to_location: "Floor 2", from_custodian: "bob@acme.test", to_custodian: "bob@acme.test" },
      { transaction_date: "2024-03-01", movement: "AMV-2", movement_id: "m2", purpose: "Issue", from_location: "Floor 2", to_location: "Floor 2", to_custodian: "alice@acme.test", due_back: "2024-04-01" },
      { transaction_date: "2024-04-10", movement: "AMV-3", movement_id: "m3", purpose: "Receipt", from_location: "Floor 2", to_location: "Store", from_custodian: "alice@acme.test" },
      { transaction_date: "2024-05-01", movement: "AMV-4", movement_id: "m4", purpose: "Transfer", from_location: "Store", to_location: "Floor 1", from_custodian: null, to_custodian: "carol@acme.test" },
    ];
    const told = history(story({ moves }), f).filter((e) => e.key.startsWith("m-"));
    expect(told.map((e) => [e.title, e.detail])).toEqual([
      ["Moved to Floor 1", "from Store · now with carol@acme.test"],
      ["Taken back from alice@acme.test", "into Store"],
      ["Handed to alice@acme.test", "due back 2024-04-01"],
      ["Moved to Floor 2", "from Floor 1"],
    ]);
    expect(told[0].to).toBe("e/AssetMovement/m4");
  });

  it("weaves every kind of record in by its date, the latest first", () => {
    const told = history(
      story({
        moves: [{ transaction_date: "2024-02-01", movement: "AMV-1", movement_id: "m1", purpose: "Transfer", to_location: "Floor 2" }],
        adjustments: [{ id: "v1", adjustment_date: "2024-06-30", current_asset_value: "615.11", new_asset_value: "400", difference_amount: "-215.11", remarks: "Screen damage" }],
        repairs: [
          { id: "r1", failure_date: "2024-01-10", repair_status: "Completed", completion_date: "2024-01-15", repair_cost: "600", capitalize_repair_cost: true, description: "Screen cracked" },
          { id: "r2", failure_date: "2024-09-01", repair_status: "Pending", description: "No power" },
          { id: "r3", failure_date: "2024-09-02", repair_status: "Cancelled" },
        ],
        logs: [
          { id: "l1", task_name: "Clean the fans", maintenance_status: "Completed", completion_date: "2024-03-03", actions_performed: "Replaced the paste" },
          { id: "l2", task_name: "Calibrate", maintenance_status: "Planned", due_date: "2024-10-01" },
          { id: "l3", task_name: "Battery test", maintenance_status: "In Progress", due_date: "2024-08-01" },
        ],
        checks: [
          { id: "c1", verified_on: "2024-05-01", result: "Found", note: "On the desk", verified_by: "manager@acme.test", _titles: { seen_at: "Floor 2" } },
          { id: "c2", verified_on: "2024-07-01", result: "Not Found" },
        ],
        disposal: { id: "d1", disposal_type: "Sell", disposal_date: "2025-03-31", proceeds: "300", gain_or_loss: "-100.5" },
      }),
      f,
    );
    expect(told.map((e) => [e.at, e.title])).toEqual([
      ["2025-03-31", "Sold for $300"],
      ["2024-09-01", "Fault reported: No power"],
      ["2024-08-01", "Battery test under way"],
      ["2024-07-01", "Not found"],
      ["2024-06-30", "Written down to $400"],
      ["2024-05-01", "Verified at Floor 2"],
      ["2024-03-03", "Clean the fans done"],
      ["2024-02-01", "Moved to Floor 2"],
      ["2024-01-15", "Repaired: Screen cracked"],
      ["2023-01-15", "Bought for $1200"],
    ]);
    const by = Object.fromEntries(told.map((e) => [e.title, e]));
    expect(by["Sold for $300"].detail).toBe("loss of $100.5");
    expect(by["Written down to $400"].detail).toBe("from $615.11 · Screen damage");
    expect(by["Verified at Floor 2"].detail).toBe("On the desk · by manager@acme.test");
    expect(by["Repaired: Screen cracked"].detail).toBe("fault reported 2024-01-10 · cost $600, added to its value");
    expect(by["Not found"].tone).toBe("bad");
  });

  it("puts what shares a day in the order it happens in a life", () => {
    const told = history(story({ asset: { id: "a" }, checks: [{ id: "c1", verified_on: "2024-05-01", result: "Found" }], adjustments: [{ id: "v1", adjustment_date: "2024-05-01", new_asset_value: "5", difference_amount: "1" }] }), f);
    expect(told.map((e) => e.kind)).toEqual(["value", "verified"]);
    expect(told[0].title).toBe("Written up to $5");
  });
});
