# Backlog

Feature wishes and ideas for future development. Each item links to a detailed description.

| #   | Title                                                                                                                                      | Status      | Depends on                        |
|-----|--------------------------------------------------------------------------------------------------------------------------------------------|-------------|-----------------------------------|
| 1   | [Work calendar integration](01-work-calendar-integration.md)                                                                               | done        | #3                                |
| 2   | [User management](02-user-management.md)                                                                                                   | done        | —                                 |
| 3   | [Multi-calendar support](03-multi-calendar.md)                                                                                             | done        | #2                                |
| 4   | [Consolidate duplicated VALARM parser](04-arch-valarm-consolidation.md)                                                                    | done        | —                                 |
| 5   | [Unify ETag algorithm](05-arch-etag-unification.md)                                                                                        | done        | —                                 |
| 6   | [Separate wire types from server logic in api.rs](06-arch-api-separation.md)                                                               | done        | —                                 |
| 7   | [Add CalendarStore trait and separate SessionStore](07-arch-calendar-store-trait.md)                                                       | done        | —                                 |
| 8   | [Refactor caldav_handlers.rs](08-arch-handlers-refactor.md)                                                                                | done        | —                                 |
| 9   | [Replace glob re-exports in tidl-caldav/lib.rs](09-arch-lib-exports.md)                                                                    | done        | —                                 |
| 10  | [Calendar import (.ics)](10-calendar-import.md)                                                                                            | done        | —                                 |
| 11  | [Move Sources config into Settings modal](11-settings-sources.md)                                                                          | done        | —                                 |
| 12  | [Notifications](12-notifications.md)                                                                                                       | done        | #13                               |
| 13  | [Calendar sharing](13-calendar-sharing.md)                                                                                                 | done        | #14                               |
| 14  | [Settings dialog tabs](14-settings-tabs.md)                                                                                                | done        | —                                 |
| 15  | [Webcal URL subscriptions](15-webcal-subscriptions.md)                                                                                     | done        | #14                               |
| 16  | [All-day and multi-day event display](16-allday-multiday-events.md)                                                                        | done        | —                                 |
| 17  | [Self-registration / sign-up flow](17-signup-flow.md)                                                                                      | done        | #2, #13                           |
| 18  | [Markdown source files for todos and pending items](18-markdown-sources.md)                                                                | done        | —                                 |
| 19  | [Tasks view — hifi redesign](19-tasks-view.md)                                                                                             | done        | #18                               |
| 20  | [Tickler view — hifi redesign](20-tickler-view.md)                                                                                         | done        | #18                               |
| 21  | [Focus view — urgency-ranked unified list](21-focus-view.md)                                                                               | done        | #18, #3                           |
| 22  | [iMIP inbound email — meeting invitation import](22-imip-inbound-email.md)                                                                 | done        | —                                 |
| 23  | [Open in editor — Markdown preview modal](23-open-in-editor.md)                                                                            | done        | —                                 |
| 24  | [Calendar view toggle pill](24-calendar-view-pill.md)                                                                                      | done        | —                                 |
| 25  | [Outlook COM bridge](25-outlook-com-bridge.md)                                                                                             | done        | #1                                |
| 26  | [Recurring event expansion (RRULE)](26-recurring-events.md)                                                                                | done        | —                                 |
| 27  | [Architecture cleanup](27-arch-cleanup.md)                                                                                                 | done        | —                                 |
| 28  | [Explore rustical OIDC + Argon2 auth](28-explore-rustical-auth.md)                                                                         | done        | —                                 |
| 29  | [CardDAV protocol — read-write contact sync](29-carddav-protocol.md)                                                                       | done        | #79                               |
| 30  | [Investigate WebDAV Push](30-investigate-webdav-push.md)                                                                                   | dropped     | —                                 |
| 31  | [DAVx5 compatibility verification and fixes](31-davx5-compatibility.md)                                                                    | done        | —                                 |
| 32  | [Color palette refinement — muted, dark-mode-friendly source colors](32-color-palette-refinement.md)                                       | done        | —                                 |
| 33  | [Wikilink navigation — Roam/markdown-memo style](33-wikilink-navigation.md)                                                                | done        | —                                 |
| 34  | [Git-backed markdown sources with in-app editor](34-git-backed-markdown-sources.md)                                                        | done        | —                                 |
| 35  | [Recurring event authoring (RRULE UI)](35-recurring-event-authoring.md)                                                                    | done        | #26                               |
| 36  | [Focus view — proximity-based calendar event urgency](36-focus-proximity-urgency.md)                                                       | done        | #21                               |
| 37  | [Notes tab — collections, ad-hoc notes, to-do/tickler coupling](37-notes-tab.md)                                                           | done        | #2, #3                            |
| 38  | [Todos/Focus/Tickler note integration](38-notes-integration.md)                                                                            | done        | #37, #19, #20, #21                |
| 39  | [Mirrored VTODO description + note/calendar sharing-mismatch warning](39-notes-vtodo-description-sharing-warning.md)                       | done        | #37                               |
| 40  | [Note to-do completion write-back + audit log](40-note-todo-completion-writeback.md)                                                       | done        | #37                               |
| 41  | [Make calendar shares actually usable (web UI + raw CalDAV)](41-caldav-share-exposure.md)                                                  | done        | #13                               |
| 42  | [Note ordering — persisted manual order + pinned-group invariant](42-note-order-pinned-invariant.md)                                       | done        | #37                               |
| 43  | [Drag-and-drop note reordering](43-note-drag-reorder.md)                                                                                   | done        | #37, #42                          |
| 44  | [Settings → Sharing: manage note-collection sharing](44-settings-sharing-note-collections.md)                                              | done        | #13, #37                          |
| 45  | [Notes grid — size cards by their own content, not row-mates](45-note-grid-content-sizing.md)                                              | done        | #37                               |
| 46  | [Restyle filter sidebar to match Notes tab sidebar](46-filter-sidebar-notes-style-parity.md)                                               | done        | #37                               |
| 47  | [Notes free-form grid layout + auto-layout](47-notes-free-grid-layout.md)                                                                  | done        | #37, #42, #43                     |
| 48  | [Notes grid — variable-height masonry via row-span units](48-notes-grid-row-span-units.md)                                                 | done        | #47                               |
| 49  | [TASK/AWAIT kind unification](49-task-await-unification.md)                                                                                | done        | #37, #38, #18, #19, #20, #21, #40 |
| 50  | [Notes grid — responsive column count (no horizontal scroll)](50-notes-responsive-columns.md)                                              | done        | #37, #42, #43, #47, #48           |
| 51  | [Notes toolbar not responsive below ~390px (fixed-width search overflows)](51-notes-toolbar-narrow-overflow.md)                            | done        | #37                               |
| 52  | [Admin user management UI](52-admin-user-management-ui.md)                                                                                 | done        | #2                                |
| 53  | [Unify calendar-list sidebar + live share/calendar updates](53-unified-calendar-list-live-updates.md)                                      | done        | #41                               |
| 54  | [User profile editor](54-user-profile-editor.md)                                                                                           | done        | —                                 |
| 55  | [Let a grantee leave a shared calendar](55-grantee-leave-shared-calendar.md)                                                               | done        | #13, #41                          |
| 56  | [Let a grantee customize and (write-)clear a shared calendar](56-grantee-shared-calendar-edit.md)                                          | done        | #13, #41                          |
| 57  | [Reshare a calendar received via a share](57-reshare-shared-calendar.md)                                                                   | dropped     | #13, #56                          |
| 58  | [Note collection picker — create-new + remembered default](58-note-collection-default-picker.md)                                           | done        | #37                               |
| 59  | [Shared calendar events don't push live to grantees](59-shared-calendar-event-live-refresh.md)                                             | done        | #13, #41                          |
| 60  | [Remove duplicate /auth/login and /auth/logout routes](60-remove-duplicate-auth-routes.md)                                                 | done        | —                                 |
| 61  | [Support generic CalDAV servers for remote sync (not just tidl)](61-generic-caldav-remote-source-support.md)                               | dropped     | —                                 |
| 62  | [Note items need a "go to source" arrow in Focus, Tasks, and Await lists](62-note-goto-source-arrow.md)                                    | done        | #37, #38, #21, #19, #20           |
| 63  | ["Show in collection" button on the note-goto modal](63-note-goto-show-in-collection.md)                                                   | done        | #62                               |
| 64  | [Note-collection sharing via email invite, closing username enumeration](64-note-collection-email-invite-sharing.md)                       | done        | #13, #37, #44                     |
| 65  | [Sidebar share icon for calendars & note collections → Settings/Sharing](65-sidebar-share-icon.md)                                         | done        | #13, #41, #44                     |
| 66  | [Note titles](66-note-titles.md)                                                                                                           | done        | —                                 |
| 67  | [Declarative SMTP configuration (env vars + NixOS module)](67-declarative-smtp-config.md)                                                  | done        | —                                 |
| 68  | [Configurable priority curves](68-configurable-priority-curves.md)                                                                         | done        | —                                 |
| 69  | [Focus: permanent item suppression](69-focus-permanent-suppression.md)                                                                     | done        | —                                 |
| 70  | [Markdown-sourced tasks aren't exposed via CalDAV at all](70-markdown-tasks-caldav-exposure.md)                                            | dropped     | #18                               |
| 71  | [External CalDAV completion/deletion policy for note-linked tasks](71-external-caldav-completion-deletion-policy.md)                       | done        | —                                 |
| 72  | [Event editor — mouse-based time selection](72-event-editor-mouse-time-selection.md)                                                       | done        | —                                 |
| 73  | [Server-side EMAIL alarm dispatch (background job)](73-alarm-email-dispatch.md)                                                            | done        | #12, #13                          |
| 74  | [Flash the task's rendered pill when opening a markdown task/await preview](74-markdown-preview-line-flash.md)                             | done        | #23, #62                          |
| 75  | [Markdown file browser / fuzzy file search](75-markdown-file-browser-search.md)                                                            | done        | #18, #34                          |
| 76  | [Fix reactive-tracking warning when seeding markdown preview nav_stack](76-focus-nav-stack-tracking-warning.md)                            | done        | —                                 |
| 77  | [Cache the wasm-side dependency build in the crane Nix package too](77-crane-wasm-deps-caching.md)                                         | dropped     | —                                 |
| 78  | [Note card — delete button without opening the note](78-note-card-delete-button.md)                                                        | done        | #37                               |
| 79  | [Contacts data model + Contacts tab (CRUD)](79-contacts-data-model-tab.md)                                                                 | done        | —                                 |
| 80  | [@mention contact linking in notes](80-note-contact-mention.md)                                                                            | done        | #79                               |
| 81  | [Contacts sharing](81-contacts-sharing.md)                                                                                                 | done        | #79                               |
| 82  | [Contacts — drag-to-reorder, shared with Notes' drag system](82-contacts-drag-reorder.md)                                                  | done        | #79                               |
| 83  | [`move_notes`/`move_contacts` can hit "database is locked" under near-simultaneous writes](83-drag-move-notes-database-locked.md)          | done        | —                                 |
| 84  | [Contacts — auto-layout should order cards alphabetically](84-contacts-alpha-auto-layout.md)                                               | done        | #79, #82                          |
| 85  | [CardDAV interoperability verification — iOS Contacts, DAVx5](85-carddav-ios-davx5-verification.md)                                        | done        | #29                               |
| 86  | [CardDAV/Contacts — photo (PHOTO) support](86-carddav-contact-photos.md)                                                                   | done        | #29, #79                          |
| 87  | [Contacts import (vCard/CSV)](87-contacts-import.md)                                                                                       | done        | #79                               |
| 88  | [Merge duplicate contacts](88-contacts-merge.md)                                                                                           | done        | #79                               |
| 89  | [Contacts export (vCard/VCF)](89-contacts-export-vcf.md)                                                                                   | done        | #79                               |
| 90  | [Clear all buttons for Contacts and Notes](90-contacts-notes-clear-all.md)                                                                 | done        | #37, #78, #79                     |
| 91  | [@mention contact linking in the markdown editor](91-markdown-editor-contact-mention.md)                                                   | done        | #79, #80                          |
| 92  | [Contact card URL should be clickable](92-contact-card-url-clickable.md)                                                                   | done        | #79                               |
| 93  | [Lock the Auto-layout toggle while searching](93-lock-autolayout-toggle-while-searching.md)                                                | done        | #37, #79                          |
| 94  | [Redesign sidebar pills (CalendarPill) to match new mockup](94-sidebar-pill-redesign.md)                                                   | done        | #37, #46, #79                     |
| 95  | [Redesign Notes/Contacts toolbar buttons and unify the trash icon](95-toolbar-icon-redesign.md)                                            | done        | #37, #79, #90                     |
| 96  | [Calendar vertical zoom / smarter default scroll position](96-calendar-vertical-zoom-autoscroll.md)                                        | done        | —                                 |
| 97  | [Auto-layout should restore manual placement when toggled back off](97-autolayout-restore-manual-on-toggle-off.md)                         | done        | #37, #79, #82, #93                |
| 98  | [@mention popup mispositioned when note text wraps](98-mention-popup-caret-position-bug.md)                                                | done        | #80                               |
| 99  | [Commit auto-layout result as the new manual layout](99-commit-auto-layout-to-manual.md)                                                   | dropped     | #37, #79, #47, #97                |
| 100 | [Reposition the merge-select checkbox on contact cards](100-contact-card-merge-checkbox-placement.md)                                      | done        | #88                               |
| 101 | [Contact search should match more fields (URLs, notes, etc.)](101-contact-search-more-fields.md)                                           | done        | #79                               |
| 102 | [Cohesion-aware grid reflow (gaggle-preserving resize)](102-cohesion-aware-grid-reflow.md)                                                 | done        | #37, #79, #47, #97, #99           |
| 103 | [Regression-test and fix the cards-grid drag/resize/visibility invariants](103-cards-layout-regression-suite.md)                           | done        | #102                              |
| 104 | [Manual-mode drag shadow snaps to the wrong column mid-drag](104-manual-drag-shadow-wrong-column.md)                                       | done        | #102, #103                        |
| 105 | [Manual mode recomputed cluster reflow on every render, not just resize](105-manual-mode-recomputes-reflow-every-render.md)                | done        | #102, #104                        |
| 106 | [Manual-mode cluster cache confused real (storage) columns with render (view) columns](106-cluster-cache-real-vs-view-column-confusion.md) | done        | #102, #104, #105                  |
| 107 | [column_offsets orders colliding columns by raw index, not content](107-column-offsets-index-order-not-geometric.md)                       | done        | #106                              |
| 108 | [Resolve webcal sync duplication](108-resolve-webcal-sync-duplication.md)                                                                  | done        | —                                 |
| 109 | [Google Keep import — ingest Keep exports as notes](109-google-keep-import.md)                                                             | done        | #37, #110                         |
| 110 | [Note image / attachment support (in-DB BLOB storage)](110-note-image-attachments.md)                                                      | done        | #37                               |
| 111 | [Clickable URLs in notes](111-note-url-autolinking.md)                                                                                     | done        | #37, #80                          |
| 112 | [Rendered markdown task checkboxes write #done back to the source](112-rendered-task-checkbox.md)                                          | done        | #18, #23                          |
| 113 | [Inline tasks and awaits in notes](113-note-inline-tasks.md)                                                                               | done        | #37, #49, #18, #110               |
| 114 | [Note labels (and Google Keep `labels[]` import)](114-note-labels.md)                                                                      | done        | #37, #109                         |
| 115 | [Merge Tasks and Await into Focus with an All/Calendar/Tasks/Await filter](115-focus-type-filter-pill.md)                                  | done        | #19, #20, #21                     |
| 116 | [Task/await chip UI — subtle dot urgency in notes and markdown](116-task-await-chip-ui.md)                                                 | done        | #113, #112, #68                   |
| 117 | [Markdown checked-off tasks keep their data and can be re-opened](117-markdown-done-roundtrip.md)                                          | done        | #112, #113, #18                   |
| 118 | [Manage ad-hoc tasks/awaits: edit/delete via the card arrow, split +Task/+Await](118-adhoc-task-await-management.md)                       | done        | #115, #49                         |
| 119 | [Consistent Calendar/Focus headers: date, centred pills, right-aligned search](119-consistent-screen-headers.md)                           | done        | #115, #118                        |
| 120 | [Cap the join gap when narrow columns compact](120-compact-gap-cap.md)                                                                     | done        | #102, #106, #107                  |
| 121 | [Clicking a rendered task/await pill opens its task/await modal](121-rendered-pill-opens-modal.md)                                         | done        | #112, #116, #118                  |
| 122 | [Consolidate a note to a markdown source (and vice versa)](122-note-markdown-consolidation.md)                                             | done        | #18, #34, #37, #110               |
| 123 | [ESC closes an open modal](123-esc-closes-modal.md)                                                                                        | done        | —                                 |
| 124 | [Clean-up for completed tasks](124-done-task-cleanup.md)                                                                                   | done        | #112, #117, #40                   |
| 125 | [Central done-item tracking with optional source removal](125-central-done-tracking.md)                                                    | done        | #124, #112, #117, #40             |
| 126 | [Note editor content box is too small; compress lower fields, hide hints behind info icons](126-note-editor-content-box.md)                | done        | #37, #66, #113                    |
| 127 | [Per-entity color palettes (no cross-type overlap)](127-per-entity-color-palettes.md)                                                      | done        | #32                               |
| 128 | [Label colour picker + create-from-sidebar](128-label-color-picker-sidebar-create.md)                                                      | done        | #114, #127                        |
| 129 | [Edit (and create) task/await tokens in the note and markdown editors](129-editor-token-click-edit.md)                                     | done        | #113, #121, #91                   |
| 130 | [Note bullet lists + convert imported Keep checklists to bullets](130-note-bullet-lists.md)                                                | done        | —                                 |
| 131 | [gts cargo leptos watch hot-reload broken in a worktree](131-gts-watch-worktree-hot-reload.md)                                             | done        | —                                 |
| 132 | ["Mark as done" in the edit task modal](132-mark-done-in-edit-modal.md)                                                                    | done        | —                                 |
| 133 | [Done — archive a single item from the list](133-done-remove-single-item.md)                                                               | done        | #124, #125                        |
| 134 | [Go-to-source for a note task should flash the task pill, not the note card](134-note-goto-flash-pill.md)                                  | done        | #62, #125                         |
| 135 | [Auto-archive completed items per source](135-auto-remove-completed-per-source.md)                                                         | done        | #124, #125                        |
| 136 | [Index external CalDAV completions/deletions immediately](136-external-caldav-done-hook.md)                                                | done        | #124, #71, #125                   |
| 137 | [Done tab: live count badge in the navbar](137-done-tab-count-badge.md)                                                                    | done        | #125                              |
| 138 | [Note age — human-friendly units (weeks/months/years)](138-note-relative-time-granularity.md)                                              | done        | —                                 |
| 139 | [Google Keep import — shared-note collections optional](139-keep-shared-collections-optional.md)                                           | done        | —                                 |
| 140 | [Inline note images — hover dim + fullscreen lightbox](140-note-image-lightbox.md)                                                         | done        | —                                 |
| 141 | [Code review findings (2026-10-11 full-project pass)](141-code-review-findings-2026-10-11.md)                                              | done        | —                                 |
| 142 | [Upgrade quick-xml past its DoS vulnerabilities](142-quick-xml-dos-vulnerability.md)                                                       | done        | —                                 |
| 143 | [Alternative packaging & distribution strategies](143-packaging-distribution-strategy.md)                                                  | done        | —                                 |
| 144 | [Architecture quality assessment (2026-10-14 full-project pass)](144-architecture-quality-assessment.md)                                   | done        | —                                 |
| 145 | [Bump rustls past RUSTSEC-2026-0346…0349 (and drop the yanked spin)](145-rustls-advisory-bump.md)                                          | done        | —                                 |
| 146 | [Python e2e fixture: login 200 but follow-up server fns "Not authenticated"](146-e2e-fixture-auth-failure.md)                              | done        | —                                 |
| 147 | [Versioning scheme and changelog](147-versioning-and-changelog.md)                                                                         | done        | —                                 |
| 148 | [CI build & publish release artifacts](148-ci-release-artifacts.md)                                                                        | done        | #147, #149                        |
| 149 | [OCI / Docker image + docker-compose + deploy docs](149-oci-docker-image.md)                                                               | done        | #147                              |
| 150 | [Native distro packages (.deb/.rpm/AUR + systemd)](150-native-distro-packages.md)                                                          | dropped     | #147, #148                        |
| 151 | [macOS distribution (Homebrew tap / signed app)](151-macos-distribution.md)                                                                | dropped     | #147, #148                        |
| 152 | [NAS / alternative-runner templates (unRAID, TrueNAS, podman)](152-nas-runner-templates.md)                                                               | done        | #149                              |
| 153 | [Graceful shutdown — checkpoint the SQLite WAL on stop](153-graceful-shutdown.md)                                                          | done        | —                                 |
| 154 | [Secret-generation CLI convenience (generate-secret)](154-generate-secret-cli.md)                                                          | done        | —                                 |
| 155 | [Consolidated note newlines are swallowed in markdown](155-consolidate-newlines.md)                                                        | done        | —                                 |
| 156 | [Calendar event priority attenuation](156-event-priority-attenuation.md)                                                                   | done        | #21, #36, #68, #70                |
| 157 | [Focus event preview opens the edit-event dialog (with Show in calendar)](157-focus-event-edit-dialog.md)                                  | done        | —                                 |
| 158 | [Show why a Focus item has its current urgency](158-focus-urgency-explanation.md)                                                          | done        | —                                 |
| 159 | [Screen toolbars wrap on narrow windows and stay visible on mobile](159-screen-toolbar-wrap-mobile.md)                                     | done        | —                                 |
| 160 | [Mobile filter bars: consistent collapsed card and top alignment](160-mobile-filter-bar-consistency.md)                                     | done        | —                                 |
| 161 | [Markdown viewer swallows task tags after a single-line backtick span](161-markdown-fence-single-line-swallows-tags.md)                    | done        | —                                 |
| 162 | [Mark a task done from the markdown editor's edit dialog](162-markdown-editor-task-done-checkbox.md)                                       | done        | #129                              |
| 163 | [Reference manager — books, articles, etc. (Zotero light)](163-reference-manager.md)                                                        | idea        | —                                 |
| 164 | [Streamline local/remote handling for opening a file in an external editor](164-open-in-editor-local-remote-consolidation.md)              | done        | #23                               |
| 165 | [Compress web assets (gzip/brotli) to speed up app startup](165-asset-compression.md)                                                      | done        | —                                 |
