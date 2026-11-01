import { describe, expect, it } from "vitest";
import { compare, decimal, minus, share, sum, units } from "./money";

describe("amounts", () => {
  it("adds exactly, where floats would not", () => {
    expect(0.1 + 0.2).not.toBe(0.3);
    expect(sum(["0.1", "0.2"])).toBe("0.3");
    expect(sum(["1200", "-215.11", "0.005"])).toBe("984.895");
    expect(sum(["33.33", "33.33", "33.34"])).toBe("100");
  });

  it("counts a blank or a word as nothing", () => {
    expect(sum([null, undefined, "", "n/a", "12.50"])).toBe("12.5");
    expect(sum([])).toBe("0");
  });

  it("reads and writes the same number", () => {
    expect(decimal(units("-3.50")!)).toBe("-3.5");
    expect(decimal(units("0.000001")!)).toBe("0.000001");
    expect(decimal(units("+7")!)).toBe("7");
    expect(units("1e21")).toBeNull();
    expect(units(42)).toBe(42_000_000n);
  });

  it("subtracts and compares", () => {
    expect(minus("100", "34.5")).toBe("65.5");
    expect(minus("1", "2.25")).toBe("-1.25");
    expect([compare("2", "10"), compare("10", "2"), compare("1.0", "1")]).toEqual([-1, 1, 0]);
  });

  it("says how much of a whole a part is, for a bar", () => {
    expect(share("25", "100")).toBe(0.25);
    expect(share("150", "100")).toBe(1);
    expect(share("-5", "100")).toBe(0);
    expect(share("5", "0")).toBe(0);
  });
});
