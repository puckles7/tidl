-- Add source_line and topic columns to events for markdown-sourced todos.

ALTER TABLE events ADD COLUMN source_line INTEGER;
ALTER TABLE events ADD COLUMN topic TEXT;
