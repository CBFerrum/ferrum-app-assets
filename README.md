<div align="center">

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/assets/banner-dark.svg">
  <img alt="Ferrum Assets: what the company owns, where it is and what it is worth" src="docs/assets/banner-light.svg" width="100%">
</picture>

<br>

[![Version](https://img.shields.io/badge/version-0.3.18-475569?style=flat-square)](CHANGELOG.md)
[![Licence: AGPL-3.0](https://img.shields.io/badge/licence-AGPL--3.0-blue?style=flat-square)](LICENSE)
[![Ferrum platform](https://img.shields.io/badge/runs%20on-Ferrum-2f5bea?style=flat-square)](https://github.com/CBFerrum/Ferrum)
![Rust](https://img.shields.io/badge/Rust-server-b7410e?style=flat-square&logo=rust)
![PostgreSQL](https://img.shields.io/badge/PostgreSQL-data-336791?style=flat-square&logo=postgresql&logoColor=white)
![React](https://img.shields.io/badge/React-interface-0ea5e9?style=flat-square&logo=react&logoColor=white)
![TypeScript](https://img.shields.io/badge/TypeScript-interface-3178c6?style=flat-square&logo=typescript&logoColor=white)
![Tests](https://img.shields.io/badge/tests-17%20Rust%20%C2%B7%2059%20UI-16a34a?style=flat-square)

### The fixed asset register, its depreciation and its upkeep, booked straight into your own ledger.

For finance teams who keep a fixed asset register and the people who look after the equipment. It replaces
a depreciation spreadsheet, a stand-alone fixed asset package and a simple maintenance tool, and it posts to
the same books as Ferrum ERP.

[💡 Why](#-why-ferrum-assets) · [📸 Screenshots](#-screenshots) · [✨ Features](#-features) · [📊 Compare](#-how-it-compares) · [📍 Roadmap](#-roadmap) · [🚀 Install](#-install) · [🔧 Develop](#-develop)

</div>

---

## 💡 Why Ferrum Assets

- 📒 **One ledger, no export.** Purchases, depreciation, repairs, revaluations and disposals are journal
  entries in ERP's ledger, with cost centres and finance books. There is no interface file to send to the books.
- 🧮 **Figures that cannot drift.** Schedules are worked out in exact decimals and end exactly at the salvage
  value. An asset's cost, depreciation and book value are recomputed from the documents behind them on every
  change, never added up as they go.
- 📚 **Book and tax side by side.** A finance book can depreciate an asset its own way (IFRS beside the tax
  rules), with its own schedule, value and gain or loss.
- 🚶 **Verify a room from a phone.** Walk a place, scan or tick what is there, and the register knows what was
  found, what is missing and what turned up in the wrong room.
- 🔧 **Upkeep in the same place.** Maintenance plans, repairs that use parts from stock, and, with Operations,
  meters, condition rules, calibration and lockout with a separate return to service.
- 🏠 **Yours to run.** Open source under the AGPL, in your own Ferrum server and PostgreSQL, with REST and MCP
  on every record under each person's permissions.

## 🎯 At a glance

| | |
|---|---|
| 📦 **Version** | `0.3.18`; the Equipment Maintenance connector `0.1.0` |
| 🖥️ **Kind** | Own interface at `/assets/` (overview, an asset's page, verifying a room); every other record in Ferrum's records frame |
| 🧱 **What it adds** | 2 packages; 16 entities and 7 line tables; 5 roles of its own (2 in Assets, 3 in Equipment Maintenance) and read access for ERP's Accounts Manager; 5 reports; 2 menus; fields on Company, Item, Account, purchase and sales invoice lines and journal entries |
| 🔗 **Depends on** | Ferrum platform `>=0.3.2, <0.4` and ERP (0.6.0 or later); the connector also needs Operations |
| 🤝 **Works with** | ERP (accounts, purchase and sales invoices, stock, finance books, cost centres); Operations (devices and their readings) |
| 🗣️ **Languages** | English, Arabic and Hindi; pages mirror right to left |
| 🧪 **Tests** | 17 Rust tests against a real PostgreSQL, 59 interface tests |
| 📄 **Licence** | AGPL-3.0-only |

## 📸 Screenshots

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/assets/screens/assets-home-dark.webp">
  <img alt="Assets overview: net book value, cost, depreciation by month and what needs attention" src="docs/assets/screens/assets-home-light.webp" width="100%">
</picture>
<p align="center"><b>🏠 Overview</b><br><sub>Net book value, cost and depreciation as of any date, what the company owns by category or place, depreciation month by month, maintenance coming up and what needs a person.</sub></p>

<table>
<tr>
<td width="50%" valign="top">
<picture><source media="(prefers-color-scheme: dark)" srcset="docs/assets/screens/assets-asset-list-dark.webp"><img alt="Assets list" src="docs/assets/screens/assets-asset-list-light.webp"></picture>
<p><b>🗂️ Assets</b><br><sub>Browse the company's assets with their category, location, status and net book value.</sub></p>
</td>
<td width="50%" valign="top">
<picture><source media="(prefers-color-scheme: dark)" srcset="docs/assets/screens/assets-asset-detail-dark.webp"><img alt="An asset" src="docs/assets/screens/assets-asset-detail-light.webp"></picture>
<p><b>🏷️ An asset</b><br><sub>Read and edit one asset, its depreciation settings and schedule, with its history and related records.</sub></p>
</td>
</tr>
<tr>
<td width="50%" valign="top">
<picture><source media="(prefers-color-scheme: dark)" srcset="docs/assets/screens/assets-asset-new-dark.webp"><img alt="New asset" src="docs/assets/screens/assets-asset-new-light.webp"></picture>
<p><b>➕ New asset</b><br><sub>Register an asset: item, category, place, cost, and how it depreciates.</sub></p>
</td>
<td width="50%" valign="top">
<picture><source media="(prefers-color-scheme: dark)" srcset="docs/assets/screens/assets-verify-a-room-dark.webp"><img alt="Verify a room" src="docs/assets/screens/assets-verify-a-room-light.webp"></picture>
<p><b>🚶 Verify a room</b><br><sub>Pick a room, then scan or tick what is there; each asset is checked in the register at once.</sub></p>
</td>
</tr>
<tr>
<td width="50%" valign="top">
<picture><source media="(prefers-color-scheme: dark)" srcset="docs/assets/screens/assets-report-fixed-asset-register-dark.webp"><img alt="Fixed asset register" src="docs/assets/screens/assets-report-fixed-asset-register-light.webp"></picture>
<p><b>📒 Fixed asset register</b><br><sub>Cost, accumulated depreciation and net book value as of a date, by asset, category or location.</sub></p>
</td>
<td width="50%" valign="top">
<picture><source media="(prefers-color-scheme: dark)" srcset="docs/assets/screens/assets-report-asset-maintenance-due-dark.webp"><img alt="Asset maintenance due" src="docs/assets/screens/assets-report-asset-maintenance-due-light.webp"></picture>
<p><b>🔧 Maintenance due</b><br><sub>Maintenance tasks due by a date and a little after, overdue first.</sub></p>
</td>
</tr>
<tr>
<td width="50%" valign="top">
<picture><source media="(prefers-color-scheme: dark)" srcset="docs/assets/screens/assets-asset-category-list-dark.webp"><img alt="Asset categories" src="docs/assets/screens/assets-asset-category-list-light.webp"></picture>
<p><b>🗃️ Asset categories</b><br><sub>Depreciation defaults and accounts shared by similar assets.</sub></p>
</td>
<td width="50%" valign="top">
<picture><source media="(prefers-color-scheme: dark)" srcset="docs/assets/screens/assets-location-list-dark.webp"><img alt="Locations" src="docs/assets/screens/assets-location-list-light.webp"></picture>
<p><b>📍 Locations</b><br><sub>Sites, buildings and rooms as a tree.</sub></p>
</td>
</tr>
</table>

<p align="center">
<picture><source media="(prefers-color-scheme: dark)" srcset="docs/assets/screens/assets-home-phone-dark.webp"><img alt="Assets overview on a phone" src="docs/assets/screens/assets-home-phone-light.webp" width="260"></picture>
&nbsp;&nbsp;
<picture><source media="(prefers-color-scheme: dark)" srcset="docs/assets/screens/assets-asset-list-phone-dark.webp"><img alt="Assets list on a phone" src="docs/assets/screens/assets-asset-list-phone-light.webp" width="260"></picture>
</p>
<p align="center"><sub>📱 The overview and the asset list on a phone.</sub></p>

> [!TIP]
> The screenshots follow your GitHub theme: switch between light and dark to see both.

## ✨ Features

### 🗃️ Categories, locations and set-up

- **Asset categories** carry the defaults similar assets share: depreciation method, useful life in periods,
  months per period, rate for written down value, start convention, how often assets are verified, capital
  work in progress, whether a purchase line of many units makes one grouped asset, and the accounts per company.
- **Accounts per company** for each category (fixed asset, accumulated depreciation, depreciation expense,
  capital work in progress, gain or loss on disposal), falling back to the company's defaults.
- **Create Asset Accounts** on the company makes the standard asset accounts it lacks and links them; it can
  be pressed again safely. A default depreciation cost centre comes with them.
- **Locations** as a tree (site, building, room); groups hold other locations, assets sit in rooms.
- **Items** marked Is Fixed Asset with their category are bought as assets and never kept as stock.

### 🏷️ Assets

- **An asset** has its item, category, company, location, custodian (the person who has it and sees it), cost
  centre, quantity, serial number, manufacturer, model, warranty end and end of life. Serial and warranty
  details stay editable after submission (Edit Details), and the list finds an asset by serial number.
- **Numbered** `AST-{YYYY}-{seq:5}` by purchase date.
- **Statuses** Draft, Submitted, Partially Depreciated, Fully Depreciated, In Maintenance, Out of Order, Sold,
  Scrapped and Cancelled, worked out from the asset's documents.
- **Existing assets** come in with their opening accumulated depreciation and the periods already behind them.
- **Land and other assets that do not depreciate** switch depreciation off.
- **Split Asset** takes some units of a grouped asset (ten chairs on one line) into an asset of their own, with
  their share of cost, salvage value, opening depreciation and every schedule row in every book.

### 🧾 Capitalisation from purchases

- A purchase invoice line marked **Is Fixed Asset** is booked to its category's fixed asset account, or to
  capital work in progress when the category says so, and submitting the invoice drafts the assets: one per
  unit, or one grouped asset, carrying the line's amount exactly.
- Submitting the asset moves its cost out of capital work in progress.
- Cancelling the invoice removes the drafts, and is refused while one of its assets is submitted.

### 🧮 Depreciation

- **Methods**: straight line, written down value, double declining balance, and manual (you enter the schedule).
- **Conventions**: pro-rata by days (the part of the first period missed is taken in one extra period at the end)
  or full first period.
- **Exact schedules**: decimals only, month ends stay month ends, the last row takes what rounding leaves, and a
  schedule never goes below the salvage value.
- **Booked by itself**: the upkeep books every row that falls due, oldest first, one row per transaction, so an
  interrupted round leaves whole rows booked and a second run finds nothing to do. A row that cannot be booked
  (a closed period, a disabled account) keeps its reason, and the Asset Managers are told once.
- **Finance books**: an asset lists the books that depreciate it their own way (method, life, frequency, rate,
  salvage, start date, and opening figures for an existing asset). Each gets its own schedule; the common
  depreciation is taken out of that book and its own is booked instead.
- **Every entry is a journal entry** of its book, linked to its asset, reversed only by cancelling the asset or
  its disposal, or corrected by a value adjustment.

### 💹 Value changes and repairs

- **Value adjustments** (impairment or revaluation) book the depreciation due by their date, post the change to
  the account you choose, and spread what remains over the rest of the asset's life; one can change a single
  finance book alone.
- **Repairs**: a pending repair puts the asset Out of Order; completing it either capitalises the cost into the
  asset or leaves it where it was booked. A capitalised repair can add whole periods to the asset's life.
- **Parts from stock**: a repair lists the parts it used; they leave stock in one material issue on its
  completion date, their value part of the repair's cost. Cancelling puts them back.
- A repair can name the purchase invoice that billed it.

### 💰 Sale and scrap

- **Scrap**: submitting the disposal writes the asset off.
- **Sell**: Create Sales Invoice makes the invoice whose line sells the asset; submitting the invoice submits the
  disposal, books the depreciation due by the sale date and the gain or loss.
- **Each finance book** clears its own difference into its gain or loss, and the disposal records each book's net
  book value and gain.
- **Cancelling** the invoice or the disposal reverses its entries and puts the schedule back.

### 🚚 Movements and custody

- **Asset movements**: Transfer to another location, Issue to a custodian (with a date it is due back), Receipt back
  into a location. The submitted movements are the custody history.
- A movement is cancelled only while no later movement has moved its assets again.
- **Due back** is worked out from the movements that stand, and a sold or scrapped asset is due nothing.

### 🚶 Physical verification

- **Asset verification** records that someone saw an asset, or looked and could not find it.
- **Verify every (months)** on a category sets when each asset is next due; the asset carries when it was last
  verified, what was found and when the next check is.
- **Verify a room**: pick a place, scan or type tags and serial numbers (or tick things off), and each is checked
  in the register at once. What should be there and was not seen is listed, what turns up in the wrong room is
  moved with a tap, and the rest can be marked not found.

### 🔧 Maintenance

- **Maintenance plans** per asset with tasks (preventive maintenance, calibration or inspection), a periodicity
  from daily to every three years, an assignee, a start and an end.
- **Due tasks** get a planned maintenance log and their assignee a notification, once per due date. A task's next
  date counts from its start or its last completion, whichever is later.
- **Maintenance logs**: while in progress the asset is In Maintenance; submit one completed or cancelled.

### 🏭 Equipment Maintenance connector (with Operations)

For equipment that is also a device in Ferrum's Operations app. Run against simulated equipment only.

- **Meters** for running hours, cycles or distance follow their device's newest good reading; service falls due by
  use, not the calendar.
- **Condition rules**: a value above or below its limit for a sustained number of minutes opens a maintenance
  request with the evidence. Missing data is never a condition.
- **Maintenance requests** from a person, a meter or a condition, with priority, failure code, root cause and
  work done: Open, In Progress, Repaired, Closed, Rejected.
- **Failure codes** by category (mechanical, electrical, software, process, operator).
- **Equipment lockout**: while locked, nothing can be sent to the device. Finishing the work does not end it;
  a **return to service** does, by someone with that authority who did not lock it out. A lockout here is a record
  and a software interlock on dispatch; physical isolation stays the site's procedure.
- **Calibration records** with certificate, date, valid until and result (Pass, Adjusted, Fail).

### 🖥️ Its own screens

| Screen | What you do there |
|---|---|
| 🏠 Overview | Net book value, cost and depreciation as of any date; what is owned by category or place; depreciation month by month (booked, to book, overdue, could not be booked); maintenance in the next 30 days; what needs a person |
| 🏷️ Asset page | Where it is, who has it, its value over time as a chart, every period of its schedule, its maintenance and its whole story on one timeline, with move, hand out, take back, maintain, report a fault, revalue, sell, scrap and verify one step away |
| 🚶 Verify | Walk a room and confirm what is there |

Every figure opens the list or report behind it, and every list respects the person's access, so a custodian
sees only what they hold.

### 📊 Reports

- **Fixed Asset Register**: cost, accumulated depreciation and net book value as of a date, by asset, category or
  location, in any finance book.
- **Asset Depreciation Schedule**: every period planned against booked, with what is overdue.
- **Asset Movement History**: where each asset went, who held it and when it was due back.
- **Asset Maintenance Due**: tasks due by a date, overdue first.
- **Asset Disposals: Gain and Loss**.

### 🔐 Roles, people and languages

- **Asset Manager** registers, depreciates, moves, revalues, repairs, maintains and disposes of assets; **Asset
  User** sees the assets in their custody and records their repairs and maintenance; ERP's **Accounts Manager** reads
  and exports the register.
- **Maintenance Technician**, **Maintenance Planner** and **Return to Service Authority** in the connector; the last
  is separate on purpose.
- **Personal data**: a person's request reaches the assets in their custody, the maintenance logs assigned to them
  and the verifications they made, and an erasure takes their name off them. When someone leaves, their assets
  and maintenance tasks can be handed over to another person.
- **The assistant** knows how assets, schedules, movements and lockouts work in these pages and suggests questions
  to start with.
- **English, Arabic and Hindi**, pages mirrored right to left; a workspace can use its own words.

## 🧭 How it works

### 🔁 An asset's life

```mermaid
flowchart LR
    PI["Purchase invoice"] --> CW["Capital work in progress"]
    CW --> A["Asset submitted"]
    PI --> A
    A --> S["Depreciation schedule"]
    S -->|each period| JE["Journal entry"]
    A --> MV["Movements and verification"]
    A --> VA["Value adjustment or repair"]
    VA --> S
    A --> D["Sale or scrap"]
    D --> GL["Gain or loss booked"]
```

An asset is drafted from a purchase invoice line (or registered by hand), and submitting it puts it on the books.
The upkeep books its schedule period by period. Anything that changes its value first books the depreciation due
by its date, then plans the rest again, and a sale or scrap closes it out with its gain or loss.

### 🧩 Inside Ferrum

```mermaid
flowchart TD
    UI["Assets screens at /assets/"] --> API["Ferrum REST, MCP and records frame"]
    API --> AS["Assets engine"]
    AS --> LED["ERP ledger and finance books"]
    AS --> INV["ERP purchase and sales invoices"]
    AS --> STK["ERP stock: parts for repairs"]
    CON["Equipment Maintenance connector"] --> AS
    CON --> OPS["Operations: devices, readings, dispatch"]
    LED --> DB[("PostgreSQL")]
```

Assets builds on ERP's engine for every posting, so its entries obey the same period locks, finance books and
cost centres as the rest of the books. The connector is offered only where both Assets and Operations are
installed.

## 🧱 Data model

| Entity | What it is | Key fields or states |
|---|---|---|
| Asset Category | Defaults shared by similar assets | method, life, frequency, rate, convention, verify every, CWIP, group assets, accounts per company |
| Location | A place in a tree | parent, group |
| Asset | One asset or a group of identical ones | Draft, Submitted, Partially Depreciated, Fully Depreciated, In Maintenance, Out of Order, Sold, Scrapped, Cancelled |
| Asset Movement | Custody and place changes | Transfer, Issue, Receipt |
| Asset Verification | A physical check | Found, Not Found |
| Asset Value Adjustment | Impairment or revaluation | new value, finance book |
| Asset Repair | A repair and its parts | Pending, Completed, Cancelled; capitalised or not |
| Asset Maintenance | An asset's maintenance plan | tasks with type, periodicity, assignee |
| Asset Maintenance Log | One occurrence of a task | Planned, In Progress, Completed, Cancelled |
| Asset Disposal | Sale or scrap | Sell, Scrap; gain or loss per book |
| Maintenance Request | A reported or detected fault (connector) | Open, In Progress, Repaired, Closed, Rejected |
| Equipment Lockout | Equipment out of service (connector) | Locked, Returned |
| Asset Meter | A use counter from a device (connector) | reading, service every, next service at |
| Condition Rule | A limit on a device's metric (connector) | above, below, sustained minutes |
| Calibration Record | A calibration (connector) | Pass, Adjusted, Fail; valid until |
| Failure Code | Why something failed (connector) | category |

| Role | What it can do |
|---|---|
| Asset Manager | Everything in the register, from purchase to disposal |
| Asset User | Sees the assets in their custody; records repairs and maintenance |
| Accounts Manager (ERP's) | Reads every asset record and exports the register, value changes and disposals |
| Maintenance Technician | Works on equipment, locks it out, records what was done; cannot return it to service |
| Maintenance Planner | Sets up meters, conditions and failure codes |
| Return to Service Authority | Hands equipment back to operations after maintenance |

**Scheduled work**: every minute, for each workspace with Assets, the upkeep books the depreciation that fell due
and opens maintenance tasks that came due; with the connector, it follows meters and condition rules.

## 🔌 Integrations

| With | How |
|---|---|
| 📒 ERP accounting | Depreciation, revaluation, repairs and disposals are journal entries in ERP's ledger, in its finance books and cost centres, under its period locks |
| 🛒 ERP buying | Purchase invoice lines marked Is Fixed Asset draft the assets, through capital work in progress when the category asks |
| 🧾 ERP selling | A disposal's sales invoice sells the asset and books the gain or loss |
| 📦 ERP stock | A repair's parts leave stock in one material issue |
| 🤖 Operations | Devices, their readings and dispatch, for meters, condition rules and lockouts (Equipment Maintenance connector) |
| 🔔 Notifications | Assignees of due maintenance tasks; Asset Managers when depreciation cannot be booked |
| 🔌 REST and MCP | Every record, under each person's permissions |

Nothing outside Ferrum is needed: no keys, accounts or outside services.

## 📊 How it compares

Compared with SAP S/4HANA (Asset Accounting with Plant Maintenance), Oracle NetSuite Fixed Assets Management, Sage
Fixed Assets, Odoo (Assets and Maintenance) and ERPNext, the usual choices for a fixed asset register.

| Capability | Ferrum Assets | SAP S/4HANA | NetSuite FAM | Sage Fixed Assets | Odoo | ERPNext |
|---|:-:|:-:|:-:|:-:|:-:|:-:|
| Register with categories, locations and custodians | ✅ | ✅ | ✅ | ✅ | 🟡 | ✅ |
| Straight line and declining balance | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Sum-of-years and units of production | ❌ | ✅ | ✅ | ✅ | ❌ | ❌ |
| Tax depreciation tables (MACRS and the like) | ❌ | ✅ | ✅ | ✅ | ❌ | 🟡 |
| Several books, each with its own schedule | ✅ | ✅ | ✅ | ✅ | ❌ | ✅ |
| Half-year and mid-month conventions | 🟡 ¹ | ✅ | ✅ | ✅ | 🟡 | 🟡 |
| Depreciation posted to the ledger by itself | ✅ | ✅ | ✅ | 🟡 | ✅ | ✅ |
| Assets made from purchase invoices | ✅ | ✅ | ✅ | 🟡 | ✅ | ✅ |
| Capital work in progress | ✅ | ✅ | ✅ | 🟡 | 🟡 | ✅ |
| One asset from several invoices | ❌ | ✅ | ✅ | 🟡 | ✅ | ✅ |
| Revaluation and impairment | ✅ | ✅ | ✅ | 🟡 | ✅ | ✅ |
| Split a grouped asset | ✅ | ✅ | ✅ | 🟡 | 🟡 | ✅ |
| Partial disposal and transfer to another company | 🟡 ² | ✅ | ✅ | ✅ | 🟡 | 🟡 |
| Sale and scrap with gain or loss per book | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Lease accounting (IFRS 16, ASC 842) | ❌ | ✅ | ✅ | 🟡 | ❌ | ❌ |
| Custody history with due-back dates | ✅ | 🟡 | 🟡 | 🟡 | ❌ | ✅ |
| Physical verification with scanning | ✅ | 🟡 | 🟡 | ✅ | ❌ | ❌ |
| Preventive maintenance plans | ✅ | ✅ | 🟡 | ❌ | ✅ | ✅ |
| Repairs using parts from stock, capitalised | ✅ | ✅ | 🟡 | ❌ | 🟡 | ✅ |
| Meter and condition-based maintenance | ✅ ³ | ✅ | ❌ | ❌ | 🟡 | ❌ |
| Lockout and a separate return to service | ✅ ³ | ✅ | ❌ | ❌ | ❌ | ❌ |
| Calibration records | ✅ ³ | ✅ | ❌ | ❌ | 🟡 | ❌ |
| Maintenance work orders with labour, spares and downtime | 🟡 ⁴ | ✅ | ❌ | ❌ | 🟡 | 🟡 |
| Same ledger as the rest of the books | ✅ | ✅ | ✅ | 🟡 | ✅ | ✅ |
| Self-hosted on your own servers | ✅ | ✅ | ❌ | ✅ | ✅ | ✅ |
| Open source | ✅ | ❌ | ❌ | ❌ | 🟡 | ✅ |
| No per-user licence fee | ✅ | ❌ | ❌ | ❌ | 🟡 | ✅ |

<sub>¹ Pro-rata by days or a full first period. ² A grouped asset is split, then one part disposed of; no transfer
between companies. ³ With the Equipment Maintenance connector and Ferrum's Operations app. ⁴ Requests, logs and
repairs, without planned labour, reserved spares or downtime taken off a workstation's capacity. For the leaders,
🟡 means partly, or through another edition, module or partner product.</sub>

### 🏆 Where Ferrum Assets stands out

- 📒 Postings in the same ledger as the rest of the books, with finance books, cost centres and period locks.
- 🧮 Exact schedules and figures recomputed from documents, so the register and the ledger never disagree.
- 🚶 Room-by-room verification built in, with custody and due-back dates.
- 🔒 Lockout and return to service kept apart, with meters and conditions from real devices.
- 🏠 Open source, self-hosted, no per-user fee.

### 🚧 Where the leaders are still ahead

- 🧮 More depreciation methods, conventions and tax tables (sum-of-years, units of production, MACRS).
- 📄 Lease accounting under IFRS 16 and ASC 842.
- 🏢 Partial disposal, transfer between companies and composite assets from several invoices.
- 🛠️ Full maintenance work orders with labour, spare parts and downtime.

> [!NOTE]
> Leader columns are from public product information as of November 2026 and may change.

## 📍 Roadmap

### ✅ Recently shipped

- [x] **0.3.18** Rebuilt on the tuned design system: better menu icons, a lighter first page, cleaner print.
- [x] **0.3.17** The redesigned interface: kit figures and meters, shaped loading, told-apart states.
- [x] **0.3.16** Reports' names and parameters in Arabic and Hindi.
- [x] **0.3.15** Models, fields and the manifest in Arabic and Hindi.
- [x] **0.3.13** The assistant knows its records and suggests questions.
- [x] **0.3.0** Finance books with their own schedules and gains, repairs that use parts and extend life, splitting grouped assets.

### 🔜 Next up

**Depreciation**
- [ ] Sum-of-years and units-of-production methods, for assets worn by use.
- [ ] Half-year and mid-month conventions, which many tax rules require.
- [ ] Country tax depreciation (US MACRS, India's block of assets) as finance books.

**The asset's life**
- [ ] One asset capitalised from several invoices or from stock.
- [ ] Partial disposal and transfer of an asset to another company.
- [ ] A revaluation surplus kept in equity and moved to retained earnings.

### 🔭 Later

- [ ] Lease accounting under IFRS 16 and ASC 842: right-of-use asset, liability schedule, interest.
- [ ] Maintenance work orders with planned labour, reserved spares and downtime taken off a workstation's capacity.
- [ ] A spare parts list per asset, with critical spares and re-order levels.

## 🚀 Install

Assets ships in the official Ferrum server image; an administrator installs it from **Apps** (`/store/`). Installing
it installs ERP whole if the workspace does not have it yet (ERP 0.6.0 or later). Self-hosters building their own
image enable the server's `assets` feature. **Equipment Maintenance** is offered in Apps where Assets and Operations
are both installed.

First steps:

1. On each company, press **Create Asset Accounts** (or set the asset accounts by hand).
2. Make **locations** (site, building, room) and **asset categories** with their methods, lives and accounts.
3. Mark the items you buy as assets **Is Fixed Asset** with their category.
4. Give people **Asset Manager** or **Asset User**; custodians see the assets they hold.
5. Register existing assets with their opening depreciation, or let purchase invoices draft new ones.

## 🔧 Develop

Clone it next to the platform and ERP:

```text
work/
├── ferrum/              https://github.com/CBFerrum/Ferrum (with Ferrum-Frontend cloned into ferrum/web)
├── ferrum-app-erp/      https://github.com/CBFerrum/ferrum-app-erp (optional)
└── ferrum-app-assets/
```

Point the platform dependencies at your checkout (this file is git-ignored):

```toml
# .cargo/config.toml
[patch."https://github.com/CBFerrum/Ferrum"]
ferrum-apps = { path = "../ferrum/crates/apps" }
ferrum-calc = { path = "../ferrum/crates/calc" }
ferrum-domain = { path = "../ferrum/crates/domain" }
ferrum-kernel = { path = "../ferrum/crates/kernel" }
ferrum-meta = { path = "../ferrum/crates/meta" }
ferrum-ops = { path = "../ferrum/crates/ops" }

# Only to work on ERP alongside it; otherwise Cargo takes ERP from GitHub.
[patch."https://github.com/CBFerrum/ferrum-app-erp"]
ferrum-app-erp = { path = "../ferrum-app-erp" }
```

### 🧪 Tests

The tests need PostgreSQL with an ordinary (non-superuser) role, so row-level security applies:

```bash
createuser ferrum --pwprompt
createdb -O ferrum ferrum_assets_test
createdb -O ferrum ferrum_assets_ops_test
cargo fmt -- --check
cargo clippy --all-targets -- -D warnings
cargo test
```

`FERRUM_ASSETS_TEST_DATABASE_URL` and `FERRUM_ASSETS_OPS_TEST_DATABASE_URL` override their addresses (default
`postgres://ferrum:ferrum@localhost:5432/<database>`). The depreciation maths in
`src/schedule.rs` is pure and unit-tested without a database.

### 🖥️ Interface

The interface lives in `ui/` and builds with Ferrum-Frontend's packages from `../ferrum/web`:

```bash
cd ui
npm run check
npm run typecheck
npm test
npm run build      # into ui/dist, checked against the size budget
```

With a development server started with `FERRUM_DEV_UI=1`, `ferrum-app dev --ui` serves the pages from Vite behind
the real bar and session, with hot reload.

## 📂 Project layout

```text
app.yaml             the manifest: requirements, launcher entry, Create menu, assistant
packages/
├── assets/          categories, locations, assets, movements, verification, value changes, repairs,
│                    maintenance plans and logs, disposals; fields on ERP's records
└── assets_ops/      meters, condition rules, requests, lockouts, calibration, failure codes
connectors/ops/      the Equipment Maintenance connector's manifest
src/                 the engine: schedule maths, posting, purchases, value, disposal, movements,
                     verification, maintenance, the upkeep, reports, the Operations connector
ui/                  its own interface (React + Vite), built into ui/dist
tests/               scenarios against a real PostgreSQL
```

## 📜 Changelog

Every release is in [CHANGELOG.md](CHANGELOG.md).

## 🤝 Contributing

Issues and pull requests are welcome at [github.com/CBFerrum/ferrum-app-assets](https://github.com/CBFerrum/ferrum-app-assets/issues).
Run the checks in [Develop](#-develop) first; a change to the depreciation maths needs a unit test in
`src/schedule.rs`.

## 📄 License

[AGPL-3.0-only](LICENSE).

---

<div align="center">
<sub>Part of <a href="https://github.com/CBFerrum/Ferrum">Ferrum</a>, the open business platform · Made with 🦀 Rust and 🐘 PostgreSQL</sub>
</div>
