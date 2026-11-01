// How amounts and days read on these screens. Amounts are exact decimal strings (see money.ts); the
// browser's number formatter writes them with the places and the symbol their currency has.

import { formatDate, formatDecimal } from "@ferrum/client";

/** Money as a person writes it: in the company's currency (an ISO code such as INR), exactly. */
export function moneyIn(code: string | null | undefined, locale?: string): (amount: unknown) => string {
  let nf: Intl.NumberFormat | null = null;
  try {
    nf = code ? new Intl.NumberFormat(locale, { style: "currency", currency: code }) : null;
  } catch {
    nf = null; // a code the browser does not know
  }
  return (amount) => {
    if (amount === null || amount === undefined || amount === "") return "";
    const text = String(amount);
    try {
      return nf ? nf.format(text as Intl.StringNumericLiteral) : formatDecimal(text, 2, locale);
    } catch {
      return text;
    }
  };
}

/** A figure for a chart's axis or a bar: 12400 reads 12.4K. For comparing, never for adding. */
export function compact(n: number, locale?: string): string {
  return n.toLocaleString(locale, { notation: "compact", maximumFractionDigits: 1 });
}

/** `2026-11-17` as "17 Nov 2026" in the reader's language. */
export const dayText = (iso: string | null | undefined) => (iso ? formatDate(iso.slice(0, 10)) : "");
