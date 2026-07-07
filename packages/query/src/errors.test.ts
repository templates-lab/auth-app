import { describe, expect, it } from "vitest";
import { ApiError, isUnauthorized, unwrap, type FetchResult } from "./errors";

/** Build a fake openapi-fetch result for a given status/body. */
function result<T>(status: number, data?: T, error?: unknown): Promise<FetchResult<T>> {
  return Promise.resolve({
    data,
    error,
    response: { ok: status >= 200 && status < 300, status } as Response,
  });
}

describe("unwrap", () => {
  it("returns the data of a successful response", async () => {
    await expect(unwrap(result(200, { id: "1" }))).resolves.toEqual({ id: "1" });
  });

  it("resolves to undefined for an empty-body success (e.g. 204)", async () => {
    await expect(unwrap(result<void>(204))).resolves.toBeUndefined();
  });

  it("throws an ApiError carrying the status on a failed response", async () => {
    await expect(unwrap(result(404, undefined, { message: "not found" }))).rejects.toMatchObject({
      status: 404,
      message: "not found",
    });
  });

  it("falls back to a status message when the body has none", async () => {
    await expect(unwrap(result(500))).rejects.toMatchObject({
      status: 500,
      message: "Request failed with status 500",
    });
  });
});

describe("isUnauthorized", () => {
  it("is true only for a 401 ApiError", () => {
    expect(isUnauthorized(new ApiError(401, "no"))).toBe(true);
    expect(isUnauthorized(new ApiError(403, "no"))).toBe(false);
    expect(isUnauthorized(new Error("401"))).toBe(false);
    expect(isUnauthorized(undefined)).toBe(false);
  });
});
