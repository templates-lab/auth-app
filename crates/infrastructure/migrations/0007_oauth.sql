-- OAuth2/OIDC sign-in (bead authapp-541886).
--
-- Two tables: the short-lived pending authorizations that tie a browser
-- redirect to its callback, and the durable links between an external
-- identity and a local admin account (the "own table" the acceptance criteria
-- ask for). Timestamps follow the Unix-epoch-seconds convention the rest of
-- the schema uses (see crates/infrastructure/src/admin_repo.rs).

-- One row per in-flight authorization-code flow, keyed by its anti-CSRF
-- `state`. The callback consumes (fetch-and-deletes) it, so a `state` is
-- single-use — a replayed callback finds nothing. `code_verifier` is the PKCE
-- secret kept server-side; it never travels to the browser.
CREATE TABLE oauth_pending_authorizations (
    state TEXT PRIMARY KEY,
    provider TEXT NOT NULL,
    nonce TEXT NOT NULL,
    code_verifier TEXT NOT NULL,
    redirect_uri TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL
);

-- Durable link: an external identity `(provider, subject)` -> a local admin.
-- `(provider, subject)` is unique (one external identity maps to at most one
-- admin); an admin may have several linked identities. Deleting the admin
-- cascades. `email` is stored for display/audit — the authoritative link is
-- the `admin_id`.
CREATE TABLE admin_oauth_identities (
    provider TEXT NOT NULL,
    subject TEXT NOT NULL,
    admin_id UUID NOT NULL REFERENCES admin_users (id) ON DELETE CASCADE,
    email TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (provider, subject)
);

CREATE INDEX admin_oauth_identities_admin_id_idx
    ON admin_oauth_identities (admin_id);
