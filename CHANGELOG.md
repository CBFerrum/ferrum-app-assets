# Changelog

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
