-- Extend the contacts model (#79/#29) to cover more of what real CardDAV
-- clients actually send — found via live iOS testing, which also exposed a
-- real bug: ADR (postal address) is repeatable in vCard, but the old
-- single-address columns on `contacts` meant a second ADR line silently
-- overwrote the first. contact_addresses fixes that by making addresses a
-- child table, same shape as contact_emails/contact_phones.

ALTER TABLE contacts ADD COLUMN middle_name TEXT;
ALTER TABLE contacts ADD COLUMN name_prefix TEXT;
ALTER TABLE contacts ADD COLUMN name_suffix TEXT;
ALTER TABLE contacts ADD COLUMN nickname    TEXT;
ALTER TABLE contacts ADD COLUMN anniversary DATE;

CREATE TABLE contact_addresses (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    contact_id  INTEGER NOT NULL REFERENCES contacts(id) ON DELETE CASCADE,
    type        TEXT    NOT NULL DEFAULT 'home' CHECK (type IN ('home', 'work', 'other')),
    is_primary  BOOLEAN NOT NULL DEFAULT 0,
    street      TEXT,
    city        TEXT,
    region      TEXT,
    postal_code TEXT,
    country     TEXT
);

CREATE INDEX idx_contact_addresses_contact ON contact_addresses (contact_id);

-- Migrate any existing single-address data before dropping those columns.
INSERT INTO contact_addresses (contact_id, type, is_primary, street, city, region, postal_code, country)
SELECT id, 'home', 1, street, city, region, postal_code, country
FROM contacts
WHERE street IS NOT NULL OR city IS NOT NULL OR region IS NOT NULL
   OR postal_code IS NOT NULL OR country IS NOT NULL;

ALTER TABLE contacts DROP COLUMN street;
ALTER TABLE contacts DROP COLUMN city;
ALTER TABLE contacts DROP COLUMN region;
ALTER TABLE contacts DROP COLUMN postal_code;
ALTER TABLE contacts DROP COLUMN country;

CREATE TABLE contact_urls (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    contact_id INTEGER NOT NULL REFERENCES contacts(id) ON DELETE CASCADE,
    value      TEXT    NOT NULL,
    type       TEXT    NOT NULL DEFAULT 'other' CHECK (type IN ('home', 'work', 'mobile', 'other')),
    is_primary BOOLEAN NOT NULL DEFAULT 0
);

CREATE INDEX idx_contact_urls_contact ON contact_urls (contact_id);

CREATE TABLE contact_impps (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    contact_id INTEGER NOT NULL REFERENCES contacts(id) ON DELETE CASCADE,
    value      TEXT    NOT NULL,
    type       TEXT    NOT NULL DEFAULT 'other' CHECK (type IN ('home', 'work', 'mobile', 'other')),
    is_primary BOOLEAN NOT NULL DEFAULT 0
);

CREATE INDEX idx_contact_impps_contact ON contact_impps (contact_id);

CREATE TABLE contact_categories (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    contact_id INTEGER NOT NULL REFERENCES contacts(id) ON DELETE CASCADE,
    value      TEXT    NOT NULL
);

CREATE INDEX idx_contact_categories_contact ON contact_categories (contact_id);
