# Reshare a calendar received via a share

Let a grantee who has a calendar shared with them extend it to a third
person, instead of only the original owner being able to invite people.
Deliberately split out of [56 - Let a grantee customize and (write-)clear a
shared calendar](56-grantee-shared-calendar-edit.md) — the permission-cascade
questions need their own design pass before this is scoped further.

## Dependencies

- [13 - Calendar sharing](13-calendar-sharing.md)
- [56 - Let a grantee customize and (write-)clear a shared calendar](56-grantee-shared-calendar-edit.md)

## Open Questions

- Can a `read`-only grantee reshare at all, or only `read-write`?
- Does a reshared grant chain back to the original owner (so `calendar_shares.owner_username`
  stays the real owner, and the resharer is just another `grantee_username`
  row), or does the resharer become an intermediate owner? The former keeps
  the existing owner-scoped revoke/color/clear logic working unmodified but
  means the "owner" listed to the new grantee isn't who actually shared it
  with them; the latter requires walking a chain everywhere permission is
  checked today.
- Does the original owner get visibility into (or veto over) who a grantee
  reshares their calendar with?
- Can the resharer's own grant be revoked out from under the people they
  reshared with, and if so what happens to those downstream grants?

## Why dropped

Parked 2026-08-31. The permission-cascade design questions above are
unresolved and there's no current demand for reshare. Revive this entry if
grantee-initiated resharing becomes a real need.
