# Markdown viewer swallows task tags after a single-line backtick span

The rendered markdown view stopped rendering task tags that appeared later in a
file whose prose uses a triple-backtick code span on a single line, e.g.
`` ```systemctl start cbs``` ``. The tag scanner's fence tracker opened a fenced
code block on that line and never closed it, so every following line — including
`#todo/...` tags — was copied through as literal code and never rendered as a
chip.

## Functional Requirements

- A backtick run that is closed later on the same line is an inline code span,
  not an opening fence, and must not leave the fence tracker open.
- Real fenced code blocks (opening fence on one line, closing fence on a later
  line) still leave tags inside them literal.

## Resolution

`process_task_tags` now recognises that a backtick fence's info string may not
contain a backtick (CommonMark), so when the remainder of the line after the
opening run contains another backtick it is treated as an inline code span
rather than a fence opening. A regression test renders the reproducer and
asserts the trailing `#todo` becomes a chip.
