import { describe, expect, it } from "vitest";
import { normalizeEmail } from "./format";

describe("normalizeEmail", () => {
  it("trims surrounding whitespace and lower-cases", () => {
    expect(normalizeEmail("  User@Example.COM ")).toBe("user@example.com");
  });
});
