import { createSignal, For, Show, type ParentProps } from "solid-js";
import { A } from "@solidjs/router";
import { collectNav } from "./compose";
import { features } from "./registry";

const APP_NAME = "Auth App";

/**
 * The responsive admin chrome: a sidebar of feature navigation, a top header,
 * and the content area where the matched route renders. It is passed as the
 * `root` of the `<Router>`, so `props.children` is the active route's view.
 *
 * On narrow viewports the sidebar collapses into an off-canvas drawer toggled
 * from the header; on wide viewports it is a permanent column.
 */
export function AdminLayout(props: ParentProps) {
  const [navOpen, setNavOpen] = createSignal(false);
  const nav = collectNav(features);
  const closeNav = () => setNavOpen(false);

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
              A
            </span>
            <span class="topbar__user-name">admin@example.com</span>
          </div>
        </header>
        <main class="content">{props.children}</main>
      </div>
    </div>
  );
}
