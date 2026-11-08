# Metrics' engine for external sources

Some numbers live outside Ferrum's records: in a data lake's CSV or Parquet files. A metric over them is an
**external metric**. It has a Wren semantic model (MDL) over the lake's datasets and one query over its models.
**Metrics' engine** (`engine/`, the binary `ferrum-metrics-engine`) answers that query. It plans the query
through the model with Wren's own engine (`wren-semantic-core` 0.5.1, pinned) and runs it on Apache DataFusion
55, as a service of its own beside the server. A heavy query is the engine's to bound, and cannot take the
server down with it. The server never links DataFusion.

## How a value is read

1. A reader asks Metrics for an external metric's value (`/api/app/metrics/value`, a subscription, Intelligence,
   Objectives).
2. Metrics looks up the **External Source** each dataset of the metric's model is described by. A Data Steward
   describes each one in Ferrum: which rows and columns each reader sees, and how fresh its data must be.
3. Metrics asks the engine for the reader alone, sending:
   - the model;
   - the query;
   - for each dataset, the rows this reader may see (a column among values) and the columns hidden from them.
4. The engine reads each dataset through a table that filters its rows and leaves out those columns in its own
   scan. Its files are never registered under a name a query could use. It plans the query through the model,
   runs it, and says when each dataset it read was last written.
5. Metrics returns the value. When a dataset's data is older than its source allows, or of no known age, the
   value carries it under `stale`, and Intelligence's answers say so in their `limits`.

A composed metric made of an external metric and a metric of the records is read the same way, part by part.
It is stale when any part is, and has no value when any part has none.

## What is refused, and why

- **Anything but one SELECT** over the model's own models. Definitions, changes, `SET`, `COPY`, several
  statements, a dataset named directly, a file path and a URL are all refused. On top of that the planned SQL
  runs with DDL, DML and statements disabled.
- **Anything Wren cannot plan exactly.** An unknown function, a dialect's own syntax (`TOP`), or a column the
  dataset does not have fails with its reason, never as an approximation. A correlated subquery is answered
  exactly or refused.
- **A model that reads SQL or another object of its own** (`refSql`, `baseObject`), names a dataset with a
  catalog or schema, or shares its name with a dataset.
- **Wren's own row and column rules** (`rowLevelAccessControls`, `columnLevelAccessControl`) and its
  `dataSource`. These are taken out before a model runs: the rules are Ferrum's, and the data is read here.
- **Too deep or too wide to plan quickly.** Planning cannot be stopped midway, so before anything is planned a
  query is held to terms nested at most 100 deep (`a + b + c` is three deep) and at most 32 models read,
  counting each time one is read. A model's calculated columns and relationships are held to the same depth,
  and a semantic model holds at most 32 models, so the longest chain of relationships through it is planned in
  moments.
  A long list of values is fine. Each of the engine's threads has a 64 MiB stack, used only as deep as a query
  goes, and DataFusion's own recursion grows its stack as it needs: no query can overflow it and take the
  process down.
- **Too much.** Each query runs within the operator's limits. When one is exceeded, nothing is answered:
  - rows: the answer is cut and says so;
  - time: the caller is told so (504) once its wait for a turn and its time are up, whether or not the query's
    work stops to yield. Each query runs as a task of its own, stopped at its next yield, and it keeps its turn
    until it has stopped, so slow queries never hold more than the turns there are. A query that fails
    unexpectedly is refused (500), and the engine goes on;
  - memory: a query needing more fails, since nothing spills to disk (422);
  - concurrency: past the queries allowed at once, a query waits its turn, then is told the engine is busy (503).

## Running it

```bash
FERRUM_METRICS_ENGINE_CONFIG=/etc/ferrum/engine.json \
FERRUM_METRICS_ENGINE_TOKEN=<at least 32 characters> \
FERRUM_METRICS_ENGINE_ADDR=0.0.0.0:5180 \
ferrum-metrics-engine
```

The configuration lists the datasets and the limits:

```json
{
  "datasets": [
    {"name": "orders_lake", "workspace": "acme", "format": "parquet", "path": "/data/acme/orders"},
    {"name": "returns_lake", "workspace": "acme", "format": "csv", "path": "/data/acme/returns.csv"}
  ],
  "limits": {"rows": 10000, "millis": 10000, "memory_mb": 512, "concurrent": 4, "wait_millis": 2000}
}
```

- **Datasets.** A dataset belongs to one workspace, by slug or by id, and no other workspace reads it. Two
  workspaces may each have a dataset of the same name. A path is absolute: one file, or a directory of files
  of that format. When it was written is the newest file's time. A dataset missing at start is reported, and
  a query of it then says it cannot be read.
- **The server** needs `FERRUM_METRICS_ENGINE_URL` (for example `http://metrics-engine:5180`, a private
  address) and the same `FERRUM_METRICS_ENGINE_TOKEN`. Without them, an external metric is refused when saved
  and has no value when read, saying the engine is not set up.
- **Health.** `GET /health` answers `{"ok": true, "datasets": n}` without a token. On SIGTERM the engine stops
  taking work and finishes what it holds.
- **Image.** Published from main as `ghcr.io/cbferrum/ferrum-metrics-engine` (`latest` and `sha-<commit>`),
  once CI has built it, run it read-only and unprivileged, asked it as Metrics does, and stopped it with
  SIGTERM. Ferrum's `deploy/metrics-engine.yml` runs it beside the stack with Compose, and its
  `deploy/k8s/metrics-engine` component runs two replicas on Kubernetes.
- **Build.** `docker build -f engine/Dockerfile .` from this repository, or `cargo build --release --locked -p
  ferrum-metrics-engine`.

## Numbers and money

DataFusion reads a CSV column of numbers as floating point. Parquet keeps decimals exact. For amounts, use
Parquet with decimal columns, or cast in the model (`CAST(total AS DECIMAL(18,2))`). An empty sum is 0, as it
is for a metric of the records.

## Limits of this version

- External metrics have no series yet.
- Datasets are files. A warehouse is read once it exports to them; no query is pushed down to a remote database.
