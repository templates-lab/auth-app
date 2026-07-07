import { afterEach, describe, expect, it, vi } from "vitest";
import { redirectToLoginOnUnauthorized } from "./unauthorized";

/** Install a stub `window.location` and return the `assign` spy. */
function stubLocation() {
  const assign = vi.fn();
  vi.stubGlobal("window", { location: { assign } });
  return assign;
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("redirectToLoginOnUnauthorized", () => {
  it("redirects to /login by default", () => {
    const assign = stubLocation();
    redirectToLoginOnUnauthorized()();
    expect(assign).toHaveBeenCalledWith("/login");
  });

  it("honours a custom login path", () => {
    const assign = stubLocation();
    redirectToLoginOnUnauthorized({ loginPath: "/sign-in" })();
    expect(assign).toHaveBeenCalledWith("/sign-in");
  });

  it("redirects after a successful logout completes", async () => {
    const assign = stubLocation();
    const logout = vi.fn().mockResolvedValue(undefined);
    redirectToLoginOnUnauthorized({ logout })();

    expect(logout).toHaveBeenCalledOnce();
    await vi.waitFor(() => expect(assign).toHaveBeenCalledWith("/login"));
  });

  it("still redirects when logout rejects", async () => {
    const assign = stubLocation();
    const logout = vi.fn().mockRejectedValue(new Error("offline"));
    redirectToLoginOnUnauthorized({ logout })();

    await vi.waitFor(() => expect(assign).toHaveBeenCalledWith("/login"));
  });
});
