import { describe, expect, it } from "vitest";
import { byDomain, loadUrl, staleSources } from "./Home";

describe("metrics by domain", () => {
  it("groups by domain in name order, other last", () => {
    const g = byDomain([
      { id: "a", domain: "Sales" },
      { id: "b" },
      { id: "c", domain: "Finance" },
      { id: "d", domain: "Sales" },
    ]);
    expect(g.map(([d, l]) => [d, l.map((m) => m.id)])).toEqual([
      ["Finance", ["c"]],
      ["Sales", ["a", "d"]],
      ["Other", ["b"]],
    ]);
  });
});

describe("datasets", () => {
  it("are loaded from an address naming them and the format", () => {
    expect(loadUrl("Customers by group", "csv")).toBe("/api/app/metrics/dataset?name=Customers+by+group&format=csv");
    expect(loadUrl("a&b", "ndjson")).toBe("/api/app/metrics/dataset?name=a%26b&format=ndjson");
  });
});

describe("stale values", () => {
  it("name the sources older than they allow, each once", () => {
    expect(staleSources(undefined)).toEqual([]);
    expect(staleSources({ value: "150" })).toEqual([]);
    expect(staleSources({ value: "150", stale: [] })).toEqual([]);
    expect(staleSources({ value: "75", stale: [{ source: "returns_lake" }, { source: "orders_lake" }, { source: "orders_lake" }] })).toEqual(["orders_lake", "returns_lake"]);
  });
});
