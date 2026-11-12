-- Open-source Phase 5: invitations. An admin makes a one-time code in the app; a new user types
-- it on the sign-in screen with a user name and password of their choice. Only the code's
-- SHA-256 is kept.
CREATE TABLE invites (
    id         TEXT PRIMARY KEY,
    code_hash  TEXT NOT NULL UNIQUE,
    created_by TEXT,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    admin      INTEGER NOT NULL DEFAULT 0,
    used_at    INTEGER,
    used_by    TEXT
);
