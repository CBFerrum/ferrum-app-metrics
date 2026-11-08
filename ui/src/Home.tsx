// The certified metrics by domain, each with its value for the viewer (read with their own rights)
// and its lineage, then the certified datasets with their addresses for a warehouse to load. Each
// opens its record. A value read from external data older than its source allows says so, and a
// metric without a value now says why, rather than seeming to load.

import { useQueries, useQuery } from "@tanstack/react-query";
import { Link } from "react-router";
import { api, docPath, t, isolate } from "@ferrum/client";
import { ErrorState, PageLoading, useShellTitle, placed } from "@ferrum/kit";

export interface Row {
  id: string;
  [k: string]: unknown;
}

const rows = (entity: string, filters: unknown[], extra: Record<string, string | number> = {}) =>
  api.get<{ rows: Row[] }>(docPath(entity), { filters: JSON.stringify(filters), limit: 500, ...extra }).then((r) => r.rows);

/** A certified dataset's address for a warehouse (or a download), in `format`. */
export function loadUrl(name: string, format: "csv" | "ndjson"): string {
  return `/api/app/metrics/dataset?${new URLSearchParams({ name, format }).toString()}`;
}

/** A metric's value as Metrics gives it: the external sources it was read from that are stale among it. */
export interface Read {
  value: string;
  stale?: { source: string }[];
}

/** The sources a value was read from whose data is older than they allow, each once, in name order. */
export function staleSources(read: Read | undefined): string[] {
  return [...new Set((read?.stale ?? []).map((s) => s.source).filter(Boolean))].sort();
}

/** Why `metric` has no value now, without the metric's own name, which its card already shows. */
export function reasonOf(metric: string, error: unknown): string {
  const said = error instanceof Error ? error.message : String(error);
  const prefix = [`${metric} has no value now: `, `${metric} has no value for you: `, `${metric} has no value: `].find((p) => said.startsWith(p));
  return prefix ? said.slice(prefix.length) : said;
}

/** Metrics by domain, domains in name order, "Other" last. */
export function byDomain(list: Row[]): [string, Row[]][] {
  const out = new Map<string, Row[]>();
  for (const m of list) {
    const d = typeof m.domain === "string" && m.domain ? m.domain : "Other";
    out.set(d, [...(out.get(d) ?? []), m]);
  }
  return [...out.entries()].sort(([a], [b]) => (a === "Other" ? 1 : b === "Other" ? -1 : a.localeCompare(b)));
}

export function Home() {
  useShellTitle(t("Metrics"));
  const q = useQuery({ queryKey: ["metrics", "certified"], queryFn: () => rows("Metric", [["status", "eq", "Certified"]], { sort: "metric_name asc" }) });
  const sets = useQuery({ queryKey: ["metrics", "datasets"], queryFn: () => rows("Dataset", [["status", "eq", "Certified"]], { sort: "dataset_name asc" }) });
  const values = useQueries({
    queries: (q.data ?? []).map((m) => ({
      queryKey: ["metrics", "value", m.metric_name],
      queryFn: () => api.get<Read>("/api/app/metrics/value", { metric: String(m.metric_name) }),
      retry: false,
    })),
  });
  if (q.isPending) return <PageLoading />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => void q.refetch()} />;
  const readOf = new Map((q.data ?? []).map((m, i) => [m.id, values[i]]));
  return (
    <div className="np-page">
      <header className="row" style={{ justifyContent: "space-between" }}>
        <h2>{t("Certified metrics")}</h2>
        <Link to="/e/Metric/new">{t("New metric")}</Link>
      </header>
      {q.data.length === 0 && (
        <p className="faint">
          {placed(t("No metric is certified yet. {p1} wait for a Data Steward."), { p1: <Link to="/e/Metric">{t("Metrics in draft")}</Link> })}
        </p>
      )}
      {byDomain(q.data).map(([domain, list]) => (
        <section key={domain} className="card card-pad stack" aria-label={domain}>
          <h3>{isolate(domain)}</h3>
          <div className="np-columns">
            {list.map((m) => {
              const read = readOf.get(m.id);
              const stale = staleSources(read?.data);
              return (
                <Link key={m.id} className="card card-pad stack" to={`/e/Metric/${m.id}`}>
                  <strong>{isolate(String(m.metric_name))}</strong>
                  <span style={{ fontSize: 28 }}>
                    {read?.isError ? "—" : (read?.data?.value ?? "…")} {isolate(String(m.unit ?? ""))}
                  </span>
                  {read?.isError && (
                    <span className="faint" role="status">
                      {placed(t("No value now: {reason}"), { reason: isolate(reasonOf(String(m.metric_name), read.error)) })}
                    </span>
                  )}
                  {stale.length > 0 && (
                    <span className="faint" role="status">
                      {placed(t("Read from data older than its source allows: {sources}"), { sources: isolate(stale.join(", ")) })}
                    </span>
                  )}
                  <span className="faint">{isolate(String(m.uses ?? ""))}</span>
                </Link>
              );
            })}
          </div>
        </section>
      ))}
      <section className="card card-pad stack" aria-label={t("Datasets")}>
        <header className="row" style={{ justifyContent: "space-between" }}>
          <h3>{t("Certified datasets")}</h3>
          <Link to="/e/Dataset/new">{t("New dataset")}</Link>
        </header>
        {(sets.data ?? []).length === 0 && <p className="faint">{t("No dataset is certified yet.")}</p>}
        <ul className="stack">
          {(sets.data ?? []).map((d) => (
            <li key={d.id} className="stack" style={{ gap: 2 }}>
              <span>
                <Link to={`/e/Dataset/${d.id}`}>{isolate(String(d.dataset_name))}</Link> ·{isolate(" ")}
                <a href={loadUrl(String(d.dataset_name), "csv")} download>
                  {t("CSV")}
                </a>{isolate(" ")}
                ·{isolate(" ")}
                <a href={loadUrl(String(d.dataset_name), "ndjson")} download>
                  {t("JSON lines")}
                </a>
              </span>
              <span className="faint">{isolate(String(d.uses ?? ""))}</span>
            </li>
          ))}
        </ul>
        <p className="faint">{t("A warehouse loads these addresses with an API key: it reads what that key's person may read.")}</p>
      </section>
    </div>
  );
}
