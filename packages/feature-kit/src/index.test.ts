import { describe, expect, it } from "vitest";
import { DEFAULT_NAV_ORDER, defineFeature, type FeatureModule } from "./index";

describe("defineFeature", () => {
  it("returns the module unchanged (identity)", () => {
    const feature: FeatureModule = {
      id: "sample",
      title: "Sample",
      routes: [{ path: "/sample", component: () => null }],
    };
    expect(defineFeature(feature)).toBe(feature);
  });
});

describe("DEFAULT_NAV_ORDER", () => {
  it("is a stable numeric fallback weight", () => {
    expect(DEFAULT_NAV_ORDER).toBe(100);
  });
});
