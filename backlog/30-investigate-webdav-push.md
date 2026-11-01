# Investigate WebDAV Push

WebDAV Push is a server-to-client notification mechanism used by DAVx5 and other sync clients to achieve near-instant sync without polling. This item is a research and feasibility spike.

## Background

Without WebDAV Push, CalDAV clients poll the server on a fixed interval (typically every 15–30 minutes on mobile to save battery). With WebDAV Push, a client registers a callback URL with the server; the server POSTs a notification to that URL whenever a tracked collection changes. Result: changes (e.g. a new meeting invite) appear on the client within seconds, and mobile clients don't need to poll at all.

This is especially valuable for shared calendars — when someone books a meeting with you, you want it on your phone immediately, not at the next poll cycle.

## How It Works (draft spec)

1. Client sends a `SUBSCRIBE` request to a calendar collection, providing a callback URL and desired push transport (e.g. Web Push, UnifiedPush)
2. Server stores the subscription
3. On any write to that collection, server POSTs a notification to the callback URL
4. Client re-syncs on receipt

## Functional Requirements (spike only — no implementation yet)

- Research the WebDAV Push draft spec: registration flow, notification format, transport options
- Assess stability of the draft — is it safe to implement against or still in flux?
- Identify what tidl's server would need to store (subscriptions table) and the outbound HTTP requirement
- Evaluate UnifiedPush as a transport (self-hosted, no Google dependency)
- Produce a written recommendation: implement, defer, or drop

## Open Questions

- WebDAV Push requires outbound HTTP from the server — any firewall/NAT implications for self-hosters?
- Is UnifiedPush the right transport, or should this support multiple transports?
- Is the draft spec stable enough to commit to, or should this wait?
