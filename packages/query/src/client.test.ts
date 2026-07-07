import { describe, expect, it, vi } from "vitest";
import { createAppQueryClient } from "./client";
import { createFeatureKeys } from "./keys";
import { ApiError } from "./errors";

/** Read the default query `retry` predicate off a built client. */
function retryPredicate(client: ReturnType<typeof createAppQueryClient>) {
  const retry = client.getDefaultOptions().queries?.retry;
  if (typeof retry !== "function") {
    throw new Error("expected a retry predicate");
  }
  return retry as (failureCount: number, error: Error) => boolean;
}

describe("createAppQueryClient — 401 interceptor", () => {
  it("calls onUnauthorized once even when several requests fail with 401", async () => {
    const onUnauthorized = vi.fn();
    const client = createAppQueryClient({ onUnauthorized });

    await Promise.all([
      client
        .fetchQuery({
          queryKey: ["a"],
          queryFn: () => Promise.reject(new ApiError(401, "no")),
        })
        .catch(() => undefined),
      client
        .fetchQuery({
          queryKey: ["b"],
          queryFn: () => Promise.reject(new ApiError(401, "no")),
        })
        .catch(() => undefined),
    ]);

    expect(onUnauthorized).toHaveBeenCalledTimes(1);
  });

  it("ignores non-401 failures", async () => {
    const onUnauthorized = vi.fn();
    const client = createAppQueryClient({ onUnauthorized });

    await client
      .fetchQuery({
        queryKey: ["a"],
        queryFn: () => Promise.reject(new ApiError(500, "boom")),
        retry: false,
      })
      .catch(() => undefined);

    expect(onUnauthorized).not.toHaveBeenCalled();
  });
});

describe("createAppQueryClient — retry policy", () => {
  it("never retries a 401", () => {
    const retry = retryPredicate(createAppQueryClient());
    expect(retry(0, new ApiError(401, "no"))).toBe(false);
  });

  it("retries transient failures up to the cap", () => {
    const retry = retryPredicate(createAppQueryClient());
    expect(retry(0, new Error("network"))).toBe(true);
    expect(retry(1, new Error("network"))).toBe(true);
    expect(retry(2, new Error("network"))).toBe(false);
  });

  it("does not retry mutations by default", () => {
    const client = createAppQueryClient();
    expect(client.getDefaultOptions().mutations?.retry).toBe(false);
  });
});

describe("createAppQueryClient — invalidation cascades to affected queries", () => {
  it("invalidates a feature's queries by prefix without touching others", async () => {
    const client = createAppQueryClient();
    const users = createFeatureKeys("users");
    const orders = createFeatureKeys("orders");

    client.setQueryData(users.list(), [{ id: "1" }]);
    client.setQueryData(orders.list(), [{ id: "9" }]);

    // A users mutation invalidates everything under the users namespace.
    await client.invalidateQueries({ queryKey: users.all });

    expect(client.getQueryState(users.list())?.isInvalidated).toBe(true);
    expect(client.getQueryState(orders.list())?.isInvalidated).toBe(false);
  });
});
