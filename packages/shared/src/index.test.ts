import { describe, expect, it } from "vitest";
import { isValidEmail, userLabel, type AuthUser } from "./index";

describe("isValidEmail", () => {
  it("accepts a well-formed address", () => {
    expect(isValidEmail("user@example.com")).toBe(true);
  });

  it("rejects a malformed address", () => {
    expect(isValidEmail("not-an-email")).toBe(false);
  });
});

describe("userLabel", () => {
  const base: AuthUser = { id: "1", email: "user@example.com" };

  it("prefers the display name when present", () => {
    expect(userLabel({ ...base, displayName: "Ada" })).toBe("Ada");
  });

  it("falls back to the email", () => {
    expect(userLabel(base)).toBe("user@example.com");
  });
});
