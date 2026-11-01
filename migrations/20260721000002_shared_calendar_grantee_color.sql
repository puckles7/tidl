-- Per-viewer color override for a shared calendar: lets a grantee pick their
-- own display color without touching the owner's calendar row (and thus
-- without affecting the owner or any other grantee's view of it).
ALTER TABLE calendar_shares ADD COLUMN grantee_color TEXT;
