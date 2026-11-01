-- Add dark_mode preference to user_preferences table.

ALTER TABLE user_preferences ADD COLUMN dark_mode INTEGER NOT NULL DEFAULT 0;
