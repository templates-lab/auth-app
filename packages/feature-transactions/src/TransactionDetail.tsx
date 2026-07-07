import { createMemo, For, Show, type Component } from "solid-js";
import { A, useParams } from "@solidjs/router";
import { useMutation, useQuery, useQueryClient } from "@tanstack/solid-query";
import { authKeys, getMe, getTransaction, refundTransaction, transactionsKeys } from "./api";
import { formatEpoch, formatMoney, formatStatus } from "./format";

/** The statuses a payment can still be refunded from. */
const REFUNDABLE = new Set(["captured", "partially_refunded"]);

/**
 * A single payment's detail: its fields, the full status history, and — for an
 * admin, on a refundable payment — a confirmed refund action that invalidates
 * the affected queries on success (ACs: detalle con historial completo;
 * reembolso solo admin y con confirmación).
 */
export const TransactionDetail: Component = () => {
  const params = useParams<{ id: string }>();
  const queryClient = useQueryClient();

  const detail = useQuery(() => ({
    queryKey: transactionsKeys.detail(params.id),
    queryFn: () => getTransaction(params.id),
  }));

  // The current admin's role gates the refund action; cached briefly and shared
  // under its own key so it is fetched once, not per payment.
  const me = useQuery(() => ({
    queryKey: authKeys.detail("me"),
    queryFn: getMe,
    staleTime: 5 * 60 * 1000,
  }));
  const isAdmin = () => me.data?.role === "admin";

  const refund = useMutation(() => ({
    mutationFn: () => refundTransaction(params.id),
    onSuccess: () => {
      // The refunded payment and any list it appears in are now stale.
      void queryClient.invalidateQueries({ queryKey: transactionsKeys.detail(params.id) });
      void queryClient.invalidateQueries({ queryKey: transactionsKeys.lists() });
    },
  }));

  const status = () => detail.data?.transaction.status;
  const canRefund = createMemo(() => isAdmin() && !!status() && REFUNDABLE.has(status() as string));

  const onRefund = () => {
    if (!window.confirm("Refund this payment in full? This cannot be undone.")) {
      return;
    }
    refund.mutate();
  };

  return (
    <section class="feature">
      <header class="feature__header">
        <A class="link" href="/transactions">
          ← Transactions
        </A>
        <h1 class="feature__title">Transaction</h1>
      </header>

      <Show when={!detail.isPending} fallback={<p class="txn-muted">Loading transaction…</p>}>
        <Show
          when={detail.data}
          fallback={<p class="txn-error">This transaction could not be found.</p>}
        >
          {(data) => (
            <>
              <div class="card txn-summary">
                <dl class="txn-dl">
                  <div>
                    <dt>Payment id</dt>
                    <dd>{data().transaction.id}</dd>
                  </div>
                  <div>
                    <dt>Amount</dt>
                    <dd>
                      {formatMoney(
                        data().transaction.amount_minor_units,
                        data().transaction.currency,
                      )}
                    </dd>
                  </div>
                  <div>
                    <dt>Status</dt>
                    <dd>
                      <span class="badge">{formatStatus(data().transaction.status)}</span>
                    </dd>
                  </div>
                  <div>
                    <dt>Provider reference</dt>
                    <dd>{data().transaction.provider_reference ?? "—"}</dd>
                  </div>
                </dl>

                <Show when={canRefund()}>
                  <div class="txn-actions">
                    <button
                      type="button"
                      class="btn-danger"
                      disabled={refund.isPending}
                      onClick={onRefund}
                    >
                      {refund.isPending ? "Refunding…" : "Refund payment"}
                    </button>
                    <Show when={refund.isError}>
                      <span class="txn-error">Refund failed. Please try again.</span>
                    </Show>
                    <Show when={refund.isSuccess}>
                      <span class="txn-ok">Refunded.</span>
                    </Show>
                  </div>
                </Show>
              </div>

              <div class="card">
                <h2 class="txn-section">Status history</h2>
                <table class="table">
                  <thead>
                    <tr>
                      <th>When</th>
                      <th>From</th>
                      <th>To</th>
                      <th>Reason</th>
                    </tr>
                  </thead>
                  <tbody>
                    <For each={data().history}>
                      {(change) => (
                        <tr>
                          <td class="table__muted">{formatEpoch(change.occurred_at_epoch)}</td>
                          <td>{change.from ? formatStatus(change.from) : "—"}</td>
                          <td>{formatStatus(change.to)}</td>
                          <td class="table__muted">{change.reason ?? "—"}</td>
                        </tr>
                      )}
                    </For>
                  </tbody>
                </table>
              </div>
            </>
          )}
        </Show>
      </Show>
    </section>
  );
};
