import { defineFeature } from "@auth-app/feature-kit";
import { Users } from "./Users";

/**
 * The users feature. It contributes the `/users` route and its sidebar entry,
 * demonstrating that several independent feature packages compose in the shell
 * without any of them referencing the others.
 */
export const usersFeature = defineFeature({
  id: "users",
  title: "Users",
  nav: [{ path: "/users", label: "Users", icon: "◍", order: 20 }],
  routes: [{ path: "/users", component: Users }],
});

export default usersFeature;
