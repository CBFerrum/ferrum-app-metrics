<div align="center">

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/assets/banner-dark.svg">
  <img alt="Ferrum Metrics: one certified definition for every number" src="docs/assets/banner-light.svg" width="100%">
</picture>

<br>

[![Version](https://img.shields.io/badge/version-0.7.4-0f766e?style=flat-square)](CHANGELOG.md)
[![Licence: AGPL-3.0](https://img.shields.io/badge/licence-AGPL--3.0-blue?style=flat-square)](LICENSE)
[![Ferrum platform](https://img.shields.io/badge/runs%20on-Ferrum-2f5bea?style=flat-square)](https://github.com/CBFerrum/Ferrum)
![Rust](https://img.shields.io/badge/Rust-server%20and%20engine-b7410e?style=flat-square&logo=rust)
![TypeScript](https://img.shields.io/badge/React-TypeScript-3178c6?style=flat-square&logo=typescript&logoColor=white)
![PostgreSQL](https://img.shields.io/badge/PostgreSQL-data-336791?style=flat-square&logo=postgresql&logoColor=white)
![Tests](https://img.shields.io/badge/tests-28%20Rust%20%C2%B7%205%20UI-16a34a?style=flat-square)

### Define every number the company steers by once, certify it, and let each person read it with their own rights.

Metrics is the governed metrics layer of Ferrum. A metric or a dataset is defined once as a Studio query (or a formula over other metrics, or a query over files in a data lake), certified by a steward who does not own it, kept under change control with its lineage, and read by everyone with the rights they already have. It is for companies that are tired of three different "revenue" figures, and it replaces a stand-alone metrics catalogue.

[💡 Why](#-why-metrics) · [📸 Screenshots](#-screenshots) · [✨ Features](#-features) · [📊 Compare](#-how-it-compares) · [📍 Roadmap](#-roadmap) · [🚀 Install](#-install) · [🔧 Develop](#-develop)

</div>

## 💡 Why Metrics

- 🎯 **One definition, one number.** A metric is defined once. The same definition feeds the certified metrics page, followed digests, goals in Objectives, answers in Intelligence and the warehouse feed.
- ✅ **Certified by someone else.** A Data Steward certifies a metric and never one they own. Certification fixes the definition: change it and the metric is a draft again.
- 🔐 **Everyone reads with their own rights.** Two sales reps and their director read three numbers from one definition, each the right one for them. Nobody is told a figure they could not see in the records.
- 🧬 **Lineage worked out, not typed.** Each metric records the record and the fields it reads, linked records included, from its definition.
- 💱 **Money that adds up.** Sums in several currencies convert at each row's own day rate, round half to even, and refuse to guess when a rate is missing.
- 🪣 **Data lake, governed.** A metric can read CSV or Parquet files through a Wren semantic model and a DataFusion engine, with each reader's rows and columns applied in the scan and a freshness limit that says when a figure is stale.
- 🏭 **Warehouse feed.** Certified datasets and kept values load into a warehouse over HTTPS with an API key, with the rows that key's person may read.
- 🔓 **Open source and self-hosted.** AGPL-3.0, your PostgreSQL, one engine container only if you use external files.

## 🎯 At a glance

| | |
|---|---|
| **Version** | 0.7.4 (package `metrics` 0.1.0) |
| **Kind** | Its own interface at `/kpis/` (certified metrics by domain, certified datasets) with Ferrum's records screens for everything else, and a separate engine service for external files |
| **What it adds** | 7 entities (Metric, Dataset, Business Term, Metric Value, Followed Metric, External Source and a small Term table), 3 roles, 1 module, 6 endpoints, 1 upkeep job (followed digests), 1 notification kind, no reports, no print formats |
| **Depends on** | Ferrum platform `>=0.3.2, <0.4` (core, Studio and the calculation crate); the optional engine for external datasets |
| **Works with** | Objectives (key results measured by a metric), Intelligence (questions about metrics), ERP and any app whose records a metric counts, warehouses and loaders |
| **Languages** | English, Arabic and Hindi (28 interface words each) with names in Arabic and Hindi, right-to-left |
| **Tests** | 28 Rust tests (against a real PostgreSQL, the engine alone, and the engine beside the kernel over HTTP), 5 interface tests |
| **Licence** | AGPL-3.0-only |

## 📸 Screenshots

<picture><source media="(prefers-color-scheme: dark)" srcset="docs/assets/screens/metrics-home-dark.webp"><img alt="Metrics — home" src="docs/assets/screens/metrics-home-light.webp" width="100%"></picture>

<p align="center"><b>🏠 Certified metrics</b><br><sub>Read each certified metric by domain with your own value, and download certified datasets for a warehouse.</sub></p>

<table>
<tr>
<td width="50%" valign="top">
<picture><source media="(prefers-color-scheme: dark)" srcset="docs/assets/screens/metrics-metric-list-dark.webp"><img alt="Metrics — list" src="docs/assets/screens/metrics-metric-list-light.webp"></picture>
<p><b>📊 Metrics</b><br><sub>Filter metrics by domain, owner and status: draft, certified or deprecated.</sub></p>
</td>
<td width="50%" valign="top">
<picture><source media="(prefers-color-scheme: dark)" srcset="docs/assets/screens/metrics-business-term-list-dark.webp"><img alt="Glossary — list" src="docs/assets/screens/metrics-business-term-list-light.webp"></picture>
<p><b>📚 Glossary</b><br><sub>Keep the words the business uses, each with a definition and an owner.</sub></p>
</td>
</tr>
<tr>
<td width="50%" valign="top">
<picture><source media="(prefers-color-scheme: dark)" srcset="docs/assets/screens/metrics-dataset-list-dark.webp"><img alt="Datasets — list" src="docs/assets/screens/metrics-dataset-list-light.webp"></picture>
<p><b>🗃️ Datasets</b><br><sub>See every dataset with its domain, owner and certification status.</sub></p>
</td>
<td width="50%" valign="top">
<picture><source media="(prefers-color-scheme: dark)" srcset="docs/assets/screens/metrics-business-term-detail-dark.webp"><img alt="Glossary term — open record" src="docs/assets/screens/metrics-business-term-detail-light.webp"></picture>
<p><b>🔤 A glossary term</b><br><sub>Define a term once and link it from the metrics that use it.</sub></p>
</td>
</tr>
<tr>
<td width="50%" valign="top">
<picture><source media="(prefers-color-scheme: dark)" srcset="docs/assets/screens/metrics-metric-detail-dark.webp"><img alt="Metric — open record" src="docs/assets/screens/metrics-metric-detail-light.webp"></picture>
<p><b>🧮 One metric</b><br><sub>Build the definition with the query builder, see what it gives now, then certify or deprecate it.</sub></p>
</td>
<td width="50%" valign="top">
<picture><source media="(prefers-color-scheme: dark)" srcset="docs/assets/screens/metrics-dataset-detail-dark.webp"><img alt="Dataset — open record" src="docs/assets/screens/metrics-dataset-detail-light.webp"></picture>
<p><b>🧾 One dataset</b><br><sub>See a dataset's definition, its lineage and how a warehouse loads it.</sub></p>
</td>
</tr>
</table>

<table>
<tr>
<td align="center" valign="top">
<picture><source media="(prefers-color-scheme: dark)" srcset="docs/assets/screens/metrics-home-phone-dark.webp"><img alt="Metrics — home" src="docs/assets/screens/metrics-home-phone-light.webp" width="260"></picture>
<p><b>📱 Certified metrics (phone)</b><br><sub>Read your figures on a phone.</sub></p>
</td>
<td align="center" valign="top">
<picture><source media="(prefers-color-scheme: dark)" srcset="docs/assets/screens/metrics-metric-list-phone-dark.webp"><img alt="Metrics — list" src="docs/assets/screens/metrics-metric-list-phone-light.webp" width="260"></picture>
<p><b>📱 Metrics (phone)</b><br><sub>Browse metrics on a phone.</sub></p>
</td>
</tr>
</table>


## ✨ Features

### 📐 Defining a metric

- **Metric** is a number defined once: name, description, owner, domain, unit and the definition. The name is unique. The owner is whoever records it.
- **A Studio query of one measure.** The definition is a Studio query with exactly one measure, ungrouped (a sum, count, average, least or most), built with Studio's report builder in the form, which shows what it gives now. It is checked when saved: a query that does not run against the records, or names a field that is not there, is refused.
- **Composed of others.** A metric can be a formula over other metrics by name, `[Revenue] / [Orders]`, worked out from each part's value for whoever reads it. A formula names at most 20 metrics and goes at most 8 deep. It is never made of itself, even by way of others, it is certified after its parts, and a part deprecated, missing or a division by zero leaves it without a value, saying so.
- **A query or a formula, not both.** And an external query is its own alone.
- **Series by.** The date field a series of it goes by (`posting_date`).
- **Glossary terms** link a metric to the words the business uses.
- **Domain** groups metrics (Sales, Finance, People) on the home page and in filters.

### 🪣 From external datasets

- **A metric over files.** A metric can be read from a data lake's CSV or Parquet files (`external`): a Wren semantic model over the datasets and one query over its models. Metrics' **engine** runs it.
- **External Source** is the record a Data Steward keeps for each dataset: its name on the engine, how fresh its data must be (24 hours by default, 0 for never stale), which rows each reader sees (all; those whose column equals the reader's address; or those whose column is a value the reader's roles map to; every rule must hold), and which columns only some roles see. Administrators see every row and column.
- **The reader alone.** The engine reads each dataset through a table that filters its rows and leaves out those columns in its own scan, so no model, rename, calculated column or relationship reaches past them.
- **Checked at save.** The query is checked when the metric is saved, on its author's own view of the datasets, and must answer with one number. Certification fixes it as it fixes a query of the records.
- **Stale is said.** A value read from data older than its source allows says so (`stale`), and so does anything composed of it. With the engine gone the metric has no value, said so.
- **No series yet** for external metrics.
- See Metrics' engine under How it works and [docs/engine.md](docs/engine.md).

### 💱 Currencies and fiscal periods

- **One currency.** A metric that sums amounts in several currencies can read them in one (`convert_to`): each row at the exchange rate valid on its own day (`rate_date_field`), the latest from that day or before, direct or else the inverse of the opposite rate, as documents are converted.
- **Exact money.** Products stay exact and each total is rounded half to even (`round_to`, 2 places by default, 0 to 6).
- **Certification fixes it.** The rule, the currency and the rounding are part of what certification fixes, so changing any of them is a draft again.
- **Never a total without a row.** A row in a currency without a rate by its day leaves the metric without a value, saying which rate to add. A read that would hold more currency-days than one read takes is refused rather than cut short.
- **Limits.** Only sums convert; a formula keeps to one currency; a currency on child rows, which would count a document once per row, is refused when the metric is saved.
- **Fiscal series.** A series goes by day, week, month, quarter or year, or by the workspace's fiscal quarters and years, which start in its fiscal start month.

### 📈 Series and values

- **A value** for whoever asks: `GET /api/app/metrics/value?metric=<name>` gives the value, its unit, when it was worked out (`as_of`) and any stale sources.
- **A series** for 1 to 120 periods: `GET /api/app/metrics/series?metric=<name>&bucket=month&periods=12` gives its value for each period by its series date field, the current period marked partial. Buckets: `day`, `week`, `month`, `quarter`, `year`, `fiscal_quarter`, `fiscal_year`.
- **Composed metrics have series** too, part by part.
- **Kept values.** A reader records a metric's value as they read it (**Record Its Value**), with the definition's fingerprint then and what the metric meant (its query or formula, and how it converts), so a value kept before a change still says what it counted. A kept value was read with its reader's rights, so it is theirs alone: no author, steward or follower of the metric reads another person's. Administrators see them all.
- **Impact.** `GET /api/app/metrics/impact?metric=<name>` lists the metrics a change would reach.

### 🧾 Governing a metric

- **Draft, Certified, Deprecated.** A new metric is a draft, read by its owner only. A Data Steward certifies it with **Certify It**, and never a metric they own, and never one made of parts that are not certified yet.
- **Change control.** Certification fixes the definition by a fingerprint. A changed definition puts the metric back in draft until certified again. Its description and other words may change freely.
- **Deprecation.** **Deprecate It** names what replaces it (never itself). It is read no more, and whoever asks is pointed at its replacement.
- **Lineage.** The record and fields it reads, and each linked record's by the field that reaches it, worked out from the definition.
- **Who sees what.** Metric Readers see certified metrics only. Authors and stewards see drafts as their roles allow.
- **Business glossary.** **Glossary** terms (unique) with a definition and an owner, linked from metrics and datasets.

### 🗃️ Datasets and the warehouse feed

- **Dataset** is a table defined once as a Studio query: record fields, or groupings and measures, reaching linked records (`customer_group.group_name`). Its columns stay the same whatever its rows, and certification keeps them. A pivot names its values (one column each), and a comparison adds each measure's period before.
- **Governed like a metric:** draft, certified by a steward who does not own it, back in draft when its definition changes, deprecated naming what replaces it. A draft is tried by its owner only.
- **Loaded into a warehouse.** `GET /api/app/metrics/dataset?name=<name>&format=csv|ndjson` with an API key returns the rows that key's person may read, up to 10,000. `x-ferrum-dataset-hash` carries the definition's fingerprint, so a load knows the definition changed.
- **Values feed.** `GET /api/app/metrics/values?since=<time>` returns certified metrics' kept values after a time, oldest first, without who read them, and `x-ferrum-next-since` says where the next load starts.
- **Download from the home page.** Each certified dataset has CSV and JSON lines buttons.

### 🔔 Followed metrics

- A person **follows** a certified metric daily, weekly (the default) or monthly. On its day in the workspace, upkeep reads it for them with their own rights, keeps that as a value of theirs, and tells them the way they chose for the notification kind **Metrics you follow** (in-app by default).
- Nobody follows for someone else, and followers see only their own.
- A metric deprecated since is said so once, with what replaces it, and is no longer delivered. One back in draft waits until it is certified again. A failed delivery keeps its reason on the follow (`last_error`).
- Upkeep delivers up to 200 follows a round.

### 📥 Wren import

`POST /api/app/metrics/import/wren` takes a Wren semantic model (MDL) and reviews each cube measure against the metrics defined here: `new`, `same`, `duplicate` (another name already means it), `conflict` (that name means something else, and stays so) or `unsupported`, with the reason. Only one aggregate of one column is brought; a calculated column, a relationship, a view or any other expression is refused rather than approximated. A model maps to a record by `map`, or by name. What a reviewer accepts (a conflict only under another name) is created as drafts, all or none, with the importer's rights, to be certified like any other. Wren's own row and column rules are not brought: each reader's rights here decide, and the review says so.

### 🖥️ The certified metrics page

- **By domain**, domains in name order, "Other" last, each metric with its value for the viewer (money in the locale when the unit is a currency code, else a grouped number then the unit) and its lineage.
- **Says when it is stale**, naming the sources, and says why a metric has no value now, instead of seeming to load.
- **Certified datasets** with their warehouse addresses and download buttons.
- **New metric** and **New dataset** buttons; an empty state that points at metrics waiting for a steward.
- Loading shows skeletons shaped like what is coming; empty, denied, offline and failed are told apart.

### 🔐 Roles and permissions

| Role | What it can do |
|---|---|
| **Metric Author** | Defines metrics, datasets and glossary terms; reads External Sources; follows metrics. What they define is certified by someone else. |
| **Data Steward** | Certifies and deprecates metrics and datasets, keeps the glossary, describes External Sources (the rows and columns each reader sees, and freshness) |
| **Metric Reader** | Reads certified metrics and datasets with their own rights to the records, and loads them into a warehouse |

- Kept values are read by whoever read them; no role reads another's.
- A warehouse feed uses an API key and returns what that key's person may read.

### 🧾 Personal data

Requests reach who owns, certifies, reads and follows a metric, who owns and certifies a dataset, and who keeps a term. Erasure takes the address off; the records stay.

### 🌍 Languages and accessibility

Arabic and Hindi interface words and names, right-to-left pages, workspace-editable interface words, and loading, empty, denied, offline and failed states told apart.

## 🧭 How it works

```mermaid
flowchart LR
    A["Author defines<br/>query, formula or external"] --> D["Draft"]
    D -->|"Certify It<br/>by a steward, not the owner"| C["Certified"]
    C -->|"definition changed"| D
    C -->|"Deprecate It, names replacement"| X["Deprecated"]
    C --> R["Read with the reader's own rights"]
    R --> V["Value, series, kept value"]
    V --> F["Followed digests"]
    V --> W["Warehouse feed"]
    V --> O["Objectives, Intelligence"]
```

A metric moves from draft to certified, and a change of definition sends it back. Every read applies the reader's own rights, so the same certified definition gives each person the figure they may see.

### 🏭 Metrics' engine

```mermaid
flowchart LR
    S["Ferrum server<br/>Metrics"] -->|"model, query, this reader's rows and columns"| E["ferrum-metrics-engine<br/>Wren planning, DataFusion"]
    E --> F[("CSV and Parquet<br/>datasets")]
    E -->|"rows, when each dataset was written"| S
```

The engine is a service of its own beside the server. It plans the query through the metric's Wren model with Wren's engine (pinned) and runs it on DataFusion, so a heavy query cannot take the server down. The server never links it.

- **One SELECT only.** Definitions, changes, `SET`, `COPY`, several statements, a dataset named directly, a file path and a URL are refused, and so is anything Wren cannot plan exactly. Wren's own row and column rules are removed before a model runs: the rules are Ferrum's.
- **Bounded.** Time, memory (nothing spills), rows and concurrency are limited. Query terms nest at most 100 deep, at most 32 models are read, and a semantic model holds at most 32 models, all checked before anything is planned.
- **Sandboxed.** Each dataset belongs to one workspace. Metrics holds the engine's token. A reader is never told where the engine or the files are.
- **Run it:**

```bash
FERRUM_METRICS_ENGINE_CONFIG=/etc/ferrum/engine.json \
FERRUM_METRICS_ENGINE_TOKEN=<at least 32 characters> \
FERRUM_METRICS_ENGINE_ADDR=0.0.0.0:5180 \
ferrum-metrics-engine
```

The server needs `FERRUM_METRICS_ENGINE_URL` and the same token. The image is published from main by CI as `ghcr.io/cbferrum/ferrum-metrics-engine` (`latest` and `sha-<commit>`) after CI has run it read-only and unprivileged and asked it as Metrics does. For amounts, use Parquet with decimal columns: a CSV column of numbers is read as floating point. See [docs/engine.md](docs/engine.md).

## 🧱 Data model

| Entity | What it is | Key fields or states |
|---|---|---|
| **Metric** | A number defined once | name, domain, definition, formula, external, series by, convert to, unit, terms, lineage, status (Draft, Certified, Deprecated), certified by and at, replaced by |
| **Dataset** | A table defined once | name, domain, definition, columns, lineage, status, certified by and at, replaced by |
| **Business Term** (Glossary) | A word the business uses | term, definition, owner |
| **Metric Value** | A value as one person read it | metric, value, read by, read at, what it meant |
| **Followed Metric** | A metric delivered to a follower | metric, follower, frequency (Daily, Weekly, Monthly), active, last sent, last value, last error |
| **External Source** | A dataset the engine reads, and who sees what of it | name, fresh within (minutes), rows each reader sees, columns for some roles |
| Term | A small table linking a metric or dataset to glossary terms | term |

| Role | What it can do |
|---|---|
| Metric Author | Define metrics, datasets and terms |
| Data Steward | Certify, deprecate, keep the glossary and External Sources |
| Metric Reader | Read certified metrics and datasets, load them |

### 🛰️ Endpoints

| Endpoint | What it does |
|---|---|
| `GET /api/app/metrics/value?metric=` | A metric's value for the caller |
| `GET /api/app/metrics/series?metric=&bucket=&periods=` | A metric's value period by period |
| `GET /api/app/metrics/impact?metric=` | The metrics a change would reach |
| `GET /api/app/metrics/dataset?name=&format=csv\|ndjson` | A certified dataset for a warehouse |
| `GET /api/app/metrics/values?since=` | Kept values after a time |
| `POST /api/app/metrics/import/wren` | Review a Wren model into drafts |

Reports: none of its own. Print formats: none. Scheduled jobs: one, the delivery of followed metrics.

## 🔌 Integrations

| With | How | Needs |
|---|---|---|
| **Ferrum Objectives** | A key result is measured by a certified metric, read as Metrics gives it to the key result's owner | Both apps installed (the Objectives connector) |
| **Ferrum Intelligence** | Questions about certified metrics are answered through `/value` and `/series` as the asker | Both apps installed |
| **ERP and other apps** | A metric counts any record through Studio, with the reader's rights | Rights on the records |
| **Studio** | The report builder defines metrics and datasets | Part of the platform |
| **Wren** | Semantic models (MDL) are reviewed into drafts, and planned by Wren's engine | Wren model files |
| **Data lakes** | CSV or Parquet files read by Metrics' engine | The engine service and its token |
| **Warehouses and loaders** | Load datasets and values over HTTPS as CSV or JSON lines | An API key |
| **Notifications** | Followed-metric digests by the person's chosen channel | None |

## 📊 How it compares

Metrics is compared with Klipfolio (PowerMetrics), Geckoboard, Databox, Microsoft Power BI (its Metrics scorecards and shared datasets) and the open-source Metabase. Those are dashboard products with a metrics layer. Ferrum Metrics is a governed layer with a certified-metrics page. It has no dashboards and no charts of its own.

| Capability | Ferrum Metrics | Klipfolio | Geckoboard | Databox | Power BI | Metabase |
|---|:-:|:-:|:-:|:-:|:-:|:-:|
| Define a metric once, reuse everywhere | ✅ | ✅ | 🟡¹ | ✅ | ✅ | ✅ |
| Certification by a steward who does not own it | ✅ | 🟡¹ | ❌ | 🟡¹ | 🟡² | 🟡² |
| A changed definition sends it back to draft | ✅ | 🟡¹ | ❌ | 🟡¹ | 🟡¹ | 🟡¹ |
| Deprecate a metric and point at its replacement | ✅ | 🟡¹ | ❌ | 🟡¹ | 🟡¹ | 🟡¹ |
| Lineage of each metric | ✅ | 🟡¹ | ❌ | 🟡¹ | ✅ | 🟡¹ |
| Impact: what a change would reach | ✅ | 🟡¹ | ❌ | 🟡¹ | ✅ | 🟡¹ |
| Business glossary | ✅ | 🟡¹ | ❌ | ❌ | 🟡³ | 🟡¹ |
| Each reader sees figures by their own rights | ✅ | 🟡¹ | ❌ | 🟡¹ | ✅ | 🟡² |
| Metrics composed of other metrics | ✅ | 🟡¹ | 🟡¹ | ✅ | ✅ | ✅ |
| Currency conversion at each day's rate | ✅ | 🟡⁴ | 🟡⁴ | 🟡⁴ | 🟡⁴ | 🟡⁴ |
| Fiscal-year periods | ✅ | 🟡¹ | 🟡¹ | 🟡¹ | 🟡⁴ | 🟡¹ |
| Values over time (a series) | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Kept values with the definition they meant | ✅ | 🟡¹ | 🟡¹ | 🟡¹ | 🟡¹ | 🟡¹ |
| Stale data said on the figure | ✅ | 🟡¹ | 🟡¹ | 🟡¹ | 🟡¹ | 🟡¹ |
| Files in a data lake as governed sources | ✅ | 🟡¹ | 🟡¹ | 🟡¹ | ✅ | 🟡¹ |
| Load certified datasets into a warehouse | ✅ | ❌ | ❌ | ❌ | 🟡¹ | 🟡¹ |
| Followed metrics delivered on a schedule | ✅ | 🟡¹ | 🟡¹ | ✅ | ✅ | ✅ |
| Dashboards with charts | ❌ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Targets, goals and status | ❌ | ✅ | ✅ | ✅ | ✅ | 🟡¹ |
| Alerts when a value crosses a limit | ❌ | ✅ | 🟡¹ | ✅ | ✅ | ✅ |
| Many ready-made data connectors | ❌⁵ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Share, embed or show on a TV | ❌ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Mobile app | ❌ | 🟡¹ | 🟡¹ | ✅ | ✅ | 🟡¹ |
| Ask questions in plain words | 🟡⁶ | 🟡¹ | 🟡¹ | 🟡¹ | ✅ | 🟡¹ |
| Arabic and Hindi, right to left | ✅ | 🟡¹ | 🟡¹ | 🟡¹ | ✅ | 🟡¹ |
| Open source | ✅ | ❌ | ❌ | ❌ | ❌ | ✅ |
| Self-hosted | ✅ | ❌ | ❌ | ❌ | 🟡⁷ | ✅ |
| One data set with ERP, projects and HR records | ✅ | ➖ | ➖ | ➖ | ➖ | ➖ |
| No per-user or per-metric price | ✅ | ❌ | ❌ | ❌ | ❌ | 🟡⁸ |

<sub>¹ Present in some form or not confirmed; check the product. ² Certification or verification exists, but we could not confirm who may do it, or on which edition. ³ Glossaries belong to a separate catalogue product. ⁴ Possible by building it in the model or formulas; not a built-in rule. ⁵ Ferrum's records, and files through Metrics' engine. ⁶ Through the Intelligence app, over certified metrics only. ⁷ Power BI Report Server runs on your own servers with fewer features than the service. ⁸ The open-source edition is free to self-host; paid editions are priced per user.</sub>

### 🏆 Where Metrics stands out

- **Governance is the product.** Certification by a second person, a definition fingerprint, change control, deprecation with a replacement, lineage and impact are part of the record, not a convention.
- **Reads with the reader's own rights**, including on files in a data lake, with rows and columns applied in the engine's own scan.
- **Money and fiscal periods done exactly**: day rates, half-to-even rounding, refusal instead of a guess.
- **Stale is said**, and a kept value says what the metric meant when it was read.
- **A warehouse feed** with the fingerprint a loader can watch.
- **Open source, self-hosted, one data set** with the apps whose records it counts.

### 🚧 Where the leaders are still ahead

- **Dashboards and charts.** Metrics shows certified values, not charts or boards.
- **Targets, goals, status and alerts** when a value crosses a limit.
- **Connectors.** Hundreds of ready-made sources, against Ferrum's records and files.
- **Sharing and embedding**, TV mode and mobile apps.
- **Natural-language and AI features** across the whole product.

> [!NOTE]
> Leader columns are from public product information as of November 2026 and may change. A 🟡 for a leader means the feature exists in some form or we could not confirm the details.

## 📍 Roadmap

### ✅ Recently shipped

- [x] **0.7.4** Rebuilt on the tuned design system: better menu icons, a lighter first page, cleaner print.
- [x] **0.7.3** The redesigned interface on the upgraded design system, with shaped loading and told-apart states.
- [x] **0.7.2** A dataset's parse errors no longer reach readers with their raw rows; readers are not told where the engine or the files are.
- [x] **0.7.1** The page says when a value is stale and why a metric has no value; the engine bounds a query's depth and width before planning.
- [x] **0.7.0** Metrics read from external datasets through Metrics' engine (Wren on DataFusion), with each reader's rows and columns.
- [x] **0.6.0** Currencies by the day's rate, fiscal series, kept values that keep their meaning, and Wren models reviewed into drafts.

### 🔜 Next up

**Seeing and steering**
- [ ] **Targets and thresholds** on a metric, with a status (on track, at risk, off track).
- [ ] **Alerts** when a value crosses a target, to the people who follow it.
- [ ] **A chart on the metric page**: the series as a line or bars, with the partial period marked.
- [ ] **Boards of metrics**: a page that lays chosen certified metrics side by side for a team or a screen.

**Sources**
- [ ] **Series for external metrics.** They have a value today, not a series.
- [ ] **Push to a warehouse**, not only pull; datasets load whole each time, and only metric values load from where the last load stopped.
- [ ] **Remote databases** read by the engine, not only files.

### 🔭 Later

- [ ] **Free joins** between unrelated records. Datasets reach linked records along their links only.
- [ ] **Values kept on a schedule** for no one in particular, not only as each follower reads them.
- [ ] **Metric history view**: how a definition changed, who certified each version and what each version gave.
- [ ] **Share a certified metric** by a signed link or embed, with the viewer's rights.
- [ ] **Slack and Teams delivery** of followed metrics.
- [ ] **Benchmarks and forecasts** on a series.
- [ ] **Natural-language questions** beyond Intelligence's rules, over the glossary too.
- [ ] **Per-metric access rules** on who may read a metric at all, beside the records' rights.
- [ ] **More names in the glossary**: synonyms, and terms that point at several metrics.
- [ ] **A mobile view** of the certified metrics page tuned for phones and wall displays.

## 🚀 Install

Metrics is a native Ferrum app for platform `>=0.3.2, <0.4`.

1. In Ferrum, open the **App Manager** (or the store) and install **Metrics**. Self-hosters who build their own server image include the `metrics` app with the others.
2. Give roles: **Metric Author** to people who define, **Data Steward** to those who certify (a metric's owner can never certify it), **Metric Reader** to everyone who reads.
3. Open **Metrics**, choose **New metric**, build the query in the form, and save. Ask a steward to **Certify It**.
4. Add **Glossary** terms for the words the business uses and link them from metrics.
5. For metrics over files in a data lake: run the engine (see Metrics' engine above), set `FERRUM_METRICS_ENGINE_URL` and the token on the server, and add an **External Source** for each dataset with its freshness and who sees which rows and columns.
6. For a warehouse feed, give the loader an API key for a person with Metric Reader and use the dataset and values addresses.
7. Followers choose how they hear about **Metrics you follow** in their notification settings.

## 🔧 Develop

```bash
# an ordinary (non-superuser) role, so row-level security applies
createuser ferrum --pwprompt
createdb -O ferrum ferrum_metrics_test
createdb -O ferrum ferrum_metrics_datasets_test

export FERRUM_METRICS_TEST_DATABASE_URL=postgres://ferrum:ferrum@localhost:5432/ferrum_metrics_test
export FERRUM_METRICS_DATASETS_TEST_DATABASE_URL=postgres://ferrum:ferrum@localhost:5432/ferrum_metrics_datasets_test

cargo update -p ferrum-kernel          # Metrics has to keep working with Ferrum's main
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
RUST_MIN_STACK=8388608 cargo test --workspace   # the engine's tests need a deeper stack
```

The interface lives in `ui/` and builds with Ferrum-Frontend's packages (clone Ferrum-Frontend into `ferrum/web` next to this repository, as CI does):

```bash
cd ui
npm run build       # ui/dist, committed
npm run check
npm run typecheck
npm test
```

- **Tests.** `tests/metrics_it.rs` walks a sales team's metrics (definitions refused, certification refused to the author, three numbers from one definition, kept values, a changed definition back to draft, deprecation, who may do what). `tests/composed_it.rs`, `tests/currency_it.rs` (sales in four currencies against numbers worked out by hand), `tests/datasets_it.rs`, `tests/wren_it.rs` and `tests/external_it.rs` (the engine beside the kernel over HTTP) cover the rest. `engine/tests/` checks the engine alone: Wren's models answered exactly, what has no exact meaning refused, the sandbox, the bounds, the token, and a query as deep as one of the largest size refused before it is planned.
- **The engine image.** `docker build -f engine/Dockerfile .` or `cargo build --release --locked -p ferrum-metrics-engine`. CI builds it, runs it read-only and unprivileged, asks it, stops it with SIGTERM and, from main, publishes it.
- The tests also build on Ferrum's `ferrum-apps`. To work against a local platform checkout, patch the Ferrum crates in `.cargo/config.toml`.

## 📂 Project layout

```text
app.yaml                     the manifest: id, version, requirements, what it adds
packages/metrics/            entities, roles, module
src/metric.rs                definitions, certification, values, series, composition, currencies
src/dataset.rs               datasets
src/warehouse.rs             the warehouse feed
src/delivery.rs              followed metrics
src/wren.rs                  Wren import review
src/engine.rs                the client of the engine
src/site.rs                  the endpoints
engine/                      ferrum-metrics-engine: Wren planning on DataFusion, its Dockerfile and tests
ui/src/                      the certified metrics page and the words in Arabic and Hindi
docs/engine.md               how the engine works and how to run it
docs/PROVENANCE.md           where the design comes from
tests/                       integration tests against a real PostgreSQL
```

## 📜 Changelog

Recent releases are listed above; every version is in [CHANGELOG.md](CHANGELOG.md).

## 🤝 Contributing

Issues and pull requests are welcome at [github.com/CBFerrum/ferrum-app-metrics](https://github.com/CBFerrum/ferrum-app-metrics/issues). Please run the checks in Develop first. Read [docs/PROVENANCE.md](docs/PROVENANCE.md) before bringing in code from elsewhere.

## 📄 License

AGPL-3.0-only. See [LICENSE](LICENSE).

<div align="center">

<sub>Part of <a href="https://github.com/CBFerrum/Ferrum">Ferrum</a>, the open business platform · Made with 🦀 Rust and 🐘 PostgreSQL</sub>

</div>
