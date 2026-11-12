-- Open-source Phase 1: the server's settings, kept here and edited through the admin API
-- (src/settings.rs). One row per key; the value is JSON. Environment variables still win.
CREATE TABLE settings (
    key        TEXT PRIMARY KEY,
    value      TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);
