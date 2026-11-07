# Changelog

## 0.3.12

Rebuilt with Ferrum-Frontend 5b22e7b: its own words carry built-in Arabic and Hindi (253 words each, ui/src/locales, translated by the model and to be reviewed by native speakers), fetched for the page's language only. The labels it keeps in objects, its dialogs, its page titles and the words it keeps in variables go through t() as well, and the names and sizes it shows as they are keep their order right to left. Its pages are drawn once their words are in, and its entry carries only the parts of the kit it uses.

## 0.3.11

Rebuilt with Ferrum-Frontend 6537d65: an action's link, code or token shown once is kept in a dialog to copy, and on a page that reads right to left, English left untranslated keeps its own order. Its own words go through the kit's t() (186 places), so a workspace's own words for its language reach them, and Admin, Interface words lists them under it (dist/words.json).

## 0.3.10

Rebuilt with Ferrum-Frontend d5d98f2: a workspace's own words for its interface (Admin, Interface words) reach its pages as they load; an attachment field that holds a picture shows it; and a value another record already has is said so, where the form said the record had been changed by someone else. Its own screens' margins, paddings, borders, alignment and insets go by the line's start and end, so they mirror right to left as the kit's do; what it places by coordinates (boards, timelines, charts, handles) stays as it was.

## 0.3.9

Rebuilt with Ferrum-Frontend 63280d3: right to left, its pages mirror as they read (icons that point along the line, margins and alignment by the line's start and end, arrow keys along it), and the records pages' words are in Arabic and Hindi.

## 0.3.8

Rebuilt with Ferrum-Frontend a64416a: a new record's box whose default is 1 shows ticked, as it is saved.

## 0.3.7

Rebuilt with Ferrum-Frontend 922fcfb: an action's dialog says every refusal, the errors on what the action made (another record's rows) among them, and an action with parameters and a confirmation asks once.

## 0.3.6

Rebuilt with Ferrum-Frontend a76295e: its pages show times on a person's own clock when they chose one on their Account page, while today and due dates stay on the workspace's day.

## 0.3.5

Rebuilt with Ferrum-Frontend 24c3951: its pages show times on the workspace's clock, naming the zone when it is not the browser's, a draft numbered on submit is headed as a draft, and a record of an app that is not installed says so.

## 0.3.4

Rebuilt with Ferrum-Frontend bb60115: a record page reads an empty number or check as the server does, so an action whose condition needs one shows, and a workspace's settings open as their one record.

## 0.3.3

Rebuilt with Ferrum-Frontend 00dea07: in a workflow state that names what may change, the other fields show read-only, and the fields the state needs show required.

## 0.3.2

Rebuilt with Ferrum-Frontend daeadda: after an action runs, a record page shows what the action said it did rather than only that it is done.

## 0.3.1

Rebuilt with Ferrum-Frontend 80ff179: a field's label names its control for assistive technology, and a record whose title is a link shows that record's title, not its id.

## Assets 0.3.0: finance books keep their own values; repairs use parts; grouped assets split

What finance books are mostly for: an asset depreciated one way for IFRS and another for tax
(FERRUM-ASSETS-002).

- An asset lists the finance books that depreciate it their own way, each with its method, number of
  depreciations, frequency, rate and salvage value (and, for an existing asset, what it had already
  depreciated); what a book leaves empty is the asset's. Each gets a schedule of its own, generated
  like the asset's and booked by the same upkeep.
- The asset's own depreciation is in every book, as an entry without a book always is. In a book
  that keeps its own, each such entry is taken out again and the book's own is booked instead, so
  the book shows its depreciation alone. Every entry is a journal entry of its book.
- A value adjustment can change one book alone (an impairment under IFRS, say): its entry is that
  book's and only that book's remaining depreciation is re-planned.
- A sale or scrap depreciates every book up to its date; each book's own entry clears what it
  depreciated (or adjusted) beyond the common books into its gain or loss, and the disposal records
  each book's net book value and gain. Cancelling it undoes all of that.
- The fixed asset register and the depreciation schedule take a finance book and show its values
  for the assets it depreciates its own way, and the common values for the rest.

Repairs:

- A capitalized repair can add to the asset's useful life, in months that make whole depreciation
  periods for the asset and for each finance book that keeps its own; the rest of each schedule is
  re-planned over the longer life.
- A repair lists the parts it used. They leave stock in one material issue on its completion date,
  booked to the repair's expense account, and their value is part of the repair's cost (capitalized
  with it, when it is). Cancelling the repair puts the parts back and takes the year away again.

A grouped asset (ten chairs on one line, say) can be split: Split Asset takes some of its units into
an asset of their own, with their share of its cost, salvage value, opening depreciation and every
schedule row in every book; the asset keeps what rounding leaves. Depreciation booked before the
split stays booked by the entries that booked it, which the two assets share, so neither undoes it
(neither is cancelled, nor disposed of on a day before it). An asset whose value a repair or an
adjustment changed is not split.

The asset's single Finance Book field, a label, is gone: an asset is in every book, and in the books
it lists with values of their own. Needs ERP 0.6.0 or later.

## Assets 0.2.1: finance books are accounting's

ERP 0.6.0 keeps finance books for parallel accounting, as entries of their own in each book. Assets'
own Finance Book, a label until now, is that one: Assets no longer defines it, the field name and the
records stay as they were, and an asset names a book as before. Install it with ERP 0.6.0 or later:
the two define the book together otherwise, and a server holding both would not start.

## Assets Operations 0.1.0

A connector between Assets and Operations, for maintaining equipment that is also on the floor: meters
read from devices, condition rules that raise requests, failure codes, calibration records, and
lockout with a separate return to service, because repaired does not mean safe to run. A lockout here
is a record, not a physical lock. Run against simulated equipment only.

## 0.2.0

- Assets has an interface of its own, at /assets/. Its overview shows what the company owns and what it is
  worth (by category or by place, as of any date), the depreciation due month by month, the maintenance
  coming up, and what needs a person: depreciation that could not be booked, assets past their return
  date, not found at the last check, due to be verified, out of order, or with a warranty ending. Every
  figure opens the list or report behind it.
- An asset has a page of its own: where it is, who has it, what it is worth, a chart of its value over
  time (booked solid, planned dashed, a period that could not be booked in red with the reason), every
  period of its schedule, its maintenance, and its whole history on one timeline. Moving it, handing it
  out, taking it back, logging maintenance, reporting a fault, revaluing, selling, scrapping and
  verifying are one step from the page.
- Verify a room: pick a place, scan or type tags and serial numbers (or tick things off by hand) and each
  one is checked in the register at once. What the register expects there and has not seen is listed,
  what turns up in the wrong room can be moved with a tap, and the rest can be marked as not found.
- Assets carry a serial number, manufacturer, model, warranty end and end of life, which stay editable
  after the asset is submitted (Edit Details), and the list search finds an asset by serial number.
- Physical verification: a new document, Asset Verification, records that someone saw an asset (or looked
  and could not find it). An asset category can say how often its assets are verified, and the asset
  carries when it was last verified, what was found and when the next check is due.
- Handing an asset out can say when it is due back. The asset carries the date until it is taken back or
  handed on, and the movement history report shows it.
- A sold, scrapped or cancelled asset is due nothing: its return and next verification dates clear, and
  come back if the disposal is cancelled.
- Personal data: a person's request now reaches assets in their custody and maintenance logs assigned
  to them, and an erasure takes their name off them. It also reaches who verified an asset.
