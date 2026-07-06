-- Admin authentication schema: the administrator accounts that can log in, and
-- the per-IP lockout counters that throttle brute-force attempts.
--
-- Owned by the auth feature (bead authapp-6674dd). Timestamps are TIMESTAMPTZ;
-- the Rust adapter reads and writes them as Unix epoch seconds (via
-- `to_timestamp()` / `extract(epoch ...)`) so it needs no timezone-typed column
-- binding.

-- Administrator accounts. `email` is stored already-normalized (trimmed and
-- lowercased by the domain) and is unique — the login lookup key. `password_hash`
-- holds the argon2id PHC string (parameters + salt are embedded in it).
CREATE TABLE admin_users (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    email TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    -- Per-account lockout counters, mirrored from the domain's LockoutState.
    failed_attempts INTEGER NOT NULL DEFAULT 0,
    locked_until TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Per-IP lockout counters. Separate from an account because an IP is throttled
-- even when it targets emails that do not exist — that is what stops an attacker
-- from using lockout as an account-enumeration oracle. Keyed by the client
-- address as text (IPv4 or IPv6), as the delivery layer resolves it.
CREATE TABLE admin_ip_lockouts (
    ip TEXT PRIMARY KEY,
    failed_attempts INTEGER NOT NULL DEFAULT 0,
    locked_until TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
