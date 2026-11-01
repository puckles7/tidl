-- Add a display colour to markdown sources (mirrors calendar colours).
ALTER TABLE markdown_sources ADD COLUMN color TEXT NOT NULL DEFAULT '#8b5cf6';
