import { describe, expect, it } from "vitest";
import { defineFeature, type FeatureModule } from "@auth-app/feature-kit";
import { collectNav, collectRoutePaths, duplicateRoutePaths } from "./compose";

const noop = () => null;

const alpha: FeatureModule = defineFeature({
  id: "alpha",
  title: "Alpha",
  nav: [{ path: "/alpha", label: "Alpha", order: 20 }],
  routes: [{ path: "/alpha", component: noop }],
});

const beta: FeatureModule = defineFeature({
  id: "beta",
  title: "Beta",
  nav: [{ path: "/", label: "Home", order: 10 }],
  routes: [{ path: "/", component: noop, children: [{ path: "/nested", component: noop }] }],
});

const headless: FeatureModule = defineFeature({
  id: "headless",
  title: "Headless",
  routes: [{ path: "/headless", component: noop }],
});

describe("collectNav", () => {
  it("merges and orders sidebar entries by weight", () => {
    expect(collectNav([alpha, beta]).map((item) => item.label)).toEqual(["Home", "Alpha"]);
  });

  it("skips features that contribute no nav entries", () => {
    expect(collectNav([headless])).toEqual([]);
  });

  it("falls back to declaration order when weights tie", () => {
    const a = defineFeature({ id: "a", title: "A", nav: [{ path: "/a", label: "A" }], routes: [] });
    const b = defineFeature({ id: "b", title: "B", nav: [{ path: "/b", label: "B" }], routes: [] });
    expect(collectNav([b, a]).map((item) => item.label)).toEqual(["B", "A"]);
  });
});

describe("collectRoutePaths", () => {
  it("flattens paths across features and nested children", () => {
    expect(collectRoutePaths([alpha, beta])).toEqual(["/alpha", "/", "/nested"]);
  });
});

describe("duplicateRoutePaths", () => {
  it("reports nothing when features own disjoint paths", () => {
    expect(duplicateRoutePaths([alpha, beta, headless])).toEqual([]);
  });

  it("detects a collision between two features", () => {
    const clash = defineFeature({
      id: "clash",
      title: "Clash",
      routes: [{ path: "/alpha", component: noop }],
    });
    expect(duplicateRoutePaths([alpha, clash])).toEqual(["/alpha"]);
  });
});
