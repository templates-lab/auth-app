/**
 * The transactions feature's data layer: its typed query-key namespace and the
 * fetchers that back its queries and the refund mutation. Everything the UI
 * needs to talk to the backend lives here, so the components stay declarative.
 */

import { createApiClient, type components } from "@auth-app/api-client";
import { createFeatureKeys, unwrap } from "@auth-app/query";

/** A single payment row. */
export type Transaction = components["schemas"]["TransactionOut"];
/** One page of payments plus the matching total. */
export type TransactionPage = components["schemas"]["TransactionPage"];
/** A payment with its full status history. */
export type TransactionDetail = components["schemas"]["TransactionDetailOut"];
/** One recorded status change. */
export type StatusChange = components["schemas"]["StatusChangeOut"];
/**
 * The feature's query-key namespace (AC: query keys tipadas y centralizadas por
 * feature). Every query and every invalidation derives its key from here.
 */
export const transactionsKeys = createFeatureKeys("transactions");

/**
 * Filters + paging the list view drives. A `type` (not an `interface`) so it
 * carries an implicit index signature and is accepted as a query-key param
 * (`Record<string, unknown>`).
 */
export type TransactionFilters = {
  status?: string;
  createdAfter?: number;
  createdBefore?: number;
  limit: number;
  offset: number;
};

// One client for the feature. `createApiClient` is a thin, stateless wrapper
// over `fetch`, so a per-feature instance couples nothing to the shell.
const api = createApiClient();

/** Fetch one filtered, paginated page of transactions. */
export function listTransactions(filters: TransactionFilters): Promise<TransactionPage> {
  return unwrap(
    api.GET("/transactions", {
      params: {
        query: {
          status: filters.status || undefined,
          created_after: filters.createdAfter,
          created_before: filters.createdBefore,
          limit: filters.limit,
          offset: filters.offset,
        },
      },
    }),
  );
}

/** Fetch one payment with its full status history. */
export function getTransaction(id: string): Promise<TransactionDetail> {
  return unwrap(api.GET("/transactions/{id}", { params: { path: { id } } }));
}

/** Refund a payment in full. Sends the CSRF header the backend requires on
 * mutations (mirrored from the client-readable `csrf` cookie). */
export function refundTransaction(id: string): Promise<components["schemas"]["RefundOut"]> {
  return unwrap(
    api.POST("/transactions/{id}/refund", {
      params: { path: { id } },
      headers: csrfHeader(),
    }),
  );
}

/**
 * The `X-CSRF-Token` header carrying the value of the client-readable `csrf`
 * cookie the login response set. Empty when absent (the request then fails
 * server-side with 403, the correct outcome for a missing token).
 */
function csrfHeader(): Record<string, string> {
  const token = readCookie("csrf");
  return token ? { "x-csrf-token": token } : {};
}

/** Read a cookie value by name from `document.cookie`, or `undefined`. */
function readCookie(name: string): string | undefined {
  return document.cookie
    .split("; ")
    .find((row) => row.startsWith(`${name}=`))
    ?.slice(name.length + 1);
}
