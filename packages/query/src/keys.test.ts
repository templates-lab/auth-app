import { describe, expect, it } from "vitest";
import { createFeatureKeys } from "./keys";

describe("createFeatureKeys", () => {
  const users = createFeatureKeys("users");

  it("roots every key at the feature id", () => {
    expect(users.all).toEqual(["users"]);
    expect(users.lists()).toEqual(["users", "list"]);
    expect(users.details()).toEqual(["users", "detail"]);
  });

  it("narrows a list by filter params while keeping the list prefix", () => {
    expect(users.list()).toEqual(["users", "list"]);
    expect(users.list({ role: "admin" })).toEqual(["users", "list", { role: "admin" }]);
  });

  it("keys a single entity by id under the detail prefix", () => {
    expect(users.detail("42")).toEqual(["users", "detail", "42"]);
    expect(users.detail(7)).toEqual(["users", "detail", 7]);
  });

  it("keeps broader keys as a prefix of narrower ones so invalidation cascades", () => {
    // `invalidateQueries({ queryKey: users.all })` matches by prefix, so it must
    // be a leading slice of both a list and a detail key.
    const detail = users.detail("42");
    const list = users.list({ role: "admin" });
    expect(detail.slice(0, users.all.length)).toEqual([...users.all]);
    expect(list.slice(0, users.lists().length)).toEqual([...users.lists()]);
  });

  it("namespaces distinct features so their keys never collide", () => {
    const orders = createFeatureKeys("orders");
    expect(orders.all).not.toEqual(users.all);
  });
});
