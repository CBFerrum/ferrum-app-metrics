// The certified metrics by domain, each with its value for the viewer (read with their own rights)
// and its lineage, then the certified datasets with their addresses for a warehouse to load. Each
// opens its record.

import { useQueries, useQuery } from "@tanstack/react-query";
import { Link } from "react-router";
import { api, docPath, t } from "@ferrum/client";
import { ErrorState, PageLoading, useShellTitle } from "@ferrum/kit";

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
  useShellTitle("Metrics");
  const q = useQuery({ queryKey: ["metrics", "certified"], queryFn: () => rows("Metric", [["status", "eq", "Certified"]], { sort: "metric_name asc" }) });
  const sets = useQuery({ queryKey: ["metrics", "datasets"], queryFn: () => rows("Dataset", [["status", "eq", "Certified"]], { sort: "dataset_name asc" }) });
  const values = useQueries({
    queries: (q.data ?? []).map((m) => ({
      queryKey: ["metrics", "value", m.metric_name],
      queryFn: () => api.get<{ value: string }>("/api/app/metrics/value", { metric: String(m.metric_name) }).then((r) => r.value),
    })),
  });
  if (q.isPending) return <PageLoading />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => void q.refetch()} />;
  const valueOf = new Map((q.data ?? []).map((m, i) => [m.id, values[i]?.data]));
  return (
    <div className="np-page">
      <header className="row" style={{ justifyContent: "space-between" }}>
        <h2>{t("Certified metrics")}</h2>
        <Link to="/e/Metric/new">{t("New metric")}</Link>
      </header>
      {q.data.length === 0 && (
        <p className="faint">
          {t("No metric is certified yet.")} <Link to="/e/Metric">{t("Metrics in draft")}</Link> {t("wait for a Data Steward.")}
        </p>
      )}
      {byDomain(q.data).map(([domain, list]) => (
        <section key={domain} className="card card-pad stack" aria-label={domain}>
          <h3>{domain}</h3>
          <div className="np-columns">
            {list.map((m) => (
              <Link key={m.id} className="card card-pad stack" to={`/e/Metric/${m.id}`}>
                <strong>{String(m.metric_name)}</strong>
                <span style={{ fontSize: 28 }}>
                  {valueOf.get(m.id) ?? "…"} {String(m.unit ?? "")}
                </span>
                <span className="faint">{String(m.uses ?? "")}</span>
              </Link>
            ))}
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
                <Link to={`/e/Dataset/${d.id}`}>{String(d.dataset_name)}</Link> ·{" "}
                <a href={loadUrl(String(d.dataset_name), "csv")} download>
                  {t("CSV")}
                </a>{" "}
                ·{" "}
                <a href={loadUrl(String(d.dataset_name), "ndjson")} download>
                  {t("JSON lines")}
                </a>
              </span>
              <span className="faint">{String(d.uses ?? "")}</span>
            </li>
          ))}
        </ul>
        <p className="faint">{t("A warehouse loads these addresses with an API key: it reads what that key's person may read.")}</p>
      </section>
    </div>
  );
}
