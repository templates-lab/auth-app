/**
 * Bridge between the api-client's non-throwing result shape and TanStack
 * Query's throw-on-error contract.
 *
 * `openapi-fetch` returns `{ data, error, response }` instead of throwing, but a
 * Query `queryFn`/`mutationFn` must *throw* for the query to enter its error
 * state. {@link unwrap} adapts one to the other: it returns `data` on success
 * and throws an {@link ApiError} carrying the HTTP status on failure, so the
 * global error handler can recognise a 401 uniformly regardless of which query
 * or mutation produced it.
 */

/** An error raised from an unsuccessful API response, tagged with its status. */
export class ApiError extends Error {
  /** The HTTP status code of the failing response (e.g. `401`, `404`, `500`). */
  readonly status: number;
  /** The parsed error body openapi-fetch surfaced, if any. */
  readonly body: unknown;

  constructor(status: number, message: string, body?: unknown) {
    super(message);
    this.name = "ApiError";
    this.status = status;
    this.body = body;
  }
}

/** Whether an unknown thrown value is an {@link ApiError} with a 401 status. */
export function isUnauthorized(error: unknown): boolean {
  return error instanceof ApiError && error.status === 401;
}

/**
 * The result shape every `openapi-fetch` call resolves to. Declared structurally
 * (rather than importing the api-client) so this package stays decoupled from
 * the generated schema — the app wires the two together.
 */
export interface FetchResult<T> {
  data?: T;
  error?: unknown;
  response: Response;
}

/**
 * Return `data` from an api-client result, or throw {@link ApiError} when the
 * response was not ok. Use it inside every `queryFn`/`mutationFn`:
 *
 * ```ts
 * queryFn: () => unwrap(api.GET("/auth/me"))
 * ```
 *
 * A successful response with no body (e.g. `204`) resolves to `undefined`,
 * which is the correct value for a `void` endpoint.
 */
export async function unwrap<T>(call: Promise<FetchResult<T>>): Promise<T> {
  const { data, error, response } = await call;
  if (!response.ok) {
    const message = errorMessage(error) ?? `Request failed with status ${response.status}`;
    throw new ApiError(response.status, message, error);
  }
  return data as T;
}

/** Best-effort extraction of a human-readable message from an error body. */
function errorMessage(error: unknown): string | undefined {
  if (typeof error === "object" && error !== null && "message" in error) {
    const { message } = error as { message: unknown };
    if (typeof message === "string") {
      return message;
    }
  }
  return undefined;
}
