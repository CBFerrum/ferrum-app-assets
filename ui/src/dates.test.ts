import { describe, expect, it } from "vitest";
import { addDays, addMonths, daysBetween, monthEnd, monthOf, monthStart, phrase, relative, urgency } from "./dates";

describe("days", () => {
  it("counts days across months, years and clock changes", () => {
    expect(daysBetween("2026-10-31", "2026-11-01")).toBe(1);
    expect(daysBetween("2024-02-28", "2024-03-01")).toBe(2);
    expect(daysBetween("2026-03-28", "2026-03-30")).toBe(2);
    expect(daysBetween("2026-12-31", "2026-01-01")).toBe(-364);
    expect(addDays("2026-12-30", 3)).toBe("2027-01-02");
    expect(addDays("2026-03-01", -1)).toBe("2026-02-28");
  });

  it("finds the edges of a month", () => {
    expect(monthStart("2026-11-17")).toBe("2026-11-01");
    expect(monthStart("2026-11-17", 2)).toBe("2027-01-01");
    expect(monthStart("2026-01-05", -1)).toBe("2025-12-01");
    expect(monthEnd("2024-02-10")).toBe("2024-02-29");
    expect(monthEnd("2026-12-01")).toBe("2026-12-31");
    expect(monthOf("2026-11-17")).toBe("2026-11");
  });

  it("moves by months and keeps to the month's end", () => {
    expect(addMonths("2026-11-17", 3)).toBe("2027-02-17");
    expect(addMonths("2026-01-31", 1)).toBe("2026-02-28");
    expect(addMonths("2024-01-31", 1)).toBe("2024-02-29");
    expect(addMonths("2026-03-31", -1)).toBe("2026-02-28");
    expect(addMonths("2026-11-17", 12)).toBe("2027-11-17");
  });

  it("says how close a day is, in words", () => {
    const format = (d: string) => `on ${d}`;
    const today = "2026-11-10";
    expect(relative("2026-11-10", today, format)).toBe("today");
    expect(relative("2026-11-11", today, format)).toBe("tomorrow");
    expect(relative("2026-11-09", today, format)).toBe("yesterday");
    expect(relative("2026-11-07", today, format)).toBe("3 days ago");
    expect(relative("2026-11-22", today, format)).toBe("in 12 days");
    expect(relative("2027-03-01", today, format)).toBe("on 2027-03-01");
    expect(relative("2026-01-01", today, format)).toBe("on 2026-01-01");
  });

  it("says how pressing a day is", () => {
    const today = "2026-11-10";
    expect(urgency("2026-11-09", today)).toBe("late");
    expect(urgency("2026-11-10", today)).toBe("soon");
    expect(urgency("2026-12-10", today)).toBe("soon");
    expect(urgency("2026-12-11", today)).toBe("later");
    expect(urgency(null, today)).toBe("none");
    expect(urgency("soon", today)).toBe("none");
  });
});

describe("a date against a noun", () => {
  const format = (d: string) => `on ${d}`;
  const today = "2026-11-10";
  it("reads as a chip would say it", () => {
    expect(phrase("2026-11-15", today, "back", format)).toBe("back in 5 days");
    expect(phrase("2026-11-10", today, "due", format)).toBe("due today");
    expect(phrase("2026-11-11", today, "due", format)).toBe("due tomorrow");
    expect(phrase("2026-11-07", today, "back", format)).toBe("3 days overdue");
    expect(phrase("2026-11-09", today, "due", format)).toBe("1 day overdue");
    expect(phrase("2025-03-01", today, "back", format)).toBe("overdue since on 2025-03-01");
    expect(phrase("2026-11-22", today, "ends", format)).toBe("ends in 12 days");
    expect(phrase("2026-11-07", today, "ends", format)).toBe("ended 3 days ago");
    expect(phrase("2030-01-01", today, "ends", format)).toBe("ends on 2030-01-01");
  });
});
