import { createSignal, onCleanup, For, Show, type ParentProps } from "solid-js";
import { A, useNavigate } from "@solidjs/router";
import { collectNav } from "./compose";
import { features } from "./registry";
import { useSession } from "../auth/session";
import { logout } from "../auth/api";

const APP_NAME = "Auth App";

/** The initial shown in the avatar, derived from the admin's role. */
function avatarInitial(role: string): string {
  return (role[0] ?? "?").toUpperCase();
}

/** Profile menu items that link to settings sub-routes. */
const PROFILE_MENU = [
  { icon: "\u26BF", label: "Passwords & security", path: "/settings/security" },
  { icon: "\u25C9", label: "Manage account", path: "/settings/account" },
  { icon: "\u270E", label: "Customize profile", path: "/settings/profile" },
  { icon: "\u25D4", label: "Preferences", path: "/settings/preferences" },
];

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
  const [profileOpen, setProfileOpen] = createSignal(false);
  const session = useSession();
  const nav = collectNav(features, session.role);
  const navigate = useNavigate();
  const closeNav = () => setNavOpen(false);

  const displayName = () => session.display_name ?? session.role;
  const email = () => session.email ?? "";

  // Sign out server-side, then hard-redirect to login. The full reload clears
  // every cached query so no authenticated data lingers after logout.
  const signOut = async () => {
    await logout().catch(() => undefined);
    window.location.assign("/login");
  };

  // Close profile popover on click-outside or ESC.
  let profileRef: HTMLDivElement | undefined;

  const handleClickOutside = (e: MouseEvent) => {
    if (profileRef && !profileRef.contains(e.target as Node)) {
      setProfileOpen(false);
    }
  };

  const handleEsc = (e: KeyboardEvent) => {
    if (e.key === "Escape") setProfileOpen(false);
  };

  // Attach/detach listeners based on popover state.
  const startListening = () => {
    document.addEventListener("mousedown", handleClickOutside);
    document.addEventListener("keydown", handleEsc);
  };
  const stopListening = () => {
    document.removeEventListener("mousedown", handleClickOutside);
    document.removeEventListener("keydown", handleEsc);
  };
  onCleanup(stopListening);

  const toggleProfile = () => {
    const next = !profileOpen();
    setProfileOpen(next);
    if (next) startListening();
    else stopListening();
  };

  const navigateTo = (path: string) => {
    setProfileOpen(false);
    stopListening();
    navigate(path);
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

          {/* Profile trigger + popover */}
          <div class="profile-wrapper" ref={profileRef}>
            <button
              class="profile-trigger"
              type="button"
              aria-expanded={profileOpen()}
              aria-haspopup="true"
              onClick={toggleProfile}
            >
              <span class="avatar" aria-hidden="true">
                {avatarInitial(session.role)}
              </span>
              <span class="topbar__user-name">{displayName()}</span>
            </button>

            <Show when={profileOpen()}>
              <div class="profile-popover" role="menu">
                <div class="profile-card">
                  <span class="profile-card__avatar" aria-hidden="true">
                    {avatarInitial(session.role)}
                  </span>
                  <div class="profile-card__info">
                    <span class="profile-card__name">{displayName()}</span>
                    <Show when={email()}>
                      <span class="profile-card__email">{email()}</span>
                    </Show>
                    <span class="profile-card__role">{session.role}</span>
                  </div>
                </div>

                <div class="profile-menu__divider" />

                <nav class="profile-menu">
                  {PROFILE_MENU.map((item) => (
                    <button
                      class="profile-menu__item"
                      type="button"
                      role="menuitem"
                      onClick={() => navigateTo(item.path)}
                    >
                      <span class="profile-menu__icon">{item.icon}</span>
                      <span>{item.label}</span>
                    </button>
                  ))}
                </nav>

                <div class="profile-menu__divider" />

                <button
                  class="profile-menu__item profile-menu__item--danger"
                  type="button"
                  role="menuitem"
                  onClick={() => void signOut()}
                >
                  <span class="profile-menu__icon">⎋</span>
                  <span>Sign out</span>
                </button>
              </div>
            </Show>
          </div>
        </header>
        <main class="content">{props.children}</main>
      </div>
    </div>
  );
}
