import { createSignal, For, Show, type ParentProps } from "solid-js";
import { A } from "@solidjs/router";
import { collectNav } from "./compose";
import { features } from "./registry";
import { useSession } from "../auth/session";
import { logout } from "../auth/api";

const APP_NAME = "Auth App";

/** The initial shown in the avatar, derived from the admin's role. */
function avatarInitial(role: string): string {
  return (role[0] ?? "?").toUpperCase();
}

/**
 * The responsive admin chrome: a sidebar of feature navigation, a top header
 * showing the signed-in admin with a sign-out control, and the content area
 * where the matched route renders. `props.children` is the active route's view.
 *
 * Rendered only inside `<RequireSession>`, so {@link useSession} always resolves
 * to a real identity here.
 *
 * On narrow viewports the sidebar collapses into an off-canvas drawer toggled
 * from the header; on wide viewports it is a permanent column.
 */
export function AdminLayout(props: ParentProps) {
  const [navOpen, setNavOpen] = createSignal(false);
  const nav = collectNav(features);
  const closeNav = () => setNavOpen(false);
  const session = useSession();

  // Sign out server-side, then hard-redirect to login. The full reload clears
  // every cached query so no authenticated data lingers after logout.
  const signOut = async () => {
    await logout().catch(() => undefined);
    window.location.assign("/login");
  };

  return (
    <div class="admin" classList={{ "admin--nav-open": navOpen() }}>
      <aside class="sidebar" aria-label="Primary">
        <div class="sidebar__brand">
          <span class="sidebar__mark">◆</span>
          <span>{APP_NAME}</span>
        </div>
        <nav class="sidebar__nav">
          <For each={nav}>
            {(item) => (
              <A
                class="nav-link"
                activeClass="nav-link--active"
                href={item.path}
                end={item.path === "/"}
                onClick={closeNav}
              >
                <Show when={item.icon}>
                  <span class="nav-link__icon" aria-hidden="true">
                    {item.icon}
                  </span>
                </Show>
                <span>{item.label}</span>
              </A>
            )}
          </For>
        </nav>
      </aside>

      <button
        class="scrim"
        aria-label="Close navigation"
        tabindex={navOpen() ? 0 : -1}
        onClick={closeNav}
      />

      <div class="admin__main">
        <header class="topbar">
          <button
            class="topbar__toggle"
            aria-label="Toggle navigation"
            aria-expanded={navOpen()}
            onClick={() => setNavOpen((open) => !open)}
          >
            ☰
          </button>
          <div class="topbar__spacer" />
          <div class="topbar__user">
            <span class="avatar" aria-hidden="true">
              {avatarInitial(session.role)}
            </span>
            <span class="topbar__user-name">{session.role}</span>
            <button class="topbar__signout" type="button" onClick={() => void signOut()}>
              Sign out
            </button>
          </div>
        </header>
        <main class="content">{props.children}</main>
      </div>
    </div>
  );
}
