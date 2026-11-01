import { describe, expect, it } from "vitest";
import { alreadySeen, clean, last, lookup, nextCheck, progress, replace, without, type Candidate, type Scan } from "./walk";

const a = (id: string, number: string, serial?: string): Candidate => ({ id, number, serial_number: serial, asset_name: `Asset ${id}` });
const room = [a("1", "AST-2026-00001", "SN-A"), a("2", "AST-2026-00002", " sn-b "), a("3", "AST-2026-00003", "SN-B")];

describe("what a scan names", () => {
  it("cleans what a scanner sends", () => {
    expect(clean("  AST-2026-00001 \r\n")).toBe("AST-2026-00001");
    expect(clean("AST   2026")).toBe("AST 2026");
  });

  it("finds an asset by its tag or its serial number, in any case", () => {
    expect(lookup("ast-2026-00001", room).asset?.id).toBe("1");
    expect(lookup("SN-A", room).asset?.id).toBe("1");
    expect(lookup("  AST-2026-00003\n", room).asset?.id).toBe("3");
    expect(lookup("AST-2026-00099", room)).toEqual({});
    expect(lookup("", room)).toEqual({});
  });

  it("does not guess when a serial number belongs to more than one", () => {
    expect(lookup("SN-B", room)).toEqual({ ambiguous: true });
    expect(lookup("AST-2026-00002", room).asset?.id).toBe("2");
  });
});

describe("a walk", () => {
  const scan = (key: string, asset: Candidate | null, status: Scan["status"], more: Partial<Scan> = {}): Scan => ({ key, code: asset?.number ?? "?", asset, status, ...more });

  it("counts what was seen of what was expected, and lists what is still to find", () => {
    const scans = [scan("s3", room[2], "elsewhere", { check: "c3" }), scan("s2", room[0], "verified", { check: "c2" }), scan("s1", null, "unknown")];
    const p = progress(room.slice(0, 2), scans);
    expect([p.done, p.total, p.missing]).toEqual([1, 2, 0]);
    expect(p.waiting.map((x) => x.id)).toEqual(["2"]);
    expect(p.elsewhere.map((s) => s.key)).toEqual(["s3"]);
    expect(p.problems.map((s) => s.key)).toEqual(["s1"]);
  });

  it("stops waiting for what was looked for and not found, without counting it as seen", () => {
    const p = progress(room.slice(0, 2), [scan("s1", room[1], "missing", { check: "c1" })]);
    expect([p.done, p.missing, p.waiting.map((x) => x.id)]).toEqual([0, 1, ["1"]]);
    expect(alreadySeen([scan("s1", room[1], "missing")], room[1])).toBe(true);
  });

  it("knows an asset it has seen, and not one that was refused", () => {
    expect(alreadySeen([scan("s1", room[0], "verified")], room[0])).toBe(true);
    expect(alreadySeen([scan("s1", room[0], "elsewhere")], room[0])).toBe(true);
    expect(alreadySeen([scan("s1", room[0], "refused")], room[0])).toBe(false);
    expect(alreadySeen([scan("s1", room[0], "verified")], room[1])).toBe(false);
  });

  it("changes one scan, drops one, and offers the latest that can be taken back", () => {
    const scans = [scan("s2", room[1], "verified", { check: "c2" }), scan("s1", room[0], "saving")];
    expect(replace(scans, "s1", { status: "verified", check: "c1" }).map((s) => [s.key, s.status])).toEqual([["s2", "verified"], ["s1", "verified"]]);
    expect(without(scans, "s2").map((s) => s.key)).toEqual(["s1"]);
    expect(last(scans)?.key).toBe("s2");
    expect(last([scan("s1", room[0], "refused")])).toBeUndefined();
  });

  it("works out when to check again", () => {
    expect(nextCheck("category", "2026-11-10")).toBeUndefined();
    expect(nextCheck("6", "2026-11-10")).toBe("2027-05-10");
    expect(nextCheck("12", "2026-02-28")).toBe("2027-02-28");
  });
});
