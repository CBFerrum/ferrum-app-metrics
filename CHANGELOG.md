# Changelog

## 0.4.1

Rebuilt with Ferrum-Frontend d5d98f2: a workspace's own words for its interface (Admin, Interface words) reach its pages as they load; an attachment field that holds a picture shows it; and a value another record already has is said so, where the form said the record had been changed by someone else.

## 0.4.0

A dataset may pivot, and compare (FERRUM-ECOSYSTEM-010). Its columns still stay the same whatever
its rows, so certification keeps them as before:
- **A pivot names its values** (`pivot.values`, which needs Ferrum with named pivot values): one
  column for each, in that order, whatever the records hold, the column of a value no record has
  kept; the totals count every value. A pivot whose columns come from its rows is still refused.
- **A comparison** adds each measure's previous period (the period before, or the year before)
  beside it, as a Studio query's does.

## 0.3.3

Rebuilt with Ferrum-Frontend 63280d3: right to left, its pages mirror as they read (icons that point along the line, margins and alignment by the line's start and end, arrow keys along it), and the records pages' words are in Arabic and Hindi.

## 0.3.2

Rebuilt with Ferrum-Frontend a64416a: a new record's box whose default is 1 shows ticked, as it is saved. An action's dialog says every refusal, the errors on what the action made (another record's rows) among them, and an action with parameters and a confirmation asks once. A metric's and a dataset's definition is built on its page with Studio's report builder (what it is about, its columns or groupings and measures across linked records, filters, the period and sort), the first rows it gives shown beside it, or written as JSON as before: its field says `widget: studio_query`, which needs Ferrum with field widgets.

## 0.3.1

Rebuilt with Ferrum-Frontend a76295e: its pages show times on a person's own clock when they chose one on their Account page, while today and due dates stay on the workspace's day.

## 0.3.0

Governed datasets, and the warehouse feed (FERRUM-ECOSYSTEM-010).

- **Dataset:** a table defined once as a Studio query: record fields, or groupings and measures,
  with filters and sort, reaching linked records and child tables by `link.field`. A pivot or a
  comparison is refused, since a warehouse needs columns that stay the same whatever the rows.
  It is checked against the records and its author's rights when saved.
- **Governed as a metric is:** a Data Steward who does not own it certifies it, which keeps its
  columns ("name (Label): type"). A changed definition puts it back in draft. It is deprecated
  naming what replaces it. Metric Readers see certified datasets only.
- **Lineage across records:** a metric's or a dataset's lineage now names each linked record it
  reads and by which field, e.g. "Customer: payment_terms_days; CustomerGroup (by customer_group):
  group_name".
- **The warehouse feed:** `GET /api/app/metrics/dataset?name=&format=csv|ndjson` gives a certified
  dataset's rows to whoever asks (an API key's person, for a warehouse), with their own rights, up
  to 10,000 rows. `x-ferrum-dataset-hash` carries the definition's fingerprint, and
  `x-ferrum-truncated` says when rows were cut short. CSV quotes as RFC 4180 does, and turns a text
  a spreadsheet would run as a formula into plain text; JSON lines carry the values exactly.
  `GET /api/app/metrics/values?since=` gives certified metrics' kept values after a time, oldest
  first, without who read them; `x-ferrum-next-since` is where the next load starts.
- The home page lists certified datasets with their CSV and JSON lines addresses.

## 0.2.2

Rebuilt with Ferrum-Frontend 24c3951: its pages show times on the workspace's clock, naming the zone when it is not the browser's, a draft numbered on submit is headed as a draft, and a record of an app that is not installed says so.

## 0.2.1

Rebuilt with Ferrum-Frontend bb60115: a record page reads an empty number or check as the server does, so an action whose condition needs one shows, and a workspace's settings open as their one record.

## 0.2.0

Followed metrics (FERRUM-ECOSYSTEM-010: scheduled delivery).

- A person follows a certified metric (`MetricSubscription`), daily, weekly or monthly, for
  themselves only, once each. On its day in the workspace, upkeep reads it with their own rights,
  keeps the value as theirs (a Metric Value), and tells them by `metrics.digest`, in the app unless
  they chose otherwise. Each follower sees only their own subscriptions.
- A metric deprecated since is said so once, naming what replaces it, and stops. One back in draft
  keeps why it was not sent, and is delivered again once certified. A follower no longer active
  keeps why too.
- Personal-data requests now reach metrics' owners and certifiers, terms' owners, readers of
  values and followers.

## 0.1.3

Rebuilt with Ferrum-Frontend 00dea07: in a workflow state that names what may change, the other fields show read-only, and the fields the state needs show required.

## 0.1.2

Rebuilt with Ferrum-Frontend daeadda: after an action runs, a record page shows what the action said it did rather than only that it is done.

## 0.1.1

Rebuilt with Ferrum-Frontend 80ff179: a field's label names its control for assistive technology, and a record whose title is a link shows that record's title, not its id.

## 0.1.0

Governed metrics on Studio (FERRUM-ECOSYSTEM-010).

- A metric as a Studio query of one measure, checked when saved; its lineage worked out from it.
- Certification by a Data Steward who does not own it; a changed definition back to draft.
- Values read with each person's own rights, at `/api/app/metrics/value`.
- Values kept as snapshots, with the definition's fingerprint.
- Deprecation naming the replacement; a glossary of business terms.
- Its own interface at `/kpis/`: the certified metrics by domain.
