import { For, type Component } from "solid-js";

interface Metric {
  label: string;
  value: string;
  hint: string;
}

const METRICS: Metric[] = [
  { label: "Active users", value: "1,284", hint: "+3.2% this week" },
  { label: "Sign-ins today", value: "342", hint: "peak at 09:00" },
  { label: "Open sessions", value: "97", hint: "across 12 regions" },
  { label: "Failed logins", value: "5", hint: "last 24h" },
];

/** Landing view of the dashboard feature: a grid of headline metrics. */
export const Dashboard: Component = () => {
  return (
    <section class="feature">
      <header class="feature__header">
        <h1 class="feature__title">Dashboard</h1>
        <p class="feature__subtitle">An overview of your workspace at a glance.</p>
      </header>
      <div class="card-grid">
        <For each={METRICS}>
          {(metric) => (
            <article class="card metric">
              <span class="metric__label">{metric.label}</span>
              <span class="metric__value">{metric.value}</span>
              <span class="metric__hint">{metric.hint}</span>
            </article>
          )}
        </For>
      </div>
    </section>
  );
};
