# Metrics

Governed metrics and datasets for [Ferrum](https://github.com/CBFerrum/Ferrum), on Studio:
- a metric defined once, as a query of one measure, with its lineage;
- a dataset defined once, as a table across linked records;
- certified by a steward who does not own it, and kept under change control;
- read by everyone with their own rights to the records it counts, and loaded into a warehouse;
- a glossary of the words the business uses.

| | |
|---|---|
| 📊 App | `metrics`, native, in the server |
| 🧩 Package | `metrics` |
| 🖥️ Interface | its own, at `/kpis/`: the certified metrics by domain, each with its value for the viewer (saying when it is stale, or why there is none now), and the certified datasets with their addresses for a warehouse |
| 🔌 Endpoints | `GET /api/app/metrics/value?metric=<name>`, `series?metric=<name>&bucket=month&periods=12`, `impact?metric=<name>`; the warehouse feed `GET /api/app/metrics/dataset?name=<name>&format=csv\|ndjson` and `GET /api/app/metrics/values?since=<time>`; `POST /api/app/metrics/import/wren` |
| 🔐 Roles | Metric Author, Data Steward, Metric Reader |
| 🛰️ Service | `ferrum-metrics-engine` (`engine/`): external datasets through Wren on DataFusion, run beside the server ([docs/engine.md](docs/engine.md)) |
| 📜 Licence | AGPL-3.0-only |

## Highlights

- **Defined once.** A metric is a Studio query of exactly one measure, ungrouped. It is checked when
  saved: a query that does not run against the records, or names a field that is not there, is
  refused.
- **Composed of others.** A metric can be a formula over other metrics by name, `[Revenue] / [Orders]`,
  worked out from each part's value for whoever reads it. It is never made of itself, even by way of
  others, it is certified after its parts, and a part deprecated or a division by zero leaves it without a
  value, said so. `GET /api/app/metrics/impact?metric=<name>` lists the metrics a change reaches.
- **Period by period.** `GET /api/app/metrics/series?metric=<name>&bucket=month&periods=12` gives its value
  for each period by its `series_by` date field, the current period marked partial; every value says when
  it was worked out (`as_of`). A series goes by day, week, month, quarter or year, or by the workspace's
  fiscal quarters and years (`fiscal_quarter`, `fiscal_year`), which start in its fiscal start month.
- **In one currency.** A metric that sums amounts in several currencies can read them in one (`convert_to`):
  each row at the exchange rate valid on its own day (`rate_date_field`), the latest from that day or
  before, direct or else the inverse of the opposite rate, as documents are converted. Products stay
  exact and each total is rounded half to even (`round_to`, 2 places by default). The rule, the currency
  and the rounding are part of what certification fixes, so changing any of them is a draft again. A row
  in a currency without a rate by its day leaves the metric without a value, saying which rate to add,
  never a total without that row; a read that would hold more currency-days than one read takes is
  refused rather than cut short. Only sums convert, a formula keeps to one currency, and a currency on
  child rows, which would count a document once per row, is refused when the metric is saved.
- **From external datasets.** A metric can be read from a data lake's CSV or Parquet files instead of the
  records (`external`): a Wren semantic model over the lake's datasets and one query over its models. Metrics'
  engine runs it: a service of its own beside the server, planning the query through the model with Wren's
  engine (pinned) and running it on DataFusion. Each dataset is an **External Source** a Data Steward
  describes. It says which rows each reader sees (theirs by address, or the values their roles map to), which
  columns only some roles see, and how fresh its data must be.
  - The engine is asked with what that reader may see and never more. It reads each dataset through a table
    that filters its rows and leaves out those columns in its own scan, so no model, rename, calculated column
    or relationship reaches past them.
  - A query is one SELECT over the model's models. Anything else is refused, and so is what Wren cannot plan
    exactly. Time, memory, rows and concurrency are bounded, and so are how deep a query's terms nest, how
    many models it reads and how many a semantic model holds, before anything is planned. A query's time holds
    whether or not its work yields.
  - A value read from data older than its source allows says so (`stale`), as does anything composed of it.
  - The query is checked when the metric is saved, on its author's own view of the datasets, and must answer
    with one number. Certification fixes it as it fixes a query of the records.
  - See [docs/engine.md](docs/engine.md).
- **From Wren.** `POST /api/app/metrics/import/wren` takes a Wren semantic model (MDL) and reviews each
  cube measure against the metrics defined here: `new`, `same`, `duplicate` (another name already means
  it), `conflict` (that name means something else, and stays so) or `unsupported`, with the reason. Only
  one aggregate of one column is brought; a calculated column, a relationship, a view or any other
  expression is refused rather than approximated. A model maps to a record by `map`, or by name. What a
  reviewer accepts (a conflict only under another name) is created as drafts, all or none, with the
  importer's rights, to be certified like any other. Wren's own row and column rules are not brought:
  each reader's rights here decide, and the review says so.
- **Lineage.** Each metric and dataset records the record and the fields it reads, and each linked
  record's by the field that reaches it, worked out from its definition, never typed in.
- **Datasets.** A dataset is a table defined once as a Studio query: record fields, or groupings and
  measures, reaching linked records (`customer_group.group_name`). Its columns stay the same
  whatever its rows, and certification keeps them: a pivot names its values (one column each,
  whatever the records hold), and a comparison adds each measure's period before. It is governed as a
  metric is: certified by a steward who does not own it, back in draft when its definition
  changes, deprecated naming what replaces it.
- **Into a warehouse.** A warehouse, or any loader of CSV or JSON lines over HTTPS, loads a
  certified dataset from `/api/app/metrics/dataset?name=…&format=csv|ndjson` with an API key, and
  gets the rows that key's person may read, up to 10,000. The definition's fingerprint
  (`x-ferrum-dataset-hash`) tells a load the definition changed. `/api/app/metrics/values?since=…`
  gives certified metrics' kept values after a time, oldest first, without who read them, and
  `x-ferrum-next-since` says where the next load starts. A draft is tried by its owner only.
- **Certified by someone else.** A Data Steward certifies a metric, and never one they own.
  Certification fixes the definition: a changed definition puts the metric back in draft until it is
  certified again. Its description and other words may change freely.
- **Read with your own rights.** A metric's value is worked out for whoever reads it, with their
  rights to the records it counts. Two sales reps and their director read three numbers from one
  definition, each the right one for them.
- **Kept values.** A reader records a metric's value as they read it, with the definition's
  fingerprint then and what the metric meant (its query or formula, and how it converts), so a value
  kept before a change still says what it counted. A kept value was read with its reader's rights, so
  it is theirs alone: no author, steward or follower of the metric reads another person's. Administrators
  see them all.
- **Deprecation.** A steward deprecates a metric, naming what replaces it; it is read no more, and
  whoever asks is pointed at its replacement.
- **Who sees what.** Metric Readers see certified metrics only. A draft is read only by its owner.

- **Followed.** A person follows a certified metric, daily, weekly or monthly. On its day in the
  workspace, upkeep reads it for them with their own rights, keeps that as a value of theirs, and
  tells them the way they chose for "Metrics you follow". A metric deprecated since is said so
  once, with what replaces it, and is no longer delivered. One back in draft waits until it is
  certified again. Nobody follows for someone else, and followers see only their own.
- **Personal data.** Requests reach who owns, certifies, reads and follows a metric, who owns and
  certifies a dataset, and who keeps a term.

## Develop

```bash
cargo test                       # needs FERRUM_METRICS_TEST_DATABASE_URL's and FERRUM_METRICS_DATASETS_TEST_DATABASE_URL's databases
cd ui && npm run build && npm run check
```

`tests/metrics_it.rs` walks a sales team's metrics:
- definitions refused when they do not run;
- certification, refused to the author and to an owner who is a steward;
- one definition read as three numbers by two reps and their director, for a count and an average;
- kept values, and who sees them;
- a changed definition back to draft, then certified anew;
- deprecation pointing at the replacement;
- who may do what.

`tests/datasets_it.rs` walks a dataset across customers and their groups:
- definitions refused for a pivot that does not name its values, for no columns, and when they do
  not run;
- its lineage naming the customer group it reaches;
- tried in draft by its owner only; certified by a steward, keeping its columns;
- loaded as JSON lines and CSV by a director and two reps, each with their own rights;
- a changed definition back to draft; deprecation naming what to load instead;
- a pivot over the customer types it names, certified with a column for each and loaded, the
  column of a type no one has kept;
- metrics' kept values loaded since a time, a reader loading her own.

## Not yet

- Metrics over time (a series), and targets with alerts when a value crosses them.
- Metrics built from other metrics (ratios). Datasets reach linked records along their links (the
  join graph Studio follows); there is no free join between unrelated records.
- Pushing to a warehouse: it pulls (with an API key), up to 10,000 rows of a dataset a load, and
  datasets are loaded whole each time; only metric values load from where the last load stopped.
- Values kept on a schedule for no one in particular: they are kept as each follower reads them.

## Changelog

| Version | Highlights |
|---|---|
| **0.7.1** | The KPI page says when a value is stale, and why a metric has no value now; the engine bounds how deep and wide a query is before planning it. |
| **0.7.0** | Metrics read from external datasets through Metrics' engine (Wren on DataFusion), with each reader's rows and columns and fresh-or-stale said; External Sources. |
| **0.6.0** | Currencies by the day's rate, fiscal series, kept values that keep their meaning and stay their readers', and Wren models reviewed into drafts. |
| **0.5.0** | Composed metrics, series with the period not yet over marked partial, and impact. |
| **0.4.5** | Its names in Arabic and Hindi; the last English sentences translated. |
| **0.4.4** | The assistant knows its records. |
| **0.4.3** | Rebuilt with Ferrum-Frontend 5b22e7b: its own words in Arabic and Hindi. |
| **0.4.2** | Rebuilt with Ferrum-Frontend 6537d65: once-shown answers kept; English isolated right to left; its own words translatable by a workspace. |
| **0.4.1** | Rebuilt with Ferrum-Frontend d5d98f2: a workspace's own interface words; pictures in attachment fields; a duplicate value said so. |
| **0.4.0** | A dataset pivots over the values it names, and compares a period with the one before; its columns still stay the same. |
| **0.3.3** | Rebuilt with Ferrum-Frontend 63280d3: right to left, pages mirror; the records pages' words in Arabic and Hindi. |
| **0.3.2** | Rebuilt with Ferrum-Frontend a64416a: a box whose default is 1 shows ticked; action dialogs say every refusal and ask once; definitions built with Studio's report builder. |
| **0.3.1** | Rebuilt with Ferrum-Frontend a76295e: times on a person's own clock when they chose one. |
| **0.3.0** | Governed datasets across linked records, certified with their columns; the warehouse feed of datasets as CSV or JSON lines and of metrics' values since a time; lineage naming linked records. |
| **0.2.2** | Rebuilt with Ferrum-Frontend 24c3951: times on the workspace's clock; drafts headed as drafts. |
| **0.2.1** | Rebuilt with Ferrum-Frontend bb60115: empty numbers read as the server reads them; settings open as their record. |
| **0.2.0** | Certified metrics delivered to who follows them, read with their own rights, daily, weekly or monthly; personal data declared. |
| **0.1.3** | Rebuilt with Ferrum-Frontend 00dea07: a workflow state's kept fields are read-only, its needed ones required. |
| **0.1.2** | Rebuilt with Ferrum-Frontend daeadda: actions say what they did. |
| **0.1.1** | Rebuilt with Ferrum-Frontend 80ff179: labels name their fields, and linked titles show their titles. |
| **0.1.0** | Metrics defined once on Studio, certified by a steward, kept under change control with their lineage, read with each person's own rights, and a glossary (FERRUM-ECOSYSTEM-010). |

## License

AGPL-3.0-only. See [LICENSE](LICENSE).

`tests/external_it.rs` runs the engine beside the kernel over HTTP: readers' rows and columns, a metric over a
hidden column refused to an author who cannot see it, a composed metric of an external metric and one of the
records, fresh and stale, and the engine gone. `engine/tests/` checks the engine alone: Wren's models,
relationships and calculated columns answered exactly; what has no exact meaning refused; the sandbox, the
bounds and the token; and a query as deep as one of the largest size can be, refused before it is planned,
on a thread with the engine's own stack.

`tests/currency_it.rs` checks sales in four currencies against numbers worked out by hand: a rate on
its own first day, the inverse of an opposite rate, a refund dated back across a fiscal year, ties
rounded to the even digit, each reader's own sales, fiscal years and quarters from April, a sale with two
matching lines counted once, lines at their sale's rate, a currency without a rate, and a kept value
that still says what it meant after the rounding changed. `tests/wren_it.rs` brings in a Wren model with
a revenue that conflicts with a certified one.
