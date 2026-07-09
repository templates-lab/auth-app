import { lazy } from "solid-js";
import { defineFeature } from "@auth-app/feature-kit";

const Settings = lazy(() => import("./Settings").then((m) => ({ default: m.Settings })));
const SettingsStub = lazy(() =>
  import("./SettingsStub").then((m) => ({ default: m.SettingsStub })),
);

export const settingsFeature = defineFeature({
  id: "settings",
  title: "Settings",
  nav: [{ path: "/settings", label: "Settings", icon: "\u2699", order: 90 }],
  routes: [
    { path: "/settings", component: Settings },
    { path: "/settings/security", component: SettingsStub },
    { path: "/settings/account", component: SettingsStub },
    { path: "/settings/profile", component: SettingsStub },
    { path: "/settings/preferences", component: SettingsStub },
  ],
});

export default settingsFeature;
