-- Server-side admin sessions. Owned by the sessions feature (bead
-- authapp-4a1cbc). Timestamps are TIMESTAMPTZ; the Rust adapter reads and
-- writes them as Unix epoch seconds, the same convention `admin_users` and
-- `admin_ip_lockouts` use (see crates/infrastructure/src/admin_repo.rs).

-- One row per live session. `token` is the bearer secret handed to the client
-- as the session cookie value and doubles as the primary key: a lookup is a
-- point query on an unguessable 256-bit value, so no separate surrogate id
-- adds anything. `csrf_token` is the synchronizer token mirrored into a
-- client-readable cookie and checked against the `X-CSRF-Token` header on
-- every mutation. Deleting an admin cascades to their sessions so a removed
-- account cannot keep a live session around.
CREATE TABLE admin_sessions (
    token TEXT PRIMARY KEY,
    admin_id UUID NOT NULL REFERENCES admin_users (id) ON DELETE CASCADE,
    csrf_token TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    last_seen_at TIMESTAMPTZ NOT NULL,
    absolute_expires_at TIMESTAMPTZ NOT NULL
);

-- Supports "revoke every session for this admin" (e.g. a future
-- change-password flow) without a full table scan.
CREATE INDEX admin_sessions_admin_id_idx ON admin_sessions (admin_id);
