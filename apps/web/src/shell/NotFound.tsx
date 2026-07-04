import { A } from "@solidjs/router";
import type { Component } from "solid-js";

/** Fallback view for a URL no registered feature owns. */
export const NotFound: Component = () => {
  return (
    <section class="feature">
      <header class="feature__header">
        <h1 class="feature__title">Page not found</h1>
        <p class="feature__subtitle">The page you were looking for does not exist.</p>
      </header>
      <A class="link" href="/">
        Back to dashboard
      </A>
    </section>
  );
};
