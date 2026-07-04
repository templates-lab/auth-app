import { defineFeature } from "@auth-app/feature-kit";
import { Dashboard } from "./Dashboard";

/**
 * The dashboard feature. It contributes the app's index route and the first
 * sidebar entry. The shell discovers everything it needs through this module.
 */
export const dashboardFeature = defineFeature({
  id: "dashboard",
  title: "Dashboard",
  nav: [{ path: "/", label: "Dashboard", icon: "▚", order: 10 }],
  routes: [{ path: "/", component: Dashboard }],
});

export default dashboardFeature;
