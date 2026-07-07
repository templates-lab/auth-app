import { describe, expect, it } from "vitest";
import { createApiClient } from "./index";

describe("createApiClient", () => {
  it("exposes the typed HTTP verb methods", () => {
    const client = createApiClient();
    // openapi-fetch clients carry a method per verb, each typed by the schema.
    expect(typeof client.GET).toBe("function");
    expect(typeof client.POST).toBe("function");
  });

  it("accepts an override base URL", () => {
    // A smoke check that construction with options does not throw; the URL
    // itself is exercised by real requests in the features that consume it.
    expect(() => createApiClient({ baseUrl: "https://api.example.com" })).not.toThrow();
  });
});
