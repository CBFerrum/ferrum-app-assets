// Calendar days as `YYYY-MM-DD` text, the way the server sends them. Worked out in UTC so a daylight
// saving change never moves a day. Pure, so it is tested without a browser.

const DAY = 86_400_000;

const at = (iso: string) => {
  const [y, m, d] = iso.split("-").map(Number);
  return Date.UTC(y, m - 1, d);
};

const text = (ms: number) => new Date(ms).toISOString().slice(0, 10);

/** Whole days from `from` to `to` (negative when `to` is earlier). */
export function daysBetween(from: string, to: string): number {
  return Math.round((at(to) - at(from)) / DAY);
}

export function addDays(iso: string, days: number): string {
  return text(at(iso) + days * DAY);
}

/** `months` later (or earlier) on the same day of the month, or the month's last day when it has
 *  no such day (31 January + 1 month is the end of February). */
export function addMonths(iso: string, months: number): string {
  const day = Number(iso.slice(8, 10));
  const first = monthStart(iso, months);
  return addDays(first, Math.min(day, Number(monthEnd(first).slice(8, 10))) - 1);
}

/** The first day of the month `months` away from the one `iso` falls in. */
export function monthStart(iso: string, months = 0): string {
  const [y, m] = iso.split("-").map(Number);
  return text(Date.UTC(y, m - 1 + months, 1));
}

/** The last day of the month `iso` falls in. */
export function monthEnd(iso: string): string {
  return addDays(monthStart(iso, 1), -1);
}

export const monthOf = (iso: string) => iso.slice(0, 7);

export const isDay = (v: unknown): v is string => typeof v === "string" && /^\d{4}-\d{2}-\d{2}/.test(v);

/** How close a day is, in words: "today", "tomorrow", "3 days ago", "in 12 days". Beyond `near` days
 *  it is the date itself, which `format` writes. */
export function relative(iso: string, today: string, format: (iso: string) => string, near = 45): string {
  const n = daysBetween(today, iso);
  if (n === 0) return "today";
  if (n === 1) return "tomorrow";
  if (n === -1) return "yesterday";
  if (Math.abs(n) > near) return format(iso);
  return n > 0 ? `in ${n} days` : `${-n} days ago`;
}

/** How pressing a day is: past, within `soon` days, or further off (or no day at all). */
export function urgency(iso: string | null | undefined, today: string, soon = 30): "late" | "soon" | "later" | "none" {
  if (!iso || !isDay(iso)) return "none";
  const n = daysBetween(today, iso.slice(0, 10));
  return n < 0 ? "late" : n <= soon ? "soon" : "later";
}

/** A date told against a noun, for a chip: "back in 5 days", "due today", "3 days overdue",
 *  "overdue since 1 Mar 2025", "ends in 12 days", "ended 3 days ago". */
export function phrase(iso: string, today: string, noun: "back" | "due" | "ends", format: (iso: string) => string, near = 45): string {
  const n = daysBetween(today, iso);
  if (n >= 0) return `${noun} ${relative(iso, today, format, near)}`;
  if (noun === "ends") return `ended ${relative(iso, today, format, near)}`;
  return -n > near ? `overdue since ${format(iso)}` : `${-n} ${-n === 1 ? "day" : "days"} overdue`;
}
