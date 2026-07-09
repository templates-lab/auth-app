import type { Component } from "solid-js";
import { A } from "@solidjs/router";
import "./settings.css";

interface SettingsSection {
  path: string;
  icon: string;
  label: string;
  description: string;
}

const SECTIONS: SettingsSection[] = [
  {
    path: "/settings/security",
    icon: "\u26BF",
    label: "Passwords & security",
    description: "Manage your password, two-factor authentication, and sign-in activity.",
  },
  {
    path: "/settings/account",
    icon: "\u25C9",
    label: "Manage account",
    description: "Update your account details, email address, and connected services.",
  },
  {
    path: "/settings/profile",
    icon: "\u270E",
    label: "Customize profile",
    description: "Change your display name, avatar, and public profile information.",
  },
  {
    path: "/settings/preferences",
    icon: "\u25D4",
    label: "Preferences",
    description: "Configure language, timezone, notification settings, and appearance.",
  },
];

export const Settings: Component = () => {
  return (
    <section class="feature">
      <header class="feature__header">
        <h1 class="feature__title">Settings</h1>
        <p class="feature__subtitle">Manage your account and preferences.</p>
      </header>
      <div class="settings-grid">
        {SECTIONS.map((section) => (
          <A href={section.path} class="settings-card">
            <span class="settings-card__icon">{section.icon}</span>
            <div class="settings-card__body">
              <span class="settings-card__label">{section.label}</span>
              <span class="settings-card__desc">{section.description}</span>
            </div>
          </A>
        ))}
      </div>
    </section>
  );
};
