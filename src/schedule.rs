//! Depreciation maths: pure and exact (decimals, no I/O), so every rule is unit-tested.
//!
//! A plan's periods ("slots") follow from the asset's settings: the first depreciation date, then
//! every `frequency` months (month ends stay month ends). With the pro-rata convention a first
//! period the asset was in use for only part of is depreciated for its share of days, and the
//! share it missed is taken in one extra period at the end. Amounts are rounded to the currency's
//! precision and the last row takes what is left, so a schedule ends exactly at the salvage value
//! and never goes below it.

use chrono::{Datelike, Duration, NaiveDate};
use rust_decimal::{Decimal, RoundingStrategy};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    StraightLine,
    WrittenDownValue,
    DoubleDecliningBalance,
    /// The user enters the schedule.
    Manual,
}

impl Method {
    pub fn parse(s: &str) -> Option<Method> {
        Some(match s {
            "Straight Line" => Method::StraightLine,
            "Written Down Value" => Method::WrittenDownValue,
            "Double Declining Balance" => Method::DoubleDecliningBalance,
            "Manual" => Method::Manual,
            _ => return None,
        })
    }
}

/// An asset's depreciation settings.
#[derive(Debug, Clone)]
pub struct Plan {
    pub method: Method,
    /// Useful life in periods (total number of depreciations).
    pub periods: u32,
    /// Months per period.
    pub frequency: u32,
    /// Written down value: percent a year.
    pub annual_rate: Decimal,
    pub pro_rata: bool,
    pub available_for_use: NaiveDate,
    /// First depreciation date (for an existing asset: the next one).
    pub start: NaiveDate,
    /// Periods depreciated before the asset was registered.
    pub opening_booked: u32,
    pub salvage: Decimal,
    pub precision: u32,
}

/// One period of a plan: its depreciation date and its share of a full period.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Slot {
    pub date: NaiveDate,
    pub weight: Decimal,
}

/// Half away from zero, at exactly `p` places (the platform's money rounding).
pub fn round(v: Decimal, p: u32) -> Decimal {
    let mut x = v.round_dp_with_strategy(p, RoundingStrategy::MidpointAwayFromZero);
    x.rescale(p);
    x
}

fn days_in_month(year: i32, month: u32) -> u32 {
    let (y, m) = if month == 12 { (year + 1, 1) } else { (year, month + 1) };
    NaiveDate::from_ymd_opt(y, m, 1).and_then(|d| d.pred_opt()).map(|d| d.day()).unwrap_or(28)
}

fn is_month_end(d: NaiveDate) -> bool {
    d.day() == days_in_month(d.year(), d.month())
}

/// `date` moved by whole months; a month end stays a month end (Jan 31 → Feb 28 → Mar 31).
pub fn add_months(date: NaiveDate, months: i32) -> NaiveDate {
    let total = date.year() * 12 + date.month0() as i32 + months;
    let (year, month) = (total.div_euclid(12), total.rem_euclid(12) as u32 + 1);
    let last = days_in_month(year, month);
    let day = if is_month_end(date) { last } else { date.day().min(last) };
    NaiveDate::from_ymd_opt(year, month, day).unwrap_or(date)
}

/// The default first depreciation date: the end of the month that closes the first period.
pub fn default_start(available_for_use: NaiveDate, frequency: u32) -> NaiveDate {
    let d = add_months(available_for_use, frequency.max(1) as i32 - 1);
    NaiveDate::from_ymd_opt(d.year(), d.month(), days_in_month(d.year(), d.month())).unwrap_or(d)
}

impl Plan {
    /// Share of the first period the asset is in use: 1 for a full period (or without pro-rata).
    pub fn first_share(&self) -> Decimal {
        if !self.pro_rata || self.opening_booked > 0 {
            return Decimal::ONE;
        }
        let full = (self.start - add_months(self.start, -(self.frequency as i32))).num_days();
        let used = (self.start - self.available_for_use).num_days() + 1;
        if full <= 0 || used >= full { Decimal::ONE } else { Decimal::from(used.max(0)) / Decimal::from(full) }
    }

    /// Every period still to depreciate, in date order.
    pub fn slots(&self) -> Vec<Slot> {
        let remaining = self.periods.saturating_sub(self.opening_booked);
        if self.method == Method::Manual || remaining == 0 {
            return vec![];
        }
        let date = |k: u32| add_months(self.start, (k * self.frequency) as i32);
        let mut out: Vec<Slot> = (0..remaining).map(|k| Slot { date: date(k), weight: Decimal::ONE }).collect();
        let share = self.first_share();
        if share < Decimal::ONE {
            out[0].weight = share;
            out.push(Slot { date: date(remaining), weight: Decimal::ONE - share });
        }
        out
    }

    /// Declining methods' rate per period: the annual rate pro rata for written down value,
    /// twice the straight-line rate for double declining balance.
    pub fn period_rate(&self) -> Decimal {
        match self.method {
            Method::WrittenDownValue => self.annual_rate / Decimal::from(100) * Decimal::from(self.frequency) / Decimal::from(12),
            Method::DoubleDecliningBalance => Decimal::from(2) / Decimal::from(self.periods.max(1)),
            _ => Decimal::ZERO,
        }
    }

    /// Depreciation for `slots` (the periods still to come) taking `value` (the net book value
    /// now) towards the salvage value. Zero amounts are left out.
    pub fn amounts(&self, slots: &[Slot], value: Decimal) -> Vec<(NaiveDate, Decimal)> {
        let amounts = match self.method {
            Method::StraightLine => spread(value - self.salvage, &slots.iter().map(|s| s.weight).collect::<Vec<_>>(), self.precision),
            Method::WrittenDownValue | Method::DoubleDecliningBalance => declining(value, self.salvage, slots, self.period_rate(), self.precision),
            Method::Manual => vec![],
        };
        slots.iter().zip(amounts).filter(|(_, a)| *a > Decimal::ZERO).map(|(s, a)| (s.date, a)).collect()
    }

    /// The day before the period that ends on `end` began: the previous depreciation date, or for
    /// the first one the day before the asset was available (pro-rata) or a full period earlier.
    pub fn period_start(&self, end: NaiveDate, previous: Option<NaiveDate>) -> NaiveDate {
        match previous {
            Some(p) => p,
            None if self.method == Method::Manual || (self.pro_rata && self.opening_booked == 0) => (self.available_for_use - Duration::days(1)).min(end),
            None => add_months(end, -(self.frequency as i32)),
        }
    }
}

/// Splits `total` by `weights`: each part rounded, the last takes what is left, and no part
/// exceeds what is left (so none is negative and the parts add up to `total` exactly).
pub fn spread(total: Decimal, weights: &[Decimal], precision: u32) -> Vec<Decimal> {
    let all: Decimal = weights.iter().sum();
    if total <= Decimal::ZERO || all <= Decimal::ZERO {
        return weights.iter().map(|_| Decimal::ZERO).collect();
    }
    let mut left = round(total, precision);
    let last = weights.len().saturating_sub(1);
    weights
        .iter()
        .enumerate()
        .map(|(i, w)| {
            let a = if i == last { left } else { round(total * *w / all, precision).min(left).max(Decimal::ZERO) };
            left -= a;
            a
        })
        .collect()
}

/// Declining balance: each period takes `rate` (times the period's share) of the value left,
/// never below the salvage value; the last period takes the value down to the salvage value.
pub fn declining(value: Decimal, salvage: Decimal, slots: &[Slot], rate: Decimal, precision: u32) -> Vec<Decimal> {
    let mut value = value;
    let last = slots.len().saturating_sub(1);
    slots
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let room = round((value - salvage).max(Decimal::ZERO), precision);
            let a = if i == last { room } else { round(value * rate * s.weight, precision).min(room) };
            value -= a;
            a
        })
        .collect()
}

/// Depreciation of a scheduled `amount` for the days of its period `(start, end]` up to `on`.
pub fn partial(amount: Decimal, start: NaiveDate, end: NaiveDate, on: NaiveDate, precision: u32) -> Decimal {
    let total = (end - start).num_days();
    if total <= 0 {
        return Decimal::ZERO;
    }
    let used = (on - start).num_days().clamp(0, total);
    round(amount * Decimal::from(used) / Decimal::from(total), precision)
}

/// Rescales a manual plan when the value it depreciates (above salvage) changes from `before` to
/// `after`: every planned amount in proportion, so the plan keeps covering the same share of the
/// value and never reaches below salvage. `None` when no plan is left to carry an increase.
pub fn rescale(amounts: &[Decimal], before: Decimal, after: Decimal, precision: u32) -> Option<Vec<Decimal>> {
    let total: Decimal = amounts.iter().sum();
    if total.is_zero() || before <= Decimal::ZERO {
        return (after <= before).then(|| amounts.iter().map(|_| Decimal::ZERO).collect());
    }
    Some(spread(round(total * after.max(Decimal::ZERO) / before, precision), amounts, precision))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn n(s: &str) -> Decimal {
        Decimal::from_str(s).unwrap()
    }

    fn plan(method: Method, periods: u32, frequency: u32, afu: NaiveDate, start: NaiveDate) -> Plan {
        Plan {
            method,
            periods,
            frequency,
            annual_rate: Decimal::ZERO,
            pro_rata: true,
            available_for_use: afu,
            start,
            opening_booked: 0,
            salvage: Decimal::ZERO,
            precision: 2,
        }
    }

    fn amounts(p: &Plan, value: &str) -> Vec<String> {
        p.amounts(&p.slots(), n(value)).iter().map(|(_, a)| a.to_string()).collect()
    }

    #[test]
    fn month_ends_stay_month_ends() {
        assert_eq!(add_months(d(2026, 1, 31), 1), d(2026, 2, 28));
        assert_eq!(add_months(d(2026, 1, 31), 2), d(2026, 3, 31));
        assert_eq!(add_months(d(2026, 1, 30), 1), d(2026, 2, 28));
        assert_eq!(add_months(d(2026, 1, 30), 2), d(2026, 3, 30));
        assert_eq!(add_months(d(2023, 2, 28), 12), d(2024, 2, 29));
        assert_eq!(add_months(d(2024, 2, 29), 12), d(2025, 2, 28));
        assert_eq!(add_months(d(2026, 3, 31), -1), d(2026, 2, 28));
        assert_eq!(add_months(d(2026, 1, 15), -13), d(2024, 12, 15));
        assert_eq!(default_start(d(2026, 1, 15), 1), d(2026, 1, 31));
        assert_eq!(default_start(d(2026, 1, 15), 3), d(2026, 3, 31));
        assert_eq!(default_start(d(2026, 1, 15), 12), d(2026, 12, 31));
    }

    #[test]
    fn straight_line_last_row_absorbs_rounding() {
        let p = plan(Method::StraightLine, 3, 12, d(2023, 1, 1), d(2023, 12, 31));
        assert_eq!(p.first_share(), Decimal::ONE, "a full first year");
        assert_eq!(amounts(&p, "1000"), ["333.33", "333.33", "333.34"]);
        let dates: Vec<NaiveDate> = p.slots().iter().map(|s| s.date).collect();
        assert_eq!(dates, [d(2023, 12, 31), d(2024, 12, 31), d(2025, 12, 31)]);
    }

    #[test]
    fn straight_line_pro_rata_first_period() {
        // In use from Jan 15: 17 of January's 31 days, the other 14 come at the end.
        let p = plan(Method::StraightLine, 12, 1, d(2023, 1, 15), d(2023, 1, 31));
        assert_eq!(p.first_share(), n("17") / n("31"));
        let rows = p.amounts(&p.slots(), n("1200"));
        assert_eq!(rows.len(), 13);
        assert_eq!(rows[0], (d(2023, 1, 31), n("54.84")));
        assert!(rows[1..12].iter().all(|(_, a)| *a == n("100.00")));
        assert_eq!(rows[1].0, d(2023, 2, 28));
        assert_eq!(rows[12], (d(2024, 1, 31), n("45.16")));
        assert_eq!(rows.iter().map(|r| r.1).sum::<Decimal>(), n("1200"));
        let full = Plan { pro_rata: false, ..p };
        assert_eq!(full.slots().len(), 12, "full first period: no extra period");
        assert_eq!(amounts(&full, "1200")[0], "100.00");
    }

    #[test]
    fn salvage_is_never_depreciated() {
        let p = Plan { salvage: n("200"), ..plan(Method::StraightLine, 4, 12, d(2023, 1, 1), d(2023, 12, 31)) };
        assert_eq!(amounts(&p, "1000"), ["200.00", "200.00", "200.00", "200.00"]);
        assert!(amounts(&p, "200").is_empty(), "nothing above the salvage value");
    }

    #[test]
    fn existing_assets_spread_what_is_left_over_the_remaining_periods() {
        let p = Plan { opening_booked: 2, salvage: n("1000"), ..plan(Method::StraightLine, 5, 12, d(2019, 1, 1), d(2021, 12, 31)) };
        assert_eq!(p.first_share(), Decimal::ONE, "no pro-rata once periods are behind it");
        assert_eq!(amounts(&p, "6000"), ["1666.67", "1666.67", "1666.66"]);
        let all_done = Plan { opening_booked: 5, ..p };
        assert!(all_done.slots().is_empty());
    }

    #[test]
    fn written_down_value_ends_at_salvage() {
        let p = Plan { annual_rate: n("40"), salvage: n("1000"), ..plan(Method::WrittenDownValue, 5, 12, d(2020, 4, 1), d(2021, 3, 31)) };
        assert_eq!(p.period_rate(), n("0.4"));
        assert_eq!(amounts(&p, "10000"), ["4000.00", "2400.00", "1440.00", "864.00", "296.00"]);
        let quarterly = Plan { frequency: 3, ..p.clone() };
        assert_eq!(quarterly.period_rate(), n("0.1"), "a quarter of the annual rate");
    }

    #[test]
    fn written_down_value_pro_rata() {
        // Oct 1 to Mar 31: 182 of 365 days.
        let p = Plan { annual_rate: n("40"), ..plan(Method::WrittenDownValue, 3, 12, d(2020, 10, 1), d(2021, 3, 31)) };
        assert_eq!(p.first_share(), n("182") / n("365"));
        assert_eq!(amounts(&p, "10000"), ["1994.52", "3202.19", "1921.32", "2881.97"]);
        // A high rate reaches the salvage value early: nothing below it, no empty rows.
        let steep = Plan { annual_rate: n("90"), salvage: n("500"), pro_rata: false, ..p };
        assert_eq!(amounts(&steep, "1000"), ["500.00"]);
    }

    #[test]
    fn double_declining_balance() {
        let p = Plan { salvage: n("1000"), ..plan(Method::DoubleDecliningBalance, 4, 12, d(2020, 1, 1), d(2020, 12, 31)) };
        assert_eq!(p.period_rate(), n("0.5"));
        assert_eq!(amounts(&p, "10000"), ["5000.00", "2500.00", "1250.00", "250.00"]);
    }

    #[test]
    fn manual_plans_are_not_generated() {
        let p = plan(Method::Manual, 3, 12, d(2023, 1, 1), d(2023, 12, 31));
        assert!(p.slots().is_empty());
    }

    #[test]
    fn pro_rata_days_of_a_period() {
        assert_eq!(partial(n("100"), d(2025, 2, 28), d(2025, 3, 31), d(2025, 3, 15), 2), n("48.39"));
        assert_eq!(partial(n("100"), d(2025, 2, 28), d(2025, 3, 31), d(2025, 3, 31), 2), n("100.00"));
        assert_eq!(partial(n("100"), d(2025, 2, 28), d(2025, 3, 31), d(2025, 2, 28), 2), n("0.00"));
        let p = plan(Method::StraightLine, 12, 1, d(2023, 1, 15), d(2023, 1, 31));
        assert_eq!(p.period_start(d(2023, 1, 31), None), d(2023, 1, 14), "the first period starts on the available-for-use date");
        assert_eq!(p.period_start(d(2023, 2, 28), Some(d(2023, 1, 31))), d(2023, 1, 31));
        let full = Plan { pro_rata: false, ..p };
        assert_eq!(full.period_start(d(2023, 1, 31), None), d(2022, 12, 31));
    }

    #[test]
    fn spreading_is_exact_and_never_negative() {
        assert_eq!(spread(n("1000.00"), &[Decimal::ONE; 3], 2), [n("333.33"), n("333.33"), n("333.34")]);
        let tiny = spread(n("1"), &[Decimal::ONE; 60], 2);
        assert_eq!(tiny.iter().sum::<Decimal>(), n("1"));
        assert!(tiny.iter().all(|a| *a >= Decimal::ZERO));
        // A plan of 600 over a depreciable 1000: the value rises 10%, so does the plan.
        assert_eq!(rescale(&[n("100"), n("200"), n("300")], n("1000"), n("1100"), 2), Some(vec![n("110.00"), n("220.00"), n("330.00")]));
        assert_eq!(rescale(&[n("100"), n("200")], n("1000"), n("0"), 2), Some(vec![n("0.00"), n("0.00")]), "down to salvage: nothing left to plan");
        assert_eq!(rescale(&[], n("1000"), n("1200"), 2), None, "no plan to carry an increase");
        assert_eq!(rescale(&[], n("1000"), n("800"), 2), Some(vec![]));
    }
}
