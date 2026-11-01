-- Add show_sidebar_hints preference to user_preferences table.
-- Toggles the small "click to show/hide" hint text in sidebar panels.

ALTER TABLE user_preferences ADD COLUMN show_sidebar_hints INTEGER NOT NULL DEFAULT 1;
