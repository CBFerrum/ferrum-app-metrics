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
| 🖥️ Interface | its own, at `/kpis/`: the certified metrics by domain, each with its value for the viewer, and the certified datasets with their addresses for a warehouse |
| 🔌 Endpoints | `GET /api/app/metrics/value?metric=<name>`; the warehouse feed `GET /api/app/metrics/dataset?name=<name>&format=csv\|ndjson` and `GET /api/app/metrics/values?since=<time>` |
| 🔐 Roles | Metric Author, Data Steward, Metric Reader |
| 📜 Licence | AGPL-3.0-only |

## Highlights

- **Defined once.** A metric is a Studio query of exactly one measure, ungrouped. It is checked when
  saved: a query that does not run against the records, or names a field that is not there, is
  refused.
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
  fingerprint then. Readers see the values they kept; stewards see them all.
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
