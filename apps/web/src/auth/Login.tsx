import { createSignal, For, Show, type Component } from "solid-js";
import { useNavigate, useSearchParams } from "@solidjs/router";
import { useMutation, useQuery } from "@tanstack/solid-query";
import { ApiError } from "@auth-app/query";
import { authKeys, listProviders, login, oauthStartUrl } from "./api";
import "./login.css";

/** Turn a failed login into a message that never reveals whether the account
 * exists — only "these credentials did not work" or "you are being throttled". */
function loginErrorMessage(error: unknown): string {
  if (error instanceof ApiError && error.status === 429) {
    return "Too many attempts. Please wait a moment and try again.";
  }
  return "Invalid email or password.";
}

/** Format a provider id (`google`) into a button label (`Continue with Google`). */
function providerLabel(id: string): string {
  return `Continue with ${id.charAt(0).toUpperCase()}${id.slice(1)}`;
}

/**
 * The sign-in screen. Rendered outside the admin chrome (see the shell router):
 * a password form plus a button per configured OAuth provider. On success it
 * returns to the route the user was sent from (`?next=`), defaulting to the
 * dashboard. Loading and error states come straight off the TanStack Query
 * mutation.
 */
export const Login: Component = () => {
  const navigate = useNavigate();
  const [params] = useSearchParams<{ next?: string; error?: string }>();

  const [email, setEmail] = createSignal("");
  const [password, setPassword] = createSignal("");

  const next = () => params.next ?? "/";

  const providers = useQuery(() => ({
    queryKey: authKeys.list({ scope: "oauth-providers" }),
    queryFn: listProviders,
    staleTime: 5 * 60 * 1000,
  }));

  const signIn = useMutation(() => ({
    mutationFn: () => login(email(), password()),
    onSuccess: () => navigate(next(), { replace: true }),
  }));

  const onSubmit = (e: Event) => {
    e.preventDefault();
    if (!signIn.isPending) {
      signIn.mutate();
    }
  };

  return (
    <main class="login">
      <form class="login__card" onSubmit={onSubmit}>
        <div class="login__brand">
          <span class="login__mark">◆</span>
          <span>Auth App</span>
        </div>
        <h1 class="login__title">Sign in</h1>

        <Show when={params.error === "oauth"}>
          <p class="login__alert" role="alert">
            Single sign-on failed or was cancelled. Please try again.
          </p>
        </Show>
        <Show when={signIn.isError}>
          <p class="login__alert" role="alert">
            {loginErrorMessage(signIn.error)}
          </p>
        </Show>

        <label class="login__field">
          <span>Email</span>
          <input
            type="email"
            name="email"
            autocomplete="username"
            required
            value={email()}
            onInput={(e) => setEmail(e.currentTarget.value)}
          />
        </label>
        <label class="login__field">
          <span>Password</span>
          <input
            type="password"
            name="password"
            autocomplete="current-password"
            required
            value={password()}
            onInput={(e) => setPassword(e.currentTarget.value)}
          />
        </label>

        <button class="login__submit" type="submit" disabled={signIn.isPending}>
          {signIn.isPending ? "Signing in…" : "Sign in"}
        </button>

        <Show when={(providers.data ?? []).length > 0}>
          <div class="login__divider">
            <span>or</span>
          </div>
          <div class="login__providers">
            <For each={providers.data}>
              {(id) => (
                <a class="login__provider" href={oauthStartUrl(id)} rel="external">
                  {providerLabel(id)}
                </a>
              )}
            </For>
          </div>
        </Show>
      </form>
    </main>
  );
};
