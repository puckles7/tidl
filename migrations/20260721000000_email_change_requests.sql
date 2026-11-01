-- Pending email-change confirmations for the profile editor.
-- A 6-digit code is emailed to the *new* address; entering it swaps
-- `users.email` over. One pending request per user — a new request
-- overwrites any previous one, same shape as `email_verifications` and
-- `password_resets`.
CREATE TABLE email_change_requests (
    username    TEXT     NOT NULL PRIMARY KEY REFERENCES users(username) ON DELETE CASCADE,
    new_email   TEXT     NOT NULL,
    code        TEXT     NOT NULL,
    expires_at  DATETIME NOT NULL
);
