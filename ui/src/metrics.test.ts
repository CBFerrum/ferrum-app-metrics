import { describe, expect, it } from "vitest";
import { byDomain } from "./Home";

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
