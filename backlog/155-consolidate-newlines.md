# Consolidated note newlines are swallowed in markdown

Consolidating a note to a markdown source preserved the note's `\n` characters
verbatim, but markdown treats a single newline as a *soft* break that renders
as a space. A note with a list of words on separate lines therefore rendered as
one run-on sentence in the new `.md` file (and in tidl's own markdown preview).

## Functional Requirements

- A single newline in a note body becomes a CommonMark hard break in the
  consolidated file, so the note's visual line structure survives rendering.
- Blank lines, lines already ending in a hard break, and fenced code blocks are
  left untouched.
- CRLF input is preserved as CRLF.

## Resolution

`build_markdown_document` now runs the image-rewritten body through
`preserve_line_breaks`, which appends the two-space hard-break marker to every
non-empty line ending in a single newline (unless it already has one or is
inside a fenced code block). Unit tests cover single newlines, existing hard
breaks, fences, and CRLF.
