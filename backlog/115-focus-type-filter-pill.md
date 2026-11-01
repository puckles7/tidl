# Merge Tasks and Await into Focus with an All/Calendar/Tasks/Await filter

Focus already ranks calendar events, tasks, and awaits together by urgency, so
the separate **Tasks** and **Await** screens are redundant top-level
destinations. Collapse them into Focus and let a pill — styled like the
Calendar Day/Work week/Week/Month and Focus Today/3 days/7 days segmented
controls — switch the list between `All`, `Calendar`, `Tasks`, and `Await`.

## Dependencies

- [19 - Tasks view](19-tasks-view.md) — the screen being folded into Focus
- [20 - Tickler view](20-tickler-view.md) — the screen being folded into Focus
- [21 - Focus view](21-focus-view.md) — the destination for both

## Functional Requirements

- Remove the `Tasks` and `Await` nav links and their `/tasks` / `/await`
  routes.
- Add an `All / Calendar / Tasks / Await` filter pill to the Focus header,
  reusing the `horizon-btn` segmented-control styling.
- `All` shows every item kind ranked by urgency (current Focus behaviour);
  `Calendar` / `Tasks` / `Await` show only that kind.
- The existing Today / 3 days / 7 days horizon keeps applying in every filter
  mode.
- Preserve task creation: a `+ Add task` button appears in the Focus header in
  `All` and `Tasks` modes, opening the same "New task" dialog the old Tasks
  screen used.
- The section-header label and empty-state message reflect the active filter.
- A CalDAV task no longer offers a "go to source" action (its old target was
  the removed Tasks screen); markdown/note-origin items keep theirs.

## Open Questions

- None.
