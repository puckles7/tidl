-- Email verification codes for new account sign-ups.
--
-- When a user signs up via the web UI, their account is created with
-- `is_active = 0` and a 6-digit numeric code is stored here.  The account
-- becomes active only after the user submits the correct code.
--
-- Records are deleted once the code is accepted.  Expired records are pruned
-- automatically during the next sign-up attempt for the same username.
CREATE TABLE IF NOT EXISTS email_verifications (
    id          INTEGER  PRIMARY KEY AUTOINCREMENT,
    username    TEXT     NOT NULL,
    code        TEXT     NOT NULL,
    expires_at  DATETIME NOT NULL,
    created_at  DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_email_verifications_username
    ON email_verifications (username);
