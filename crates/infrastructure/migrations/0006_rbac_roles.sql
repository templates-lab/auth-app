-- Minimal RBAC (bead authapp-e00d47): a role column on `admin_users`, and a
-- snapshot of it on `admin_sessions` (set when the session is issued, so
-- authenticating a request never needs a second query against admin_users —
-- the same trade-off already made for admin_sessions.csrf_token).
--
-- `role` is free-form TEXT, not a Postgres enum: adding a new role is a data
-- change (insert a row with that value), never a migration — see
-- `domain::auth::Role` for why that is a deliberate design choice, not an
-- oversight.
ALTER TABLE admin_users ADD COLUMN role TEXT NOT NULL DEFAULT 'admin';
ALTER TABLE admin_sessions ADD COLUMN role TEXT NOT NULL DEFAULT 'admin';
