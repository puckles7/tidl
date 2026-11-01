# Contact search should match more fields (URLs, notes, etc.)

The Contacts search box only filters on `display_name`, `org`, `emails`, and `phones`. It misses several fields that are visible on the card and plausible things to search for — most notably URLs, but also notes, nickname, and the other text fields a contact can carry.

## Dependencies

- [79 - Contacts data model + Contacts tab (CRUD)](79-contacts-data-model-tab.md)

## Context / Background

- The filter lives in `crates/tidl-web/src/components/contacts/mod.rs` (~line 203-218): `contacts.retain(...)` checks `display_name`, `org`, `emails[].value`, `phones[].value` only, all lowercased `.contains(&query)`.
- `ContactDto` (`crates/tidl-web/src/types.rs:983`) has several other text-bearing fields not included in the check: `nickname`, `title`, `notes`, `categories`, `urls` (`Vec<ContactMethod>`, same shape as `emails`/`phones`), `impps`, and `addresses` (`Vec<ContactAddress>`).

## Functional Requirements

- Extend the search predicate to also match: `urls[].value`, `notes`, `nickname`, `title`, `categories`, `impps[].value`, and `addresses` (at least the free-text-ish parts, e.g. street/city — needs a look at `ContactAddress`'s fields).
- Keep the same case-insensitive substring match already used for the existing fields, for consistency.
- Given/family/middle/prefix/suffix name parts are effectively covered already since `display_name` is typically composed from them — check whether that's true in all cases (e.g. a contact with only a `family_name` set) before deciding whether to add them explicitly too.

## Open Questions

- None — scope is “match what's already on the card, not just the four fields currently wired up.”

## Resolution

Implemented as a pure, unit-tested predicate `contact_matches_search(&ContactDto, &str)`
in `crates/tidl-web/src/util.rs` (alongside `filter_contacts_for_mention`), replacing
the inline four-field `retain` closure in `components/contacts/mod.rs`.

- Matches, case-insensitively as a substring: `display_name`; the explicit name parts
  (`given`/`family`/`middle`/`prefix`/`suffix` — included even though `display_name` is
  usually composed from them, since the editor lets them diverge); `nickname`, `org`,
  `title`, `notes`; each `categories` entry; every `emails`/`phones`/`urls`/`impps` value;
  and every postal-address line (`street`/`city`/`region`/`postal_code`/`country`).
- Address `kind` and the date fields (`birthday`/`anniversary`) are deliberately excluded —
  they're classifications/dates, not free text someone would search by.
- Seven unit tests cover empty-query, case-insensitivity, non-match, the scalar text
  fields, name parts absent from `display_name`, the method lists (incl. URLs/IMPPs), and
  address lines. `README.md`'s Contacts section updated accordingly.

No deviations from the spec.
