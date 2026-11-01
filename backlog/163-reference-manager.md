# Reference manager — books, articles, etc. (Zotero light)

A lightweight, self-hosted reference-keeping system for books, articles, papers,
web pages and similar sources: capture a reference once, keep its metadata
(title, authors, year, publisher/venue, DOI/ISBN/URL, tags, notes, maybe a PDF
attachment) and be able to find it again and cite/link to it from notes and
calendar items. Deliberately "Zotero light" — the goal is a small, hackable
store that lives in the same SQLite database and UI as everything else, not a
full citation-manager replacement.

## Context / Background

tidl already stores contacts, notes, calendars and markdown sources. References
are a natural sibling to contacts: structured, shareable, searchable records
that other parts of the app can point at (e.g. `@cite` in notes, linking a book
to a reading task or project). A full Zotero clone is out of scope; the value is
capture + search + linking, with import/export for interoperability.

## Functional Requirements

- Models for references: at minimum book, article/paper, web page; extensible
  type field rather than one table per type.
- Core metadata: title, authors/editors, year/date, container (journal,
  publisher, site), identifiers (ISBN, DOI, URL), abstract, tags, notes.
- CRUD UI in the app (list, search, detail/edit view), reusing existing card/
  grid and search patterns where sensible.
- Capture via DOI/ISBN lookup (optional online metadata fetch) and manual entry.
- Import/export: BibTeX and/or RIS; consider importing Zotero exports.
- Optional file attachment (PDF) using the existing in-DB BLOB attachment
  approach from notes (#110).
- Linking: reference notes from authored notes/markdown, and treat references as
  first-class in global search.
- Sharing model mirroring notes/contacts (per-user, optionally shared
  collections).

## Open Questions

- Separate "reference" entity + tab, or a new kind of note/collection?
- Citation keys: auto-generated (`authorYear`) vs. user-defined?
- How much of Zotero's model (collections, tags, attachments, CSL styles) do we
  actually need before it stops being "light"?
- Online metadata lookup: which APIs (Crossref, OpenLibrary) and do we want a
  network dependency at all, or import-only?
- Do we need citation formatting (CSL/BibTeX output inline in notes), or is
  export enough?
- Sync semantics if shared — is this CardDAV-like read/write sync of interest,
  or purely in-app?
