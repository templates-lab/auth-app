import type { Component } from "solid-js";
import { useLocation } from "@solidjs/router";

/** Placeholder view for settings sub-sections not yet implemented. */
export const SettingsStub: Component = () => {
  const location = useLocation();
  const sectionName = () => {
    const last = location.pathname.split("/").pop() ?? "";
    return last.charAt(0).toUpperCase() + last.slice(1);
  };

  return (
    <section class="feature">
      <header class="feature__header">
        <h1 class="feature__title">{sectionName()}</h1>
        <p class="feature__subtitle">This section is coming soon.</p>
      </header>
      <div class="card" style={{ "text-align": "center", padding: "3rem 1.5rem" }}>
        <p style={{ "font-size": "2rem", margin: "0 0 0.5rem" }}>🚧</p>
        <p style={{ color: "var(--color-muted)" }}>
          The <strong>{sectionName()}</strong> settings page is under construction.
        </p>
      </div>
    </section>
  );
};
