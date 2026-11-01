// Amounts arrive as decimal strings and are added here as whole numbers of millionths (a Ferrum
// currency has at most six decimal places), so a total reads exactly as the server would say it,
// never off by a float's rounding. Pure, so it is tested without a browser.

const SCALE = 6;
const ONE = 10n ** BigInt(SCALE);
const DECIMAL = /^([-+]?)(\d+)(?:\.(\d+))?$/;

/** An amount as millionths; null when it is not one (blank, text, a float that wrote an exponent). */
export function units(value: unknown): bigint | null {
  const text = typeof value === "string" ? value.trim() : typeof value === "number" && Number.isFinite(value) ? String(value) : "";
  const m = DECIMAL.exec(text);
  if (!m) return null;
  const fraction = (m[3] ?? "").slice(0, SCALE).padEnd(SCALE, "0");
  const n = BigInt(m[2]) * ONE + BigInt(fraction);
  return m[1] === "-" ? -n : n;
}

/** Millionths back to a decimal string with only the places it needs: "12.5", "-3". */
export function decimal(n: bigint): string {
  const abs = n < 0n ? -n : n;
  const fraction = (abs % ONE).toString().padStart(SCALE, "0").replace(/0+$/, "");
  return `${n < 0n ? "-" : ""}${abs / ONE}${fraction ? `.${fraction}` : ""}`;
}

/** The exact sum of some amounts; a blank or anything that is not an amount counts as nothing. */
export function sum(values: Iterable<unknown>): string {
  let total = 0n;
  for (const v of values) total += units(v) ?? 0n;
  return decimal(total);
}

/** `a - b`, exactly. */
export function minus(a: unknown, b: unknown): string {
  return decimal((units(a) ?? 0n) - (units(b) ?? 0n));
}

/** -1, 0 or 1; what is not an amount sorts as zero. */
export function compare(a: unknown, b: unknown): number {
  const [x, y] = [units(a) ?? 0n, units(b) ?? 0n];
  return x < y ? -1 : x > y ? 1 : 0;
}

/** An amount as a float, for drawing only (positions and lengths); never for showing or adding. */
export function plot(value: unknown): number {
  return Number(units(value) ?? 0n) / Number(ONE);
}

/** How much of `whole` is `part`, from 0 to 1 (0 when there is no whole); for bars and rings. */
export function share(part: unknown, whole: unknown): number {
  const w = plot(whole);
  return w > 0 ? Math.min(1, Math.max(0, plot(part) / w)) : 0;
}
