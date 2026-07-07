import { afterEach, describe, expect, it, vi } from "vitest";
import { redirectToLoginOnUnauthorized } from "./unauthorized";

/** Install a stub `window.location` and return the `assign` spy. */
function stubLocation(pathname = "/", search = "") {
  const assign = vi.fn();
  vi.stubGlobal("window", { location: { assign, pathname, search } });
  return assign;
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("redirectToLoginOnUnauthorized", () => {
  it("redirects to /login preserving the current path as ?next", () => {
    const assign = stubLocation("/transactions", "?status=captured");
    redirectToLoginOnUnauthorized()();
    expect(assign).toHaveBeenCalledWith(
      `/login?next=${encodeURIComponent("/transactions?status=captured")}`,
    );
  });

  it("does not stack a next when already on the login page", () => {
    const assign = stubLocation("/login", "");
    redirectToLoginOnUnauthorized()();
    expect(assign).toHaveBeenCalledWith("/login");
  });

  it("honours a custom login path", () => {
    const assign = stubLocation("/", "");
    redirectToLoginOnUnauthorized({ loginPath: "/sign-in" })();
    expect(assign).toHaveBeenCalledWith(`/sign-in?next=${encodeURIComponent("/")}`);
  });

  it("redirects after a successful logout completes", async () => {
    const assign = stubLocation("/users", "");
    const logout = vi.fn().mockResolvedValue(undefined);
    redirectToLoginOnUnauthorized({ logout })();

    expect(logout).toHaveBeenCalledOnce();
    await vi.waitFor(() =>
      expect(assign).toHaveBeenCalledWith(`/login?next=${encodeURIComponent("/users")}`),
    );
  });

  it("still redirects when logout rejects", async () => {
    const assign = stubLocation("/users", "");
    const logout = vi.fn().mockRejectedValue(new Error("offline"));
    redirectToLoginOnUnauthorized({ logout })();

    await vi.waitFor(() =>
      expect(assign).toHaveBeenCalledWith(`/login?next=${encodeURIComponent("/users")}`),
    );
  });
});
