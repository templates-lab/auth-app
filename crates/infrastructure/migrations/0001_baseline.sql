-- Baseline migration: cluster-wide prerequisites shared by every module.
--
-- Feature modules (users, sessions, credentials, ...) add their own versioned
-- migrations under this directory as they land — one file per change, named
-- `NNNN_<module>_<change>.sql`, applied in lexical order. This baseline only
-- establishes what every later migration may assume, so a fresh database
-- (`sqlx migrate run` from scratch) always starts from the same foundation.

-- `pgcrypto` provides gen_random_uuid() and the crypt()/digest() family, used
-- by the auth modules for primary keys and password/secret hashing.
CREATE EXTENSION IF NOT EXISTS pgcrypto;
