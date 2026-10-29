# Ferrum Assets

A native app for [Ferrum](https://github.com/CBFerrum/Ferrum), the open-source, AI-first ERP:
fixed assets from purchase to disposal, with every figure exact to the cent and every posting in
the platform ledger.

- **Asset categories** with depreciation defaults (straight line, written down value, double
  declining balance or a manual schedule; useful life, frequency, rate, pro-rata or full first
  period) and accounts per company: fixed asset, accumulated depreciation, depreciation expense,
  capital work in progress, gain/loss on disposal. Companies get the standard asset accounts
  automatically (or from **Create Asset Accounts** on the company).
- **Assets** (submittable) with location (a tree), custodian (who sees the asset), cost center,
  optional finance book, purchase and available-for-use dates, salvage value, and existing assets
  with opening accumulated depreciation. The depreciation schedule is generated with exact decimal
  maths: a part first period is depreciated pro rata by days and the rest taken in an extra period
  at the end, the last row absorbs the rounding, and nothing is ever depreciated below salvage.
- **Depreciation posting**: the periodic upkeep books every row that falls due (up to the
  workspace's today) as a submitted depreciation journal entry, one row per transaction, so it is
  idempotent and resumable; a row that cannot be booked (a closed period) keeps the reason and
  Asset Managers are notified once.
- **Capitalization from purchase invoices**: lines of fixed asset items are booked to the
  category's fixed asset account (or capital work in progress) and drafts are made on submit, one
  asset per unit or one grouped asset; CWIP moves to the fixed asset account when the asset is
  submitted. The invoice cannot be cancelled while its assets are submitted.
- **Asset movements**: transfers between locations, issues to custodians and receipts back, with
  the custody history kept.
- **Value adjustments** (impairment or revaluation) and **repairs** (capitalized into the value or
  left expensed): the depreciation due by their date is booked first, the change is posted and the
  rest of the schedule is re-planned. A pending repair puts the asset out of order.
- **Maintenance plans**: tasks with a periodicity, an assignee and a next due date; due tasks open
  a maintenance log and notify the assignee; a log in progress puts the asset in maintenance.
- **Disposal**: scrapping writes off the net book value; selling creates a sales invoice whose line
  is credited to the fixed asset account and whose submission books the gain or loss. Depreciation
  up to the disposal date (pro rata for the running period) is booked first; cancelling restores
  the asset and its schedule.
- **Reports**: fixed asset register (cost, accumulated depreciation and net book value as of a
  date, by asset, category or location — it ties to the ledger), depreciation schedule (planned vs
  booked), movement history, maintenance due, and disposal gains and losses.
- **Roles**: Asset Manager (everything), Asset User (the assets in their custody, their repairs and
  maintenance work), and read access for the Accounts Manager.

## Install

Workspace administrators install it from **Apps** in Ferrum; it ships in the official Ferrum
server image. Self-hosters building their own image enable the `assets` feature of the server. It
needs the platform's accounting, buying and selling packages. Everything it adds is also available
to AI agents over Ferrum's MCP server.

## Develop

Clone it next to the platform and point the platform dependencies at your checkout (this file is
git-ignored):

```toml
# .cargo/config.toml
[patch."https://github.com/CBFerrum/Ferrum"]
ferrum-kernel = { path = "../ferrum/crates/kernel" }
ferrum-meta = { path = "../ferrum/crates/meta" }
ferrum-domain = { path = "../ferrum/crates/domain" }
ferrum-calc = { path = "../ferrum/crates/calc" }
```

The tests need PostgreSQL with an ordinary (non-superuser) role — row-level security must apply:

```bash
createuser ferrum --pwprompt && createdb -O ferrum ferrum_assets_test
```

```bash
cargo test
```

`FERRUM_ASSETS_TEST_DATABASE_URL` overrides the database URL. The depreciation maths
(`src/schedule.rs`) is pure and unit-tested; `tests/assets_it.rs` drives a company's assets end to
end through the kernel API and checks every figure against the ledger. The app's metadata lives in
`packages/` and is embedded at build time; `app.yaml` is its manifest (see Ferrum's
`docs/plan/apps.md`).

## License

AGPL-3.0-only, like the platform.
