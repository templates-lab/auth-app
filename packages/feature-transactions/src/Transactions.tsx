import { createMemo, createSignal, For, Show, type Component } from "solid-js";
import { A } from "@solidjs/router";
import { useQuery } from "@tanstack/solid-query";
import { listTransactions, transactionsKeys, type TransactionFilters } from "./api";
import { formatEpoch, formatMoney, formatStatus } from "./format";

const PAGE_SIZE = 20;

/** The statuses offered in the filter dropdown, matching the backend's tokens. */
const STATUSES = [
  "created",
  "requires_action",
  "authorized",
  "captured",
  "partially_refunded",
  "refunded",
  "failed",
  "canceled",
];

/** Parse a `<input type="date">` value (`YYYY-MM-DD`) to Unix epoch seconds. */
function dateToEpoch(value: string): number | undefined {
  if (!value) return undefined;
  const ms = Date.parse(`${value}T00:00:00Z`);
  return Number.isNaN(ms) ? undefined : Math.floor(ms / 1000);
}

/**
 * The transactions list: status/date filters and pagination, all driven through
 * TanStack Query so the table reflects the current filter and page with cached,
 * deduplicated fetches (AC: listado paginado con filtros vía TanStack Query).
 */
export const Transactions: Component = () => {
  const [status, setStatus] = createSignal("");
  const [from, setFrom] = createSignal("");
  const [to, setTo] = createSignal("");
  const [page, setPage] = createSignal(0);

  const filters = createMemo<TransactionFilters>(() => ({
    status: status() || undefined,
    createdAfter: dateToEpoch(from()),
    createdBefore: dateToEpoch(to()),
    limit: PAGE_SIZE,
    offset: page() * PAGE_SIZE,
  }));

  const query = useQuery(() => ({
    queryKey: transactionsKeys.list(filters()),
    queryFn: () => listTransactions(filters()),
  }));

  // Changing a filter resets to the first page so the offset never points past
  // a freshly narrowed result set.
  const onFilter = (apply: () => void) => {
    apply();
    setPage(0);
  };

  const total = () => query.data?.total ?? 0;
  const hasNext = () => (page() + 1) * PAGE_SIZE < total();

  return (
    <section class="feature">
      <header class="feature__header">
        <h1 class="feature__title">Transactions</h1>
        <p class="feature__subtitle">Payments across your workspace.</p>
      </header>

      <div class="txn-filters">
        <label class="txn-field">
          <span>Status</span>
          <select
            value={status()}
            onChange={(e) => onFilter(() => setStatus(e.currentTarget.value))}
          >
            <option value="">All</option>
            <For each={STATUSES}>{(s) => <option value={s}>{formatStatus(s)}</option>}</For>
          </select>
        </label>
        <label class="txn-field">
          <span>From</span>
          <input
            type="date"
            value={from()}
            onChange={(e) => onFilter(() => setFrom(e.currentTarget.value))}
          />
        </label>
        <label class="txn-field">
          <span>To</span>
          <input
            type="date"
            value={to()}
            onChange={(e) => onFilter(() => setTo(e.currentTarget.value))}
          />
        </label>
      </div>

      <div class="card">
        <Show when={!query.isPending} fallback={<p class="txn-muted">Loading transactions…</p>}>
          <Show
            when={!query.isError}
            fallback={<p class="txn-error">Could not load transactions.</p>}
          >
            <table class="table">
              <thead>
                <tr>
                  <th>Payment</th>
                  <th>Amount</th>
                  <th>Status</th>
                  <th>Created</th>
                </tr>
              </thead>
              <tbody>
                <For
                  each={query.data?.items ?? []}
                  fallback={
                    <tr>
                      <td colspan="4" class="txn-muted">
                        No transactions match these filters.
                      </td>
                    </tr>
                  }
                >
                  {(txn) => (
                    <tr>
                      <td>
                        <A class="link" href={`/transactions/${txn.id}`}>
                          {txn.id.slice(0, 8)}…
                        </A>
                      </td>
                      <td>{formatMoney(txn.amount_minor_units, txn.currency)}</td>
                      <td>
                        <span class="badge">{formatStatus(txn.status)}</span>
                      </td>
                      <td class="table__muted">{formatEpoch(txn.created_at_epoch)}</td>
                    </tr>
                  )}
                </For>
              </tbody>
            </table>
          </Show>
        </Show>
      </div>

      <div class="txn-pager">
        <button
          type="button"
          disabled={page() === 0}
          onClick={() => setPage((p) => Math.max(0, p - 1))}
        >
          Previous
        </button>
        <span class="txn-muted">
          Page {page() + 1} · {total()} total
        </span>
        <button type="button" disabled={!hasNext()} onClick={() => setPage((p) => p + 1)}>
          Next
        </button>
      </div>
    </section>
  );
};
