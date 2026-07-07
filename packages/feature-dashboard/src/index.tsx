import { lazy } from "solid-js";
import { defineFeature } from "@auth-app/feature-kit";

/**
 * The dashboard feature. It contributes the app's index route and the first
 * sidebar entry. The shell discovers everything it needs through this module.
 *
 * The route component is `lazy`-loaded so Vite splits it into its own chunk —
 * the shell's initial bundle stays small and each feature's view is fetched on
 * demand when its route is first visited.
 */
export const dashboardFeature = defineFeature({
  id: "dashboard",
  title: "Dashboard",
  nav: [{ path: "/", label: "Dashboard", icon: "▚", order: 10 }],
  routes: [
    {
      path: "/",
      component: lazy(() => import("./Dashboard").then((m) => ({ default: m.Dashboard }))),
    },
  ],
});

export default dashboardFeature;
