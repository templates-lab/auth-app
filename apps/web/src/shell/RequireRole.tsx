import type { ParentProps } from "solid-js";
import { A } from "@solidjs/router";
import { isRoleAllowed, useSession } from "@auth-app/feature-kit";

interface RequireRoleProps extends ParentProps {
  roles: string[];
}

/**
 * Route guard that renders its children only when the authenticated user's role
 * is included in `props.roles`. Otherwise it renders a 403 "Access denied"
 * view with a link back to the dashboard.
 *
 * Must be rendered inside `<RequireSession>` so that {@link useSession} always
 * resolves to a real identity.
 */
export function RequireRole(props: RequireRoleProps) {
  const session = useSession();

  if (isRoleAllowed(session.role, props.roles)) {
    return <>{props.children}</>;
  }

  return (
    <section class="feature">
      <header class="feature__header">
        <h1 class="feature__title">Access denied</h1>
        <p class="feature__subtitle">You do not have permission to view this page.</p>
      </header>
      <A class="link" href="/">
        Back to dashboard
      </A>
    </section>
  );
}
