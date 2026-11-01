// Walking a room to see what is there: what a scan means, what is still to find, and what was found
// where it should not be. The screen asks the server about every scan and shows what it says; this
// is only the bookkeeping around that. Pure, so it is tested without a browser.

import { addMonths } from "./dates";

export interface Candidate {
  id: string;
  number?: string | null;
  asset_name?: string | null;
  serial_number?: string | null;
  company?: string | null;
  /** Where the register says it is. */
  location?: string | null;
  custodian?: string | null;
  _titles?: Record<string, string>;
}

/** `saving`: asked, not answered yet. `verified`: seen where the register says it is. `elsewhere`:
 *  seen here, recorded somewhere else. `missing`: looked for and not found. `again`: already seen
 *  on this walk. `refused`: the server would not take it, and says why. `unknown`: no asset has
 *  that tag or serial number. */
export type Status = "saving" | "verified" | "elsewhere" | "missing" | "again" | "refused" | "unknown";

export interface Scan {
  key: string;
  code: string;
  asset: Candidate | null;
  status: Status;
  reason?: string;
  /** The verification the server made, which undo takes back. */
  check?: string;
  /** The asset was moved here from the place the register had. */
  moved?: boolean;
}

/** What a scanner or a keyboard sent, cleaned: no stray spaces or line ends. */
export const clean = (raw: string) => raw.replace(/\s+/g, " ").trim();

/** The asset a code names among those at hand: by its tag (number) or serial number, in any case.
 *  More than one match means the code does not say which. */
export function lookup(code: string, pool: readonly Candidate[]): { asset?: Candidate; ambiguous?: boolean } {
  const want = clean(code).toLowerCase();
  if (!want) return {};
  const hits = pool.filter((a) => a.number?.toLowerCase() === want || a.serial_number?.trim().toLowerCase() === want);
  const unique = [...new Map(hits.map((a) => [a.id, a])).values()];
  return unique.length === 1 ? { asset: unique[0] } : unique.length > 1 ? { ambiguous: true } : {};
}

/** Whether this walk has already dealt with the asset: seen it, or looked for it and not found it. */
export const alreadySeen = (scans: readonly Scan[], asset: Candidate) => scans.some((s) => s.asset?.id === asset.id && (s.status === "verified" || s.status === "elsewhere" || s.status === "missing"));

export interface Progress {
  /** Expected here and seen. */
  done: number;
  total: number;
  /** Expected here, not seen and not given up on yet. */
  waiting: Candidate[];
  /** Expected here and looked for, and not found. */
  missing: number;
  /** Seen here, recorded elsewhere (and not moved yet). */
  elsewhere: Scan[];
  /** Scans the server refused or that named nothing. */
  problems: Scan[];
}

export function progress(expected: readonly Candidate[], scans: readonly Scan[]): Progress {
  const ids = (status: Status) => new Set(scans.filter((s) => s.status === status && s.asset).map((s) => s.asset!.id));
  const [here, lost] = [ids("verified"), ids("missing")];
  return {
    done: expected.filter((a) => here.has(a.id)).length,
    total: expected.length,
    waiting: expected.filter((a) => !here.has(a.id) && !lost.has(a.id)),
    missing: expected.filter((a) => lost.has(a.id)).length,
    elsewhere: scans.filter((s) => s.status === "elsewhere"),
    problems: scans.filter((s) => s.status === "refused" || s.status === "unknown"),
  };
}

/** When to check again: a category's own schedule, or a number of months from `today`. */
export type NextCheck = "category" | "3" | "6" | "12";

export function nextCheck(choice: NextCheck, today: string): string | undefined {
  return choice === "category" ? undefined : addMonths(today, Number(choice));
}

/** One scan replaced by its next state. */
export const replace = (scans: readonly Scan[], key: string, change: Partial<Scan>): Scan[] => scans.map((s) => (s.key === key ? { ...s, ...change } : s));

/** The scans without one. */
export const without = (scans: readonly Scan[], key: string): Scan[] => scans.filter((s) => s.key !== key);

/** The latest scan that can still be taken back. */
export const last = (scans: readonly Scan[]): Scan | undefined => scans.find((s) => !!s.check);
