# Metrics

Governed metrics for [Ferrum](https://github.com/CBFerrum/Ferrum), on Studio:
- a metric defined once, as a query of one measure, with its lineage;
- certified by a steward who does not own it, and kept under change control;
- read by everyone with their own rights to the records it counts;
- a glossary of the words the business uses.

| | |
|---|---|
| 📊 App | `metrics`, native, in the server |
| 🧩 Package | `metrics` |
| 🖥️ Interface | its own, at `/kpis/`: the certified metrics by domain, each with its value for the viewer |
| 🔌 Endpoint | `GET /api/app/metrics/value?metric=<name>` |
| 🔐 Roles | Metric Author, Data Steward, Metric Reader |
| 📜 Licence | AGPL-3.0-only |

## Highlights

- **Defined once.** A metric is a Studio query of exactly one measure, ungrouped. It is checked when
  saved: a query that does not run against the records, or names a field that is not there, is
  refused.
- **Lineage.** Each metric records the record and the fields it reads, worked out from its
  definition, never typed in.
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

## Develop

```bash
cargo test                       # needs FERRUM_METRICS_TEST_DATABASE_URL's database
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

## Not yet

- Metrics over time (a series), and targets with alerts when a value crosses them.
- Metrics built from other metrics (ratios), and a semantic layer joining records.
- Values kept on a schedule, rather than when someone records them.
- Lineage beyond one record: the fields a linked record contributes.

## Changelog

| Version | Highlights |
|---|---|
| **0.1.1** | Rebuilt with Ferrum-Frontend 80ff179: labels name their fields, and linked titles show their titles. |
| **0.1.0** | Metrics defined once on Studio, certified by a steward, kept under change control with their lineage, read with each person's own rights, and a glossary (FERRUM-ECOSYSTEM-010). |

## License

AGPL-3.0-only. See [LICENSE](LICENSE).
