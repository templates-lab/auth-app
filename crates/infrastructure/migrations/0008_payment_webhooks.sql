-- Payment webhook receipts (bead authapp-c78901). One append-only row per
-- received webhook: the raw payload for diagnostics/replay, whether it was
-- accepted (signature verified) or rejected, and the provider event id used
-- for idempotency.
--
-- Lives in the `payments` schema alongside the rest of the payments module.
CREATE TABLE payments.webhook_events (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    -- The provider's globally-unique event id. NULL for a rejected receipt
    -- (bad/missing signature): we do not trust an unverified body enough to
    -- read an id out of it.
    event_id TEXT,
    -- The raw request body, exactly as received (signatures are computed over
    -- these bytes, so we keep them verbatim for replay).
    payload BYTEA NOT NULL,
    -- The signature header as received, for diagnostics.
    signature TEXT,
    -- Whether the signature verified and the event was claimed for processing.
    accepted BOOLEAN NOT NULL,
    -- A short machine label for a rejected receipt (e.g. `invalid_signature`).
    reason TEXT,
    received_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Idempotency: at most one row per provider event id. Partial (only accepted
-- receipts carry an event id), so the many NULL-event-id rejected rows never
-- collide. This unique index is the ON CONFLICT target that makes a redelivered
-- event a no-op.
CREATE UNIQUE INDEX payments_webhook_events_event_id_idx
    ON payments.webhook_events (event_id)
    WHERE event_id IS NOT NULL;
