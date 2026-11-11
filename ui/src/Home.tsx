// The certified metrics by domain, each with its value for the viewer (read with their own rights)
// and its lineage, then the certified datasets with their addresses for a warehouse to load. Each
// opens its record. A value read from external data older than its source allows says so, and a
// metric without a value now says why, rather than seeming to load.

import { useQueries, useQuery } from "@tanstack/react-query";
import { Gauge } from "lucide-react";
import { api, docPath, formatDecimal, t, isolate } from "@ferrum/client";
import { ButtonLink, HomeSkeleton, LoadFailed, PageHead, PageStack, Queue, QueueGrid, QueueItem, StatePanel, useShellTitle, placed } from "@ferrum/kit";

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

/** A value as a person reads it: money in the locale when `unit` is a currency code, else a grouped number then the unit. */
export function valueText(value: unknown, unit: unknown, locale?: string): string {
  const v = String(value ?? "").trim();
  const u = String(unit ?? "").trim();
  if (!/^-?\d+(\.\d+)?$/.test(v)) return u ? `${v} ${u}` : v;
  if (/^[A-Z]{3}$/.test(u)) {
    try {
      return new Intl.NumberFormat(locale, { style: "currency", currency: u }).format(v as Intl.StringNumericLiteral);
    } catch {
      // a code the browser does not know reads as a number then the code
    }
  }
  const n = formatDecimal(v, undefined, locale);
  return u ? `${n} ${u}` : n;
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
  const head = (
    <PageHead
      title={t("Certified metrics")}
      actions={
        <>
          <ButtonLink to="/e/Dataset/new">{t("New dataset")}</ButtonLink>
          <ButtonLink to="/e/Metric/new" variant="primary" icon="plus">
            {t("New metric")}
          </ButtonLink>
        </>
      }
    />
  );
  if (q.isPending) return <PageStack>{head}<HomeSkeleton /></PageStack>;
  if (q.isError) return <PageStack>{head}<LoadFailed error={q.error} onRetry={() => void q.refetch()} /></PageStack>;
  const readOf = new Map((q.data ?? []).map((m, i) => [m.id, values[i]]));
  return (
    <PageStack>
      {head}
        {q.data.length === 0 && (
          <StatePanel kind="empty" icon={Gauge} title={t("No metric is certified yet")} action={<ButtonLink to="/e/Metric">{t("Metrics in draft")}</ButtonLink>}>
            {t("Metrics in draft wait for a Data Steward to certify them.")}
          </StatePanel>
        )}
        <QueueGrid>
          {byDomain(q.data).map(([domain, list]) => (
            <Queue key={domain} title={isolate(domain)} count={list.length}>
              {list.map((m) => {
                const read = readOf.get(m.id);
                const stale = staleSources(read?.data);
                const notes = [String(m.uses ?? "") && <span key="u">{isolate(String(m.uses))}</span>, read?.isError && <span key="e" role="status">{placed(t("No value now: {reason}"), { reason: isolate(reasonOf(String(m.metric_name), read.error)) })}</span>, stale.length > 0 && <span key="s" role="status">{placed(t("Read from data older than its source allows: {sources}"), { sources: isolate(stale.join(", ")) })}</span>].filter(Boolean);
                return (
                  <QueueItem
                    key={m.id}
                    to={`/e/Metric/${m.id}`}
                    title={isolate(String(m.metric_name))}
                    meta={notes.length ? <span className="flex flex-col gap-0.5 whitespace-normal">{notes}</span> : undefined}
                    trailing={
                      <span className="num text-base font-semibold">{read?.isError ? "—" : read?.data ? isolate(valueText(read.data.value, m.unit)) : "…"}</span>
                    }
                  />
                );
              })}
            </Queue>
          ))}
          {sets.data && (
            <div className="flex flex-col gap-2">
              <Queue title={t("Certified datasets")} count={sets.data.length} empty={t("No dataset is certified yet.")}>
                {sets.data.map((d) => (
                  <QueueItem
                    key={d.id}
                    to={`/e/Dataset/${d.id}`}
                    title={isolate(String(d.dataset_name))}
                    meta={d.uses ? String(d.uses) : undefined}
                    action={
                      <span className="flex gap-1">
                        <ButtonLink href={loadUrl(String(d.dataset_name), "csv")} download size="sm">{t("CSV")}</ButtonLink>
                        <ButtonLink href={loadUrl(String(d.dataset_name), "ndjson")} download size="sm">{t("JSON lines")}</ButtonLink>
                      </span>
                    }
                  />
                ))}
              </Queue>
              <p className="px-1 text-xs text-muted-foreground">{t("A warehouse loads these addresses with an API key: it reads what that key's person may read.")}</p>
            </div>
          )}
        </QueueGrid>
    </PageStack>
  );
}
