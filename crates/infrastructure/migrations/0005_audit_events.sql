-- Authentication audit trail (bead authapp-c418dc). Append-only: rows are
-- only ever inserted, never updated. `admin_id` has no foreign key to
-- `admin_users` — a failed login against a nonexistent email, or an event for
-- an account later deleted, must still keep its audit row.
--
-- No column here can ever hold a password, session token, or CSRF token —
-- that invariant is enforced by `domain::NewAuditEvent` simply having nowhere
-- to put one, not by a runtime check.
CREATE TABLE admin_audit_events (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    event_type TEXT NOT NULL,
    admin_id UUID,
    email_attempted TEXT,
    ip TEXT NOT NULL,
    user_agent TEXT,
    occurred_at TIMESTAMPTZ NOT NULL
);

-- Supports "recent events" (the admin view) and per-admin history without a
-- full table scan.
CREATE INDEX admin_audit_events_occurred_at_idx ON admin_audit_events (occurred_at DESC);
CREATE INDEX admin_audit_events_admin_id_idx ON admin_audit_events (admin_id);
