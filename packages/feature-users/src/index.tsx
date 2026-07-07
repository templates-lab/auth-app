import { lazy } from "solid-js";
import { defineFeature } from "@auth-app/feature-kit";

/**
 * The users feature. It contributes the `/users` route and its sidebar entry,
 * demonstrating that several independent feature packages compose in the shell
 * without any of them referencing the others.
 *
 * Its route component is `lazy`-loaded, so this view ships as its own chunk
 * fetched only when `/users` is first visited (see the dashboard feature for
 * the rationale).
 */
export const usersFeature = defineFeature({
  id: "users",
  title: "Users",
  nav: [{ path: "/users", label: "Users", icon: "◍", order: 20 }],
  routes: [
    {
      path: "/users",
      component: lazy(() => import("./Users").then((m) => ({ default: m.Users }))),
    },
  ],
});

export default usersFeature;
