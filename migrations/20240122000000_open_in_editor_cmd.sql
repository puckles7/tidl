-- Add open_in_editor_cmd preference to user_preferences table.
-- NULL means "use xdg-open fallback".

ALTER TABLE user_preferences ADD COLUMN open_in_editor_cmd TEXT;
