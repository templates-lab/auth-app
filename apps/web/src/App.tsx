import { useState, type FormEvent } from "react";
import { isValidEmail } from "@auth-app/shared";

/**
 * Placeholder sign-in screen. It exists to prove the workspace wiring end to
 * end: the app consumes `@auth-app/shared`, typechecks, lints, and builds.
 * Real authentication flows land in follow-up beads.
 */
export function App() {
  const [email, setEmail] = useState("");
  const [submitted, setSubmitted] = useState<string | null>(null);

  const emailValid = email.length === 0 || isValidEmail(email);

  function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (isValidEmail(email)) {
      setSubmitted(email);
    }
  }

  return (
    <main>
      <h1>Auth App</h1>
      <form onSubmit={handleSubmit}>
        <label htmlFor="email">Email</label>
        <input
          id="email"
          type="email"
          value={email}
          onChange={(event) => setEmail(event.target.value)}
          aria-invalid={!emailValid}
        />
        <button type="submit" disabled={!isValidEmail(email)}>
          Continue
        </button>
      </form>
      {submitted ? <p>Signed in as {submitted}</p> : null}
    </main>
  );
}
