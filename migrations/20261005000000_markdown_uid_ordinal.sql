-- Markdown task/await events now key their uid by the tag's ordinal within its
-- line: `md:{source_id}:{rel_path}:{line}:{ordinal}` (was
-- `md:{source_id}:{rel_path}:{line}`). Two tags on one line previously produced
-- the same uid, so `INSERT OR REPLACE` (unique on username+calendar+uid) silently
-- dropped all but the last tag — the first was missing from Focus and CalDAV.
--
-- 1) Focus suppressions are keyed by the item's uid, so a suppressed markdown
--    task/await would otherwise resurface once the uid gains its `:ordinal`
--    suffix. Rewrite those keys in place. At migration time the old events are
--    still present (the sync hasn't run yet), so joining on the current uid
--    identifies exactly the markdown suppression keys. Appending `:0` is exact
--    for the single-tag-per-line case (the overwhelmingly common one); a
--    multi-tag line's survivor was its last tag, so its suppression may still
--    resurface — acceptable for a best-effort, one-time repair.
UPDATE OR IGNORE focus_suppressions
SET item_key = item_key || ':0'
WHERE EXISTS (
    SELECT 1 FROM events e
    WHERE e.source = 'markdown' AND e.uid = focus_suppressions.item_key
);

-- 2) Existing event rows still carry the old 3-segment uid. The sync loop only
--    reprocesses a file whose content hash changed, so unchanged files would
--    keep their old uids (and old, colliding behaviour). Clear the per-file
--    hashes so every enabled markdown source is re-walked on the next sync
--    tick; each file's events are deleted and re-inserted with the new uids
--    (see `markdown_sync::sync_source`). Events for sources that are disabled
--    or whose base path is unavailable are left as-is — harmless, since nothing
--    recreates them until the source is reachable again.
DELETE FROM sync_state;
