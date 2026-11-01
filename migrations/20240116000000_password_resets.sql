-- Password reset codes.
-- A 6-digit code is generated and emailed; the user enters it together with
-- their new password.  Codes expire after 60 minutes and are single-use.
CREATE TABLE IF NOT EXISTS password_resets (
    id         TEXT    PRIMARY KEY DEFAULT (lower(hex(randomblob(16)))),
    email      TEXT    NOT NULL,
    code       TEXT    NOT NULL,
    expires_at TEXT    NOT NULL
);
