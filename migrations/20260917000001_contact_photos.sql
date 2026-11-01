-- Photo support for contacts (#86), the last major vCard PHOTO property #29
-- deferred for lack of blob storage. Stored as a BLOB column on `contacts`
-- itself (not a separate table/file) — atomic with the contact row, no
-- orphan-cleanup concerns, trivial at personal-address-book scale.

ALTER TABLE contacts ADD COLUMN photo BLOB;
ALTER TABLE contacts ADD COLUMN photo_content_type TEXT;
