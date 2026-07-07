import { lazy } from "solid-js";
import { defineFeature } from "@auth-app/feature-kit";
import "./transactions.css";

/**
 * The transactions feature. It contributes the `/transactions` list route, the
 * `/transactions/:id` detail route, and a sidebar entry — the admin view of
 * payments, with a role-gated refund action (bead authapp-a18fa6).
 *
 * Both route components are `lazy`-loaded so each ships as its own chunk,
 * fetched only when first visited (see the dashboard feature for the rationale).
 * The feature depends on `@auth-app/query` for its typed keys and the api-client
 * bridge, never on another feature.
 */
export const transactionsFeature = defineFeature({
  id: "transactions",
  title: "Transactions",
  nav: [{ path: "/transactions", label: "Transactions", icon: "▤", order: 30 }],
  routes: [
    {
      path: "/transactions",
      component: lazy(() => import("./Transactions").then((m) => ({ default: m.Transactions }))),
    },
    {
      path: "/transactions/:id",
      component: lazy(() =>
        import("./TransactionDetail").then((m) => ({ default: m.TransactionDetail })),
      ),
    },
  ],
});

export default transactionsFeature;
