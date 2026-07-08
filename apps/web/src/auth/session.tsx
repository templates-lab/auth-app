import { Show, type JSX, type ParentProps } from "solid-js";
import { useQuery } from "@tanstack/solid-query";
import { isUnauthorized } from "@auth-app/query";
import { SessionContext, useSession, useHasRole } from "@auth-app/feature-kit";
import { getMe, meKey, type Me } from "./api";

// Re-export so existing shell imports keep working.
export { useSession, useHasRole };
export type { Me };

/**
 * Gate a subtree behind a valid session (AC: admin routes inaccessible without
 * one). It resolves `/auth/me` before rendering: while pending it shows a
 * loader; on success it provides the session and renders its children; on a
 * 401 the global interceptor has already hard-redirected to `/login?next=…`
 * (see `redirectToLoginOnUnauthorized`), so this only ever renders a brief
 * "redirecting" state for that case, and a retryable error for anything else.
 */
export function RequireSession(props: ParentProps): JSX.Element {
  const me = useQuery(() => ({
    queryKey: meKey(),
    queryFn: getMe,
    // A 401 is a definitive "no session" — don't retry it; fail fast so the
    // global handler redirects. staleTime keeps the check from refetching on
    // every navigation within a session.
    retry: false,
    staleTime: 60_000,
  }));

  return (
    <Show when={!me.isPending} fallback={<SessionSplash message="Loading…" />}>
      <Show
        when={me.data}
        fallback={
          <Show
            when={me.isError && !isUnauthorized(me.error)}
            fallback={<SessionSplash message="Redirecting to sign in…" />}
          >
            <SessionSplash
              message="Could not verify your session."
              onRetry={() => void me.refetch()}
            />
          </Show>
        }
      >
        {(data) => (
          <SessionContext.Provider value={() => data()}>{props.children}</SessionContext.Provider>
        )}
      </Show>
    </Show>
  );
}

/** A minimal full-viewport status screen used while the session resolves. */
function SessionSplash(props: { message: string; onRetry?: () => void }): JSX.Element {
  return (
    <div class="session-splash">
      <p>{props.message}</p>
      <Show when={props.onRetry}>
        <button type="button" onClick={() => props.onRetry?.()}>
          Retry
        </button>
      </Show>
    </div>
  );
}
