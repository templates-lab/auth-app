import type { FeatureModule } from "@auth-app/feature-kit";
import { dashboardFeature } from "@auth-app/feature-dashboard";
import { usersFeature } from "@auth-app/feature-users";
import { transactionsFeature } from "@auth-app/feature-transactions";
import { settingsFeature } from "@auth-app/feature-settings";

/**
 * The features mounted into the shell, in registration order.
 *
 * This array is the *only* place a feature is wired into the app. To add a
 * feature: create its package, add it as a dependency of `@auth-app/web`, then
 * import and list it here. No existing feature — and no layout code — changes.
 */
export const features: FeatureModule[] = [
  dashboardFeature,
  usersFeature,
  transactionsFeature,
  settingsFeature,
];
