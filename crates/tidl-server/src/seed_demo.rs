//! Synthetic demo data for a disposable test server (the `gts`/"generate
//! test server" dev alias in `flake.nix`) — four contact collections (the
//! default "My Contacts" plus three more, each its own curated color), three
//! colored calendars with near-future events (some recurring), and three
//! Notes collections: Reminders (dated tasks/awaits of varying priority and
//! due date), Recipes (each carrying a real dish photo pulled from Wikimedia
//! Commons), and Journal (plain notes `@mention`-ing a contact), so a fresh
//! throwaway instance has something to look at immediately. Each note
//! collection's cards are positioned as its own little "island" on the board
//! (non-adjacent column blocks, see `seed_notes`), so the cohesion-aware grid
//! reflow sees three separate gaggles rather than one undifferentiated pile.
//! A recipe photo fetch that fails (offline, rate-limited) falls back to a
//! locally generated gradient so seeding never leaves the board imageless.
//! Dev tooling only — never invoked in production.
//!
//! Contacts and plain calendar events reuse `ContactStore`/`CalendarStore`
//! directly. A task/await note also needs a mirrored VTODO in the `events`
//! table for the Tasks/Tickler/Focus views to pick it up (backlog #38); that
//! mirroring logic (`sync_mirror`) is private to `tidl-web::api::notes`, so
//! the relevant slice of it — building the VTODO ical text and the
//! `events`/`notes` inserts — is reproduced here rather than widening that
//! function's visibility just for a dev script.

use std::sync::Arc;

use anyhow::Result;
use chrono::{Duration, Local, NaiveDate};
use sqlx::{Row, SqlitePool};
use tidl_carddav::{VCardFields, VCardMethod};

use crate::storage::{CalendarStore, ContactStore};

pub async fn seed(
    pool: &SqlitePool,
    calendars: &Arc<dyn CalendarStore + Send + Sync>,
    contacts: &Arc<dyn ContactStore + Send + Sync>,
    username: &str,
) -> Result<()> {
    calendars.ensure_personal_calendar(username).await?;
    // A couple more calendars so the note editor's per-note calendar picker
    // (backlog #113) and the calendar sidebar have something to switch between
    // on a fresh instance (`create_calendar` is a no-op if it already exists).
    let _ = calendars
        .create_calendar(username, "work", Some("Work"))
        .await;
    let _ = calendars
        .create_calendar(username, "family", Some("Family"))
        .await;

    // Each calendar gets a distinct color from the *muted calendar palette*
    // (`--pal-*`, the only colors `CalendarEditModal` can pick), deliberately
    // different from the vivid hexes the three note collections use
    // (`#8b5cf6` violet, `#f59e0b` amber, `#22c55e` green) so a screenshot of
    // the demo instance shows the two color systems side by side.
    for (calendar, color) in [
        ("personal", "var(--pal-1)"), // sky blue
        ("work", "var(--pal-7)"),     // rust
        ("family", "var(--pal-9)"),   // mauve
    ] {
        sqlx::query("UPDATE calendars SET color = ? WHERE username = ? AND name = ?")
            .bind(color)
            .bind(username)
            .bind(calendar)
            .execute(pool)
            .await?;
    }

    let contact_names = seed_contacts(pool, contacts, username).await?;
    seed_events(calendars, username).await?;
    seed_notes(pool, calendars, username, &contact_names).await?;
    seed_done_items(pool, username).await?;

    Ok(())
}

/// ~50 completed calendar items with completion dates spread over the past
/// year, so the Done tab (backlog #124/#125) has a realistic history to page
/// through on a fresh demo instance. Inserted straight into `events` with
/// `status = 'COMPLETED'` — exactly what `reconcile_caldav` reads — rather than
/// through `CalendarStore`, which has no "insert an already-completed VTODO"
/// path.
async fn seed_done_items(pool: &SqlitePool, username: &str) -> Result<()> {
    const DONE_SLUGS: &[&str] = &[
        "pay-the-electricity-bill",
        "book-dentist-appointment",
        "renew-passport",
        "submit-expense-report",
        "review-quarterly-budget",
        "water-the-plants",
        "call-the-plumber",
        "renew-car-insurance",
        "update-emergency-contacts",
        "plan-summer-holiday",
        "back-up-laptop",
        "replace-smoke-detector-battery",
        "schedule-annual-checkup",
        "cancel-old-subscription",
        "return-library-books",
        "fix-leaky-faucet",
        "order-new-glasses",
        "renew-gym-membership",
        "write-thank-you-notes",
        "clear-out-garage",
        "renew-drivers-licence",
        "file-tax-return",
        "service-car",
        "paint-the-hallway",
        "organise-photos",
    ];
    const AWAIT_SLUGS: &[&str] = &[
        "waiting-on-invoice",
        "awaiting-contractor-quote",
        "pending-visa-approval",
        "hoping-for-a-reply",
        "waiting-on-refund",
        "awaiting-test-results",
        "pending-booking-confirmation",
        "waiting-on-shipping",
        "awaiting-sign-off",
        "pending-background-check",
    ];
    let calendars = ["personal", "work", "family"];
    let today = Local::now().date_naive();

    for i in 0..50i64 {
        // Roughly a third are awaits; clock steps back ~a week per item so the
        // completion dates fan out across the last year.
        let is_await = i % 3 == 0;
        let completed = today - Duration::days((i * 7) % 360);
        let calendar = calendars[(i as usize) % calendars.len()];
        let uid = format!("demo-done-{i:03}");
        let etag = format!("etag-demo-done-{i:03}");

        let (slug, summary, kind, priority, due) = if is_await {
            let slug = AWAIT_SLUGS[(i as usize) % AWAIT_SLUGS.len()];
            let due = completed - Duration::days(10);
            (
                slug,
                title_case(slug),
                "await",
                None,
                Some(format!("{} 00:00:00", due.format("%Y-%m-%d"))),
            )
        } else {
            let slug = DONE_SLUGS[(i as usize) % DONE_SLUGS.len()];
            (slug, title_case(slug), "task", Some(i * 7 % 100 + 1), None)
        };

        let ical = format!(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//tidl//demo//EN\r\n\
             BEGIN:VTODO\r\nUID:{uid}\r\nSUMMARY:{summary}\r\n\
             STATUS:COMPLETED\r\nEND:VTODO\r\nEND:VCALENDAR\r\n"
        );

        sqlx::query(
            "INSERT INTO events \
             (username, calendar, uid, type, summary, priority, due_date, status, \
              completed_date, etag, ical_data, kind, topic, source) \
             VALUES (?, ?, ?, 'VTODO', ?, ?, ?, 'COMPLETED', ?, ?, ?, ?, ?, NULL)",
        )
        .bind(username)
        .bind(calendar)
        .bind(&uid)
        .bind(&summary)
        .bind(priority)
        .bind(&due)
        .bind(format!("{} 00:00:00", completed.format("%Y-%m-%d")))
        .bind(&etag)
        .bind(&ical)
        .bind(kind)
        .bind(slug)
        .execute(pool)
        .await?;
    }

    Ok(())
}

/// `"pay-the-electricity-bill"` → `"Pay The Electricity Bill"`.
fn title_case(slug: &str) -> String {
    slug.split('-')
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(Clone, Copy)]
struct DemoContact {
    name: &'static str,
    org: Option<&'static str>,
    email: Option<&'static str>,
    phone: Option<&'static str>,
}

/// Three named collections beyond the default "My Contacts" (seeded by
/// `ensure_default_contact_collection` below), each with its own color from
/// `ContactCollectionEditorModal::CONTACT_COLLECTION_PALETTE` (the 6 curated
/// collection colors; "My Contacts" itself already claims `#3b82f6`) so the
/// Contacts tab's collection switcher has several distinct collections —
/// not just the default — to switch between on a fresh instance.
const EXTRA_CONTACT_COLLECTIONS: [(&str, &str, &[DemoContact]); 3] = [
    (
        "Family",
        "#e35d7a",
        &[
            DemoContact {
                name: "Sarah Chen",
                org: None,
                email: Some("sarah.chen@example.com"),
                phone: Some("+1 555-0201"),
            },
            DemoContact {
                name: "David Chen",
                org: None,
                email: None,
                phone: Some("+1 555-0202"),
            },
            DemoContact {
                name: "Emily Rodriguez",
                org: None,
                email: Some("emily.rodriguez@example.com"),
                phone: None,
            },
            DemoContact {
                name: "Tom Okafor",
                org: None,
                email: Some("tom.okafor@example.com"),
                phone: Some("+1 555-0204"),
            },
        ],
    ),
    (
        "Work",
        "#a78bfa",
        &[
            DemoContact {
                name: "Priya Patel",
                org: Some("Northwind Logistics"),
                email: Some("priya.patel@northwind.example"),
                phone: Some("+1 555-0301"),
            },
            DemoContact {
                name: "Marcus Webb",
                org: Some("Northwind Logistics"),
                email: Some("marcus.webb@northwind.example"),
                phone: None,
            },
            DemoContact {
                name: "Lena Fischer",
                org: Some("Acme Corp"),
                email: None,
                phone: Some("+1 555-0303"),
            },
            DemoContact {
                name: "Jordan Kim",
                org: Some("Acme Corp"),
                email: Some("jordan.kim@acme.example"),
                phone: Some("+1 555-0304"),
            },
        ],
    ),
    (
        "Friends",
        "#06b6d4",
        &[
            DemoContact {
                name: "Noah Bennett",
                org: None,
                email: Some("noah.bennett@example.com"),
                phone: None,
            },
            DemoContact {
                name: "Ava Martinez",
                org: None,
                email: None,
                phone: Some("+1 555-0402"),
            },
            DemoContact {
                name: "Liam O'Connor",
                org: None,
                email: Some("liam.oconnor@example.com"),
                phone: Some("+1 555-0403"),
            },
            DemoContact {
                name: "Sophie Nguyen",
                org: None,
                email: Some("sophie.nguyen@example.com"),
                phone: Some("+1 555-0404"),
            },
        ],
    ),
];

async fn seed_contacts(
    pool: &SqlitePool,
    contacts: &Arc<dyn ContactStore + Send + Sync>,
    username: &str,
) -> Result<Vec<String>> {
    let demo = [
        DemoContact {
            name: "Ada Lovelace",
            org: Some("Analytical Engines"),
            email: Some("ada@example.com"),
            phone: Some("+1 555-0101"),
        },
        DemoContact {
            name: "Grace Hopper",
            org: Some("US Navy"),
            email: Some("grace@example.com"),
            phone: Some("+1 555-0102"),
        },
        DemoContact {
            name: "Alan Turing",
            org: Some("Bletchley Park"),
            email: Some("alan@example.com"),
            phone: None,
        },
        DemoContact {
            name: "Margaret Hamilton",
            org: Some("NASA"),
            email: None,
            phone: Some("+1 555-0104"),
        },
        DemoContact {
            name: "Katherine Johnson",
            org: None,
            email: Some("katherine@example.com"),
            phone: None,
        },
    ];

    let demo_collection_id = contacts.ensure_default_contact_collection(username).await?;
    let mut names = store_demo_contacts(contacts, username, demo_collection_id, &demo).await?;

    for (name, color, demo) in EXTRA_CONTACT_COLLECTIONS {
        let collection_id: i64 = sqlx::query(
            "INSERT INTO contact_collections (owner_username, name, color) VALUES (?, ?, ?) RETURNING id",
        )
        .bind(username)
        .bind(name)
        .bind(color)
        .fetch_one(pool)
        .await?
        .try_get("id")?;

        names.extend(store_demo_contacts(contacts, username, collection_id, demo).await?);
    }

    Ok(names)
}

async fn store_demo_contacts(
    contacts: &Arc<dyn ContactStore + Send + Sync>,
    username: &str,
    collection_id: i64,
    demo: &[DemoContact],
) -> Result<Vec<String>> {
    let mut names = Vec::new();
    for &DemoContact {
        name,
        org,
        email,
        phone,
    } in demo
    {
        let uid = format!("demo-contact-{}", uuid::Uuid::new_v4());
        let etag = format!("etag-{}", uuid::Uuid::new_v4().simple());
        let fields = VCardFields {
            uid: uid.clone(),
            display_name: name.to_string(),
            org: org.map(str::to_string),
            emails: email
                .map(|e| {
                    vec![VCardMethod {
                        value: e.to_string(),
                        kind: "work".into(),
                        is_primary: true,
                    }]
                })
                .unwrap_or_default(),
            phones: phone
                .map(|p| {
                    vec![VCardMethod {
                        value: p.to_string(),
                        kind: "mobile".into(),
                        is_primary: true,
                    }]
                })
                .unwrap_or_default(),
            ..Default::default()
        };
        contacts
            .store_contact_vcard(username, collection_id, &uid, &fields, &etag)
            .await?;
        contacts
            .bump_address_book_sync_token(collection_id, &uid, "created")
            .await?;
        names.push(name.to_string());
    }
    Ok(names)
}

/// One synthetic VEVENT. `hour`/`minute` pin a plausible time of day (rather
/// than inheriting "whatever time `gts` happens to run"), and `rrule` adds an
/// `RRULE:` line — the UI expands it at read time from `ical_data`, so no
/// extra bookkeeping is needed here (see `api/events.rs`).
struct DemoEvent {
    summary: &'static str,
    calendar: &'static str,
    day_offset: i64,
    hour: u32,
    minute: u32,
    minutes: i64,
    rrule: Option<&'static str>,
}

async fn seed_events(
    calendars: &Arc<dyn CalendarStore + Send + Sync>,
    username: &str,
) -> Result<()> {
    let now = Local::now();
    let today = now.date_naive();
    let dtstamp = now.format("%Y%m%dT%H%M%SZ").to_string();

    // Every event falls inside the next two weeks; each calendar gets both
    // one-off and recurring entries so the calendar views have something to
    // expand and scroll through immediately.
    let demo: [DemoEvent; 9] = [
        DemoEvent {
            summary: "Dentist appointment",
            calendar: "personal",
            day_offset: 3,
            hour: 10,
            minute: 0,
            minutes: 60,
            rrule: None,
        },
        DemoEvent {
            summary: "Morning run",
            calendar: "personal",
            day_offset: 1,
            hour: 7,
            minute: 0,
            minutes: 45,
            rrule: Some("FREQ=WEEKLY;BYDAY=MO,WE,FR"),
        },
        DemoEvent {
            summary: "Book club",
            calendar: "personal",
            day_offset: 9,
            hour: 19,
            minute: 0,
            minutes: 90,
            rrule: None,
        },
        DemoEvent {
            summary: "Team standup",
            calendar: "work",
            day_offset: 1,
            hour: 9,
            minute: 30,
            minutes: 30,
            rrule: Some("FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR"),
        },
        DemoEvent {
            summary: "Quarterly planning",
            calendar: "work",
            day_offset: 7,
            hour: 13,
            minute: 0,
            minutes: 120,
            rrule: None,
        },
        DemoEvent {
            summary: "1:1 with manager",
            calendar: "work",
            day_offset: 2,
            hour: 15,
            minute: 0,
            minutes: 30,
            rrule: Some("FREQ=WEEKLY"),
        },
        DemoEvent {
            summary: "Family dinner",
            calendar: "family",
            day_offset: 5,
            hour: 18,
            minute: 0,
            minutes: 90,
            rrule: Some("FREQ=WEEKLY"),
        },
        DemoEvent {
            summary: "School pick-up",
            calendar: "family",
            day_offset: 1,
            hour: 15,
            minute: 0,
            minutes: 30,
            rrule: Some("FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR"),
        },
        DemoEvent {
            summary: "Weekend hike",
            calendar: "family",
            day_offset: 6,
            hour: 9,
            minute: 0,
            minutes: 180,
            rrule: None,
        },
    ];
    for DemoEvent {
        summary,
        calendar,
        day_offset,
        hour,
        minute,
        minutes,
        rrule,
    } in demo
    {
        let start = today.and_hms_opt(hour, minute, 0).unwrap() + Duration::days(day_offset);
        let uid = format!("demo-event-{}", uuid::Uuid::new_v4());
        let dtstart = start.format("%Y%m%dT%H%M%S").to_string();
        let dtend = (start + Duration::minutes(minutes))
            .format("%Y%m%dT%H%M%S")
            .to_string();
        let rrule_line = rrule.map(|r| format!("\r\nRRULE:{r}")).unwrap_or_default();
        let ical = format!(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//tidl//tidl//EN\r\n\
             BEGIN:VEVENT\r\nUID:{uid}\r\nDTSTAMP:{dtstamp}\r\nDTSTART:{dtstart}\r\nDTEND:{dtend}\r\n\
             SUMMARY:{summary}{rrule_line}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
        );
        let etag = format!("etag-{}", uuid::Uuid::new_v4().simple());
        calendars
            .store_event(username, calendar, &uid, &ical, &etag)
            .await?;
    }
    Ok(())
}

/// A top→bottom RGB gradient, used only as the offline fallback for a recipe
/// photo that could not be pulled from the web.
type DemoGradient = ((u8, u8, u8), (u8, u8, u8));

/// One demo recipe note plus the real dish photo that accompanies it. The
/// image is downloaded at seed time so a fresh board shows actual food rather
/// than generated placeholders; `image_url` uses Commons' `Special:FilePath`
/// resolver, which redirects to a width-limited thumbnail of the named file.
struct DemoRecipe {
    title: &'static str,
    body: &'static str,
    image_url: &'static str,
    filename: &'static str,
}

const DEMO_RECIPES: [DemoRecipe; 4] = [
    DemoRecipe {
        title: "Weeknight spaghetti bolognese",
        body: "Soffritto low and slow, a splash of red wine, simmer for an hour.",
        image_url: "https://commons.wikimedia.org/wiki/Special:FilePath/Spaghetti_Bolognese.jpg?width=800",
        filename: "spaghetti-bolognese.jpg",
    },
    DemoRecipe {
        title: "Chicken tikka masala",
        body: "Marinate overnight, char under the grill, finish in a tomato-cream sauce.",
        image_url: "https://commons.wikimedia.org/wiki/Special:FilePath/Chicken_tikka_masala.jpg?width=800",
        filename: "chicken-tikka-masala.jpg",
    },
    DemoRecipe {
        title: "Margherita pizza",
        body: "48-hour cold ferment, San Marzano tomatoes, torn basil, a very hot oven.",
        image_url: "https://commons.wikimedia.org/wiki/Special:FilePath/Margherita_pizza.jpg?width=800",
        filename: "margherita-pizza.jpg",
    },
    DemoRecipe {
        title: "Brown-butter chocolate chip cookies",
        body: "Brown the butter, chill the dough overnight, finish with flaky salt.",
        image_url: "https://commons.wikimedia.org/wiki/Special:FilePath/Chocolate_chip_cookies.jpg?width=800",
        filename: "chocolate-chip-cookies.jpg",
    },
];

/// One demo collection and the column block its cards occupy on the board.
struct DemoCollection {
    name: &'static str,
    color: &'static str,
    /// Real grid columns this collection's island uses, in `0..CARD_GRID_COLS`.
    /// Kept non-adjacent to every other island's columns (see `seed_notes`).
    columns: &'static [i32],
}

async fn seed_notes(
    pool: &SqlitePool,
    calendars: &Arc<dyn CalendarStore + Send + Sync>,
    username: &str,
    contact_names: &[String],
) -> Result<()> {
    // Three collections, each laid out as its own little "island" so the
    // board's cohesion-aware reflow (`grid::detect_clusters`, backlog #102)
    // sees three separate gaggles rather than one undifferentiated pile.
    //
    // The layout assumes a 7-column board, split into three column blocks
    // with a blank column between them: [0,1] [3,4] [6]. The separating
    // columns are load-bearing: two collections in adjacent (or the same)
    // columns with close row-centres would merge into one gaggle, so each
    // island stays at least one empty column from its neighbours. The whole
    // 7-wide span sits inside `grid::CARD_GRID_COLS` (8) real columns, so
    // the positions render correctly and compact down to separate view
    // columns on a narrow viewport.
    let collections = [
        DemoCollection {
            name: "Reminders",
            color: "#8b5cf6",
            columns: &[0, 1],
        },
        DemoCollection {
            name: "Recipes",
            color: "#f59e0b",
            columns: &[3, 4],
        },
        DemoCollection {
            name: "Journal",
            color: "#22c55e",
            columns: &[6],
        },
    ];

    let mut collection_ids = Vec::new();
    for DemoCollection { name, color, .. } in &collections {
        let id: i64 = sqlx::query(
            "INSERT INTO note_collections (owner_username, name, color) VALUES (?, ?, ?) RETURNING id",
        )
        .bind(username)
        .bind(name)
        .bind(color)
        .fetch_one(pool)
        .await?
        .try_get("id")?;
        collection_ids.push(id);
    }

    let today = Local::now().date_naive();

    // Reminders — island #1. Dated tasks of varying priority (1 = highest,
    // 100 = lowest) plus a couple of await items, all mirrored to VTODOs
    // (see `insert_mirrored_note`) so the Tasks/Tickler/Focus views pick
    // them up. Both kinds share this island's heights so they pack together.
    let mut heights = island_heights(collections[0].columns);
    let tasks: [(&str, u32, Option<NaiveDate>); 4] = [
        (
            "Pay the electricity bill",
            5,
            Some(today + Duration::days(2)),
        ),
        ("Renew passport", 30, Some(today + Duration::days(14))),
        (
            "Book dentist appointment",
            70,
            Some(today + Duration::days(7)),
        ),
        ("Send birthday card to Mum", 95, None),
    ];
    for (title, priority, due) in tasks {
        let (grid_row, grid_col) = tidl_web::grid::place_in_shortest(&mut heights, 1);
        insert_mirrored_note(
            pool,
            calendars,
            username,
            collection_ids[0],
            grid_row,
            grid_col,
            "task",
            title,
            &format!("{title}."),
            Some(priority),
            due,
        )
        .await?;
    }

    let awaits: [(&str, NaiveDate); 3] = [
        (
            "Waiting on plumber to confirm the quote",
            today - Duration::days(3),
        ),
        ("Waiting on tax refund", today + Duration::days(10)),
        (
            "Waiting on landlord to confirm move-out date",
            today + Duration::days(30),
        ),
    ];
    for (title, due) in awaits {
        let (grid_row, grid_col) = tidl_web::grid::place_in_shortest(&mut heights, 1);
        insert_mirrored_note(
            pool,
            calendars,
            username,
            collection_ids[0],
            grid_row,
            grid_col,
            "await",
            title,
            &format!("{title}."),
            None,
            Some(due),
        )
        .await?;
    }

    // Recipes — island #2. Plain notes (kind stays NULL, so no VTODO mirror),
    // each carrying a real dish photo downloaded from Wikimedia Commons. The
    // photo lands in `note_images` and the body references it with backlog
    // #110's `@img:<id>` token. A failed fetch (offline, rate-limited) falls
    // back to a generated gradient so the card renderer's inline-image path is
    // always exercised.
    let mut heights = island_heights(collections[1].columns);
    for recipe in DEMO_RECIPES {
        let (grid_row, grid_col) = tidl_web::grid::place_in_shortest(&mut heights, 1);
        let note_id = insert_plain_note(
            pool,
            collection_ids[1],
            username,
            Some(recipe.title),
            recipe.body,
            grid_row,
            grid_col,
        )
        .await?;

        let (bytes, content_type) = match fetch_web_image(recipe.image_url).await {
            Ok((bytes, content_type)) => (bytes, content_type),
            Err(err) => {
                tracing::warn!(
                    url = recipe.image_url,
                    %err,
                    "demo recipe image fetch failed; using gradient fallback"
                );
                let (top, bottom): DemoGradient = ((58, 110, 165), (135, 168, 200));
                (demo_png(480, 320, top, bottom), "image/png".to_string())
            }
        };
        attach_image(pool, note_id, &bytes, &content_type, recipe.filename).await?;
    }

    // Journal — island #3. Plain notes @mention-ing a contact, kind NULL.
    let mut heights = island_heights(collections[2].columns);
    let journal: [String; 3] = [
        format!(
            "Caught up with @{} about the roadmap today — good energy.",
            contact_names.first().map(String::as_str).unwrap_or("")
        ),
        format!(
            "@{} suggested we revisit the onboarding doc next sprint.",
            contact_names.get(1).map(String::as_str).unwrap_or("")
        ),
        format!(
            "Follow-up from lunch with @{}: send over the deck by Friday.",
            contact_names.get(2).map(String::as_str).unwrap_or("")
        ),
    ];
    for text in journal {
        let (grid_row, grid_col) = tidl_web::grid::place_in_shortest(&mut heights, 1);
        insert_plain_note(
            pool,
            collection_ids[2],
            username,
            None,
            &text,
            grid_row,
            grid_col,
        )
        .await?;
    }

    Ok(())
}

/// Insert one plain (kind NULL) note — the shape the Journal and Recipes
/// islands use. Returns the new note id so a caller can attach an image.
async fn insert_plain_note(
    pool: &SqlitePool,
    collection_id: i64,
    username: &str,
    title: Option<&str>,
    text: &str,
    grid_row: i32,
    grid_col: i32,
) -> Result<i64> {
    let note_id: i64 = sqlx::query(
        "INSERT INTO notes (collection_id, author_username, title, text, pinned, grid_row, grid_col, grid_row_span)
         VALUES (?, ?, ?, ?, 0, ?, ?, 1) RETURNING id",
    )
    .bind(collection_id)
    .bind(username)
    .bind(title)
    .bind(text)
    .bind(grid_row)
    .bind(grid_col)
    .fetch_one(pool)
    .await?
    .try_get("id")?;
    Ok(note_id)
}

/// Store `data` as the first `note_images` row for `note_id` and append the
/// `@img:<id>` reference the card renderer resolves to a body line.
async fn attach_image(
    pool: &SqlitePool,
    note_id: i64,
    data: &[u8],
    content_type: &str,
    filename: &str,
) -> Result<()> {
    let image_id: i64 = sqlx::query(
        "INSERT INTO note_images (note_id, data, content_type, filename, position)
         VALUES (?, ?, ?, ?, 0) RETURNING id",
    )
    .bind(note_id)
    .bind(data)
    .bind(content_type)
    .bind(filename)
    .fetch_one(pool)
    .await?
    .try_get("id")?;
    sqlx::query("UPDATE notes SET text = text || ? WHERE id = ?")
        .bind(format!("\n\n@img:{image_id}"))
        .bind(note_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Download one demo photo from the web. Returns the bytes plus the server's
/// `Content-Type` (so the served image keeps the right MIME type).
async fn fetch_web_image(url: &str) -> Result<(Vec<u8>, String)> {
    let client = reqwest::Client::builder()
        // Wikimedia rejects requests without a descriptive User-Agent
        // (otherwise intermittent 429s), and a timeout keeps a slow network
        // from hanging the dev seeder.
        .user_agent("tidl-demo-seeder/1.0 (dev tooling; +https://example.invalid)")
        .timeout(std::time::Duration::from_secs(20))
        .build()?;
    let response = client.get(url).send().await?.error_for_status()?;
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("image/jpeg")
        .to_string();
    let bytes = response.bytes().await?.to_vec();
    Ok((bytes, content_type))
}

/// A `place_in_shortest` heights vector that only ever selects an island's
/// own `columns`: every other real column is parked at `i32::MAX`, so the
/// masonry pack can never spill a collection's card onto a neighbouring
/// island (or across one of the blank separating columns).
fn island_heights(columns: &[i32]) -> Vec<i32> {
    let mut heights = vec![i32::MAX; tidl_web::grid::CARD_GRID_COLS as usize];
    for &col in columns {
        heights[col as usize] = 0;
    }
    heights
}

/// Insert one TASK/AWAIT note plus its mirrored VTODO (see this module's
/// doc comment) — the same two-write shape `sync_mirror` + `create_note`
/// perform together in one transaction, reproduced here without the
/// transaction (a seeding failure partway through just leaves a few extra
/// rows in a throwaway database, not worth the ceremony).
#[allow(clippy::too_many_arguments)]
async fn insert_mirrored_note(
    pool: &SqlitePool,
    calendars: &Arc<dyn CalendarStore + Send + Sync>,
    username: &str,
    collection_id: i64,
    grid_row: i32,
    grid_col: i32,
    kind: &str,
    title: &str,
    body: &str,
    priority: Option<u32>,
    due: Option<NaiveDate>,
) -> Result<()> {
    let uid = format!("note:{}", uuid::Uuid::new_v4());
    let etag = format!("etag-{}", uuid::Uuid::new_v4().simple());
    let due_date_str = due.map(|d| d.format("%Y-%m-%d").to_string());

    let priority_line = if kind == "task" {
        format!(
            "\r\nPRIORITY:{}",
            tidl_web::types::source_priority_to_caldav(priority.unwrap_or(50))
        )
    } else {
        String::new()
    };
    let due_line = due_date_str
        .as_deref()
        .map(|d| format!("\r\nDUE;VALUE=DATE:{}", d.replace('-', "")))
        .unwrap_or_default();
    let description_line = {
        use tidl_caldav::icalendar_utils::{escape_ical_text, fold_ical_line};
        format!(
            "\r\n{}",
            fold_ical_line(&format!("DESCRIPTION:{}", escape_ical_text(body)))
        )
    };
    let ical_data = format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//tidl//tidl//EN\r\n\
         BEGIN:VTODO\r\nUID:{uid}\r\nSUMMARY:{title}{priority_line}\r\n\
         STATUS:NEEDS-ACTION{due_line}{description_line}\r\n\
         END:VTODO\r\nEND:VCALENDAR\r\n"
    );

    let db_priority = if kind == "task" {
        priority.map(i64::from)
    } else {
        None
    };

    // `events.due_date` is a DATETIME column decoded straight into
    // `chrono::DateTime<Utc>` by `storage::list_events`/`list_todos` (CalDAV
    // listing, calendar `.ics` export) — a bare `YYYY-MM-DD` fails that
    // decode. Mirrors the same fix in `tidl_web::api::notes::sync_mirror`,
    // whose real INSERT this reproduces (see this module's doc comment).
    let events_due_date = due_date_str.as_deref().map(|d| format!("{d} 00:00:00"));

    sqlx::query(
        "INSERT INTO events
         (username, calendar, uid, type, summary, priority, status, due_date, source, etag, ical_data, kind)
         VALUES (?, 'personal', ?, 'VTODO', ?, ?, 'NEEDS-ACTION', ?, 'note', ?, ?, ?)",
    )
    .bind(username)
    .bind(&uid)
    .bind(title)
    .bind(db_priority)
    .bind(&events_due_date)
    .bind(&etag)
    .bind(&ical_data)
    .bind(kind)
    .execute(pool)
    .await?;

    calendars
        .bump_sync_token(username, "personal", &uid, "modified")
        .await?;

    sqlx::query(
        "INSERT INTO notes
         (collection_id, author_username, title, text, pinned, kind, priority, calendar_name, due_date, mirrored_event_uid, grid_row, grid_col, grid_row_span)
         VALUES (?, ?, ?, ?, 0, ?, ?, 'personal', ?, ?, ?, ?, 1)",
    )
    .bind(collection_id)
    .bind(username)
    .bind(title)
    .bind(body)
    .bind(kind)
    .bind(db_priority)
    .bind(&due_date_str)
    .bind(&uid)
    .bind(grid_row)
    .bind(grid_col)
    .execute(pool)
    .await?;

    Ok(())
}

/// Build a real (uncompressed, truecolor) PNG of `width`×`height`, filled
/// with a vertical gradient from `top` to `bottom`. Hand-rolled so the dev
/// seeder needs no image-encoding dependency: a PNG is just signature +
/// IHDR/IDAT/IEND chunks, and the zlib stream can use stored (uncompressed)
/// deflate blocks — the browser decodes it as an ordinary PNG.
fn demo_png(width: u32, height: u32, top: (u8, u8, u8), bottom: (u8, u8, u8)) -> Vec<u8> {
    let mut raw = Vec::with_capacity(height as usize * (1 + width as usize * 3));
    for y in 0..height {
        raw.push(0); // per-scanline filter type: None
        let t = if height <= 1 {
            0.0
        } else {
            y as f32 / (height - 1) as f32
        };
        let px = (
            lerp_u8(top.0, bottom.0, t),
            lerp_u8(top.1, bottom.1, t),
            lerp_u8(top.2, bottom.2, t),
        );
        for _ in 0..width {
            raw.extend_from_slice(&[px.0, px.1, px.2]);
        }
    }

    let mut out = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]); // 8-bit, truecolor RGB
    write_png_chunk(&mut out, b"IHDR", &ihdr);
    write_png_chunk(&mut out, b"IDAT", &zlib_stored(&raw));
    write_png_chunk(&mut out, b"IEND", &[]);
    out
}

fn lerp_u8(a: u8, b: u8, t: f32) -> u8 {
    (a as f32 + (b as f32 - a as f32) * t)
        .round()
        .clamp(0.0, 255.0) as u8
}

fn write_png_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc = Crc32::new();
    crc.update(kind);
    crc.update(data);
    out.extend_from_slice(&crc.finish().to_be_bytes());
}

/// Wrap `data` in a zlib stream using only stored (uncompressed) deflate
/// blocks — valid zlib, just not compressed.
fn zlib_stored(data: &[u8]) -> Vec<u8> {
    // CMF/FLG: deflate, 32K window; the FLG check bits are set so the header
    // is a multiple of 31 (the 0x78 0x01 pair is the canonical empty case).
    let mut out = vec![0x78, 0x01];
    let mut chunks = data.chunks(0xFFFF).peekable();
    if data.is_empty() {
        out.extend_from_slice(&[0x01, 0x00, 0x00, 0xFF, 0xFF]);
    }
    while let Some(chunk) = chunks.next() {
        let final_block = chunks.peek().is_none();
        out.push(if final_block { 0x01 } else { 0x00 });
        let len = chunk.len() as u16;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(chunk);
    }
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b): (u32, u32) = (1, 0);
    for &byte in data {
        a = (a + byte as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

/// Incremental CRC-32 (IEEE, the polynomial PNG uses) — table-free bit loop,
/// plenty fast for a handful of tiny dev-only images.
struct Crc32(u32);

impl Crc32 {
    fn new() -> Self {
        Crc32(0xFFFF_FFFF)
    }

    fn update(&mut self, data: &[u8]) {
        for &byte in data {
            self.0 ^= byte as u32;
            for _ in 0..8 {
                let mask = (self.0 & 1).wrapping_neg();
                self.0 = (self.0 >> 1) ^ (0xEDB8_8320 & mask);
            }
        }
    }

    fn finish(self) -> u32 {
        self.0 ^ 0xFFFF_FFFF
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Storage;
    use sqlx::sqlite::SqliteConnectOptions;
    use std::str::FromStr;

    // Mirrors `storage::tests::test_pool`/`test_storage` — kept local rather
    // than shared since those are private to that module's own test scope.
    async fn test_pool() -> SqlitePool {
        let opts = SqliteConnectOptions::from_str("sqlite::memory:").unwrap();
        let pool = SqlitePool::connect_with(opts).await.unwrap();
        sqlx::migrate!("../../migrations").run(&pool).await.unwrap();
        pool
    }

    fn test_storage(pool: SqlitePool) -> Storage {
        Storage {
            pool,
            data_dir: std::env::temp_dir().to_str().unwrap().to_string(),
        }
    }

    #[tokio::test]
    async fn seed_populates_contacts_events_and_notes() {
        let pool = test_pool().await;
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin, is_active)
             VALUES ('demo', 'demo@example.com', 'Demo', 'x', 1, 1)",
        )
        .execute(&pool)
        .await
        .unwrap();

        let storage = test_storage(pool.clone());
        let calendars: Arc<dyn CalendarStore + Send + Sync> = Arc::new(storage.clone());
        let contacts: Arc<dyn ContactStore + Send + Sync> = Arc::new(storage);

        seed(&pool, &calendars, &contacts, "demo").await.unwrap();

        let contact_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM contacts WHERE owner_username = 'demo'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(contact_count, 17); // 5 in "My Contacts" + 4 each in Family/Work/Friends

        // "My Contacts" plus the three extra collections, each with its own
        // color so the Contacts tab's collection switcher can tell them apart.
        let contact_collection_colors: Vec<String> = sqlx::query_scalar(
            "SELECT color FROM contact_collections WHERE owner_username = 'demo' ORDER BY name",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(contact_collection_colors.len(), 4);
        let distinct_contact_collection_colors: std::collections::HashSet<&String> =
            contact_collection_colors.iter().collect();
        assert_eq!(
            distinct_contact_collection_colors.len(),
            4,
            "demo contact collections must have distinct colors"
        );

        // Demo contacts go through the same `store_contact_vcard` path as a
        // real CardDAV PUT (backlog #81) — must never end up collection-less,
        // or the demo Contacts tab would render as silently empty.
        let uncollected: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM contacts WHERE owner_username = 'demo' AND collection_id IS NULL",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            uncollected, 0,
            "every demo contact must have a collection_id"
        );

        // Three collections, each laid out as its own island of cards.
        let collection_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM note_collections WHERE owner_username = 'demo'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(collection_count, 3);

        // Every collection's cards occupy their own column block, and no two
        // blocks are adjacent -- that separation is what keeps the
        // cohesion-aware reflow (`grid::detect_clusters`) from merging the
        // three islands into one gaggle.
        let mut islands: Vec<(i32, i32)> = sqlx::query(
            "SELECT MIN(n.grid_col), MAX(n.grid_col)
             FROM note_collections c JOIN notes n ON n.collection_id = c.id
             WHERE c.owner_username = 'demo'
             GROUP BY c.id",
        )
        .fetch_all(&pool)
        .await
        .unwrap()
        .into_iter()
        .map(|r| (r.get::<i32, _>(0), r.get::<i32, _>(1)))
        .collect();
        islands.sort_unstable();
        assert_eq!(islands, vec![(0, 1), (3, 4), (6, 6)]);
        for pair in islands.windows(2) {
            assert!(
                pair[1].0 - pair[0].1 >= 2,
                "demo islands must be non-adjacent, got {islands:?}"
            );
        }

        // 9 plain VEVENTs + 4 task VTODOs + 3 await VTODOs + 50 seeded
        // completed items for the Done tab.
        let event_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM events WHERE username = 'demo'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(event_count, 66);

        // The 50 Done-tab items are all completed VTODOs.
        let done_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM events WHERE username = 'demo' \
             AND type = 'VTODO' AND status = 'COMPLETED'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(done_count, 50);

        // The three demo calendars each carry their own color, all distinct.
        let colors: Vec<String> =
            sqlx::query_scalar("SELECT color FROM calendars WHERE username = 'demo' ORDER BY name")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(colors.len(), 3);
        let distinct_colors: std::collections::HashSet<&String> = colors.iter().collect();
        assert_eq!(
            distinct_colors.len(),
            3,
            "demo calendars must have distinct colors"
        );

        // ...and those colors must differ from the note collections' vivid
        // hexes, so the calendar and notes color systems stay visually
        // distinguishable on the demo board.
        let collection_colors: Vec<String> =
            sqlx::query_scalar("SELECT color FROM note_collections WHERE owner_username = 'demo'")
                .fetch_all(&pool)
                .await
                .unwrap();
        for color in &colors {
            assert!(
                !collection_colors.contains(color),
                "calendar color {color} must differ from every note-collection color"
            );
        }

        // Every calendar has at least one event inside the next two weeks.
        let calendars_with_near_events: i64 = sqlx::query_scalar(
            "SELECT COUNT(DISTINCT calendar) FROM events
             WHERE username = 'demo' AND type = 'VEVENT'
               AND date(start_date) >= date('now')
               AND date(start_date) <= date('now', '+14 days')",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(calendars_with_near_events, 3);

        // Some of those events recur (the UI expands their RRULE at read time).
        let recurring_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM events
             WHERE username = 'demo' AND type = 'VEVENT' AND ical_data LIKE '%RRULE:%'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(recurring_count, 5);

        // Each recipe note carries a dish photo, referenced from its body
        // with an `@img:<id>` token. (When the network is unavailable the
        // seeder falls back to a generated gradient, so the count holds
        // either way.)
        let image_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM note_images")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(image_count, 4);
        let notes_with_images: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM notes n JOIN note_collections c ON n.collection_id = c.id
             WHERE c.owner_username = 'demo' AND n.text LIKE '%@img:%'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(notes_with_images, 4);

        // The demo photos keep the recipe filenames (and a web download or the
        // offline fallback content type).
        let image_files: Vec<String> =
            sqlx::query_scalar("SELECT filename FROM note_images ORDER BY filename")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(
            image_files,
            vec![
                "chicken-tikka-masala.jpg",
                "chocolate-chip-cookies.jpg",
                "margherita-pizza.jpg",
                "spaghetti-bolognese.jpg",
            ]
        );
        let image_types: Vec<String> =
            sqlx::query_scalar("SELECT DISTINCT content_type FROM note_images")
                .fetch_all(&pool)
                .await
                .unwrap();
        for content_type in &image_types {
            assert!(
                content_type == "image/jpeg" || content_type == "image/png",
                "unexpected demo image content type {content_type}"
            );
        }

        let task_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM notes n JOIN note_collections c ON n.collection_id = c.id
             WHERE c.owner_username = 'demo' AND n.kind = 'task'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(task_count, 4);

        let await_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM notes n JOIN note_collections c ON n.collection_id = c.id
             WHERE c.owner_username = 'demo' AND n.kind = 'await'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(await_count, 3);

        // Journal @mentions a contact; the `@img:` token is not a mention, so
        // exclude it from the count.
        let mention_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM notes n JOIN note_collections c ON n.collection_id = c.id
             WHERE c.owner_username = 'demo' AND n.kind IS NULL
               AND n.text LIKE '%@%' AND n.text NOT LIKE '%@img:%'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(mention_count, 3);

        // Every task/await note must have a mirrored VTODO for the Tasks/
        // Tickler/Focus views to pick it up.
        let unmirrored: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM notes n JOIN note_collections c ON n.collection_id = c.id
             WHERE c.owner_username = 'demo' AND n.kind IS NOT NULL AND n.mirrored_event_uid IS NULL",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(unmirrored, 0);

        // Priorities actually vary (not all defaulted to the same value).
        let distinct_priorities: i64 = sqlx::query_scalar(
            "SELECT COUNT(DISTINCT priority) FROM notes n JOIN note_collections c ON n.collection_id = c.id
             WHERE c.owner_username = 'demo' AND n.kind = 'task'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(distinct_priorities, 4);

        // Due dates actually vary across the await items.
        let distinct_due_dates: i64 = sqlx::query_scalar(
            "SELECT COUNT(DISTINCT due_date) FROM notes n JOIN note_collections c ON n.collection_id = c.id
             WHERE c.owner_username = 'demo' AND n.kind = 'await'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(distinct_due_dates, 3);

        // Regression check: `list_events`/`list_todos` decode `due_date`
        // straight into `chrono::DateTime<Utc>` — this must not fail for
        // the task/await notes' mirrored VTODOs (see this module's
        // `events_due_date` doc comment).
        calendars.list_events("demo", "personal").await.unwrap();
    }

    #[test]
    fn demo_png_is_a_valid_png_with_expected_pixels() {
        let bytes = demo_png(4, 2, (0, 0, 0), (255, 255, 255));
        assert_eq!(
            &bytes[..8],
            &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]
        );

        // Walk the chunks, validating each one's CRC.
        let mut pos = 8;
        let mut idat = Vec::new();
        let mut saw_ihdr = false;
        let mut saw_iend = false;
        while pos < bytes.len() {
            let len = u32::from_be_bytes(bytes[pos..pos + 4].try_into().unwrap()) as usize;
            let kind: [u8; 4] = bytes[pos + 4..pos + 8].try_into().unwrap();
            let data = &bytes[pos + 8..pos + 8 + len];
            let crc = u32::from_be_bytes(bytes[pos + 8 + len..pos + 12 + len].try_into().unwrap());
            let mut c = Crc32::new();
            c.update(&kind);
            c.update(data);
            assert_eq!(
                c.finish(),
                crc,
                "bad PNG chunk CRC for {}",
                String::from_utf8_lossy(&kind)
            );
            match &kind {
                b"IHDR" => {
                    saw_ihdr = true;
                    assert_eq!(&data[0..4], &4u32.to_be_bytes());
                    assert_eq!(&data[4..8], &2u32.to_be_bytes());
                    assert_eq!(&data[8..13], &[8, 2, 0, 0, 0]);
                }
                b"IDAT" => idat.extend_from_slice(data),
                b"IEND" => saw_iend = true,
                _ => {}
            }
            pos += 12 + len;
        }
        assert!(saw_ihdr && saw_iend);
        assert_eq!(pos, bytes.len());

        // Decode the zlib stream: header, one stored final block, Adler-32.
        assert_eq!(&idat[..2], &[0x78, 0x01]);
        let mut z = 2;
        assert_eq!(idat[z] & 1, 1, "block must be marked final");
        assert_eq!((idat[z] >> 1) & 0b11, 0, "only stored deflate blocks");
        z += 1;
        let len = u16::from_le_bytes([idat[z], idat[z + 1]]) as usize;
        let nlen = u16::from_le_bytes([idat[z + 2], idat[z + 3]]);
        assert_eq!(!nlen, len as u16);
        z += 4;
        let raw = &idat[z..z + len];
        z += len;
        assert_eq!(raw.len(), 2 * (1 + 4 * 3));
        // Scanline 0 filters with 0 and is all black; scanline 1 all white.
        assert_eq!(raw[0], 0);
        assert_eq!(&raw[1..4], &[0, 0, 0]);
        let last_row = 1 + 4 * 3;
        assert_eq!(raw[last_row], 0);
        assert_eq!(&raw[last_row + 1..last_row + 4], &[255, 255, 255]);
        // The trailing Adler-32 covers the raw scanlines.
        assert_eq!(
            u32::from_be_bytes(idat[z..z + 4].try_into().unwrap()),
            adler32(raw)
        );
        assert_eq!(z + 4, idat.len());
    }
}
