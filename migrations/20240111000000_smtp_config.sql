-- smtp_config: singleton row (id always 1) for outbound email settings.
-- password_encrypted stores AES-256-GCM ciphertext as "{nonce_b64}:{ct_b64}".
CREATE TABLE smtp_config (
    id                 INTEGER PRIMARY KEY CHECK (id = 1),
    host               TEXT    NOT NULL,
    port               INTEGER NOT NULL DEFAULT 587,
    tls_mode           TEXT    NOT NULL DEFAULT 'starttls'
                           CHECK (tls_mode IN ('none', 'starttls', 'tls')),
    username           TEXT    NOT NULL DEFAULT '',
    password_encrypted TEXT    NOT NULL DEFAULT '',
    from_address       TEXT    NOT NULL,
    from_name          TEXT    NOT NULL DEFAULT '',
    updated_at         DATETIME NOT NULL DEFAULT (datetime('now'))
);
