-- Payments bounded context: its own Postgres schema, kept separate from the
-- default/auth tables so the module's data model stays visibly self-contained
-- (bead authapp-7262d1). Amounts are minor units (e.g. cents) in a BIGINT
-- column, never floating point; timestamps follow the same Unix-epoch-seconds
-- convention as `admin_users`/`admin_sessions` (see
-- crates/infrastructure/src/admin_repo.rs).

CREATE SCHEMA payments;

-- One row per payment. `provider_reference` is NULL until the configured
-- `PaymentProvider` creates its intent for this payment; `status` is the
-- `PaymentStatus` state machine's current state, written only through
-- `payments.payment_status_history` (see below) so the two never drift.
CREATE TABLE payments.payments (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    provider_reference TEXT,
    amount_minor_units BIGINT NOT NULL CHECK (amount_minor_units >= 0),
    currency TEXT NOT NULL,
    status TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);

CREATE UNIQUE INDEX payments_provider_reference_idx
    ON payments.payments (provider_reference)
    WHERE provider_reference IS NOT NULL;

-- The full audit trail of status changes. `from_status` is NULL for the row
-- recording a payment's creation. Never updated or deleted — a payment's
-- history is append-only by construction (the repository adapter only ever
-- INSERTs here, in the same transaction as the `payments.payments` update).
CREATE TABLE payments.payment_status_history (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    payment_id UUID NOT NULL REFERENCES payments.payments (id) ON DELETE CASCADE,
    from_status TEXT,
    to_status TEXT NOT NULL,
    reason TEXT,
    occurred_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX payment_status_history_payment_id_idx
    ON payments.payment_status_history (payment_id);
