//! Root application component

use leptos::prelude::*;
use leptos_meta::*;
use leptos_router::{components::*, path};

use crate::api::{
    do_login, do_logout, do_request_password_reset, do_reset_password, do_signup, do_verify_email,
    get_oidc_config, get_preferences, get_priority_curves, get_registration_enabled,
    get_session_info, list_suppressed_focus_items, Event,
};
use crate::components::filter_context::{
    HiddenMdSourcesCtx, HiddenNoteCollectionsCtx, HiddenNoteLabelsCtx,
};
use crate::components::logo::Logo;
use crate::components::markdown_file_finder::MarkdownFileFinder;
use crate::components::profile_modal::ProfileModal;
use crate::components::settings::{IsAdminCtx, OpenSettingsCtx, SettingsModal, UserPrefsCtx};
use crate::components::{
    calendar::CalendarView,
    contacts::ContactsView,
    done::DoneView,
    focus::{FocusView, PreviewOverlay, PreviewState, SuppressedFocusItemsCtx},
    notes::NotesView,
};
#[cfg(feature = "hydrate")]
use crate::local_editor::{host_is_loopback, load_prefs};
use crate::local_editor::{LocalEditorCtx, LocalEditorPrefs};
use crate::types::CalendarEntry;
use crate::types::{PriorityCurves, UserPrefs};
use crate::util::format_server_fn_error;

/// A trigger signal shared via context so child components can force a session
/// re-check (after login or logout) without a full page reload.
#[derive(Clone, Copy)]
struct SessionTrigger(RwSignal<u32>);

/// Incremented when sidebar-listed metadata changes: calendars (name, colour,
/// sync URL…), markdown/git sources, or note collections. Components that
/// show any of those lists subscribe to this signal.
#[derive(Clone, Copy)]
pub struct SidebarRefresh(pub RwSignal<u32>);

/// Incremented when calendar events or todos change.
/// Components that display events subscribe to this signal.
#[derive(Clone, Copy)]
pub struct EventsRefresh(pub RwSignal<u32>);

/// Incremented when contacts change from outside `ContactsView` itself —
/// currently just Settings' `.vcf` import (#87) — so an already-open
/// `ContactsView` refetches without a manual reload. Not wired into the SSE
/// live-push stream below; contacts aren't part of that system yet (out of
/// scope for #87).
#[derive(Clone, Copy)]
pub struct ContactsRefresh(pub RwSignal<u32>);

/// Set by Settings' `.vcf` import (#87) whenever it creates at least one new
/// contact, so `ContactsView` runs one auto-layout pass with its own
/// correctly-measured `view_cols` next time it's mounted (or immediately, if
/// already mounted underneath the Settings modal) — the import itself places
/// new contacts across the full `grid::CARD_GRID_COLS` real-column space
/// (see `import_vcf`'s doc comment), which renders unevenly once compacted
/// down to however many columns actually fit the viewport
/// (`grid::pack_for_view`'s doc comment). `ContactsView` clears this once
/// it's acted on it. Newtype, not a bare `RwSignal<bool>` alias — see
/// `CurrentUsernameCtx`'s doc comment above for why.
#[derive(Clone, Copy)]
pub struct ContactsNeedsRelayoutCtx(pub RwSignal<bool>);

/// Incremented when notes change from outside `NotesView` itself — currently
/// just Settings' Google Keep import (#109) — so an already-open `NotesView`
/// refetches without a manual reload. Newtype, not a bare `RwSignal<u32>`
/// alias, for the `TypeId`-collision reason [`CurrentUsernameCtx`] documents.
#[derive(Clone, Copy)]
pub struct NotesRefresh(pub RwSignal<u32>);

/// Access the sidebar-refresh signal from context.
///
/// Increments drive refetches of calendar/source/collection metadata
/// throughout the component tree. Returns a detached signal when called
/// outside the context (e.g. in tests).
pub fn use_sidebar_refresh() -> RwSignal<u32> {
    use_context::<SidebarRefresh>()
        .map(|r| r.0)
        .unwrap_or_else(|| RwSignal::new(0u32))
}

/// Access the events-refresh signal from context.
///
/// Increments drive refetches of calendar events and todos throughout the component tree.
/// Returns a detached signal when called outside the context (e.g. in tests).
pub fn use_events_refresh() -> RwSignal<u32> {
    use_context::<EventsRefresh>()
        .map(|r| r.0)
        .unwrap_or_else(|| RwSignal::new(0u32))
}

/// Access the contacts-refresh signal from context.
///
/// Increments drive a refetch in `ContactsView` when contacts change from
/// outside it. Returns a detached signal when called outside the context
/// (e.g. in tests).
pub fn use_contacts_refresh() -> RwSignal<u32> {
    use_context::<ContactsRefresh>()
        .map(|r| r.0)
        .unwrap_or_else(|| RwSignal::new(0u32))
}

/// Access the contacts-needs-relayout signal from context.
///
/// Set by Settings' `.vcf` import when it creates new contacts; consumed by
/// `ContactsView` to run one auto-layout pass. Returns a detached signal
/// when called outside the context (e.g. in tests).
pub fn use_contacts_needs_relayout() -> RwSignal<bool> {
    use_context::<ContactsNeedsRelayoutCtx>()
        .map(|r| r.0)
        .unwrap_or_else(|| RwSignal::new(false))
}

/// Access the notes-refresh signal from context.
///
/// Increments drive a refetch in `NotesView` when notes change from outside
/// it (Settings' Keep import, #109). Returns a detached signal when called
/// outside the context (e.g. in tests).
pub fn use_notes_refresh() -> RwSignal<u32> {
    use_context::<NotesRefresh>()
        .map(|r| r.0)
        .unwrap_or_else(|| RwSignal::new(0u32))
}

/// Incremented when a done item's live/archived state changes from within the
/// app — a task/await completed or re-opened, or a Done-view / Settings
/// clean-up purge — so open Done views refetch.
///
/// Deliberately separate from the SSE-driven `EventsRefresh`: refreshing the
/// list runs a full done-index reconcile, which is too expensive to trigger on
/// every sync broadcast. External completions therefore don't refresh until the
/// next in-app change (that's backlog #136's territory). Newtype, not a bare
/// `RwSignal<u32>` alias — see [`CurrentUsernameCtx`] for why.
#[derive(Clone, Copy)]
pub struct DoneRefresh(pub RwSignal<u32>);

/// Access the done-refresh signal from context. Returns a detached signal when
/// called outside the context (e.g. in tests), matching the other `use_*` helpers.
pub fn use_done_refresh() -> RwSignal<u32> {
    use_context::<DoneRefresh>()
        .map(|r| r.0)
        .unwrap_or_else(|| RwSignal::new(0u32))
}

/// Unified "calendars I can see" list — the current user's own calendars plus
/// any shared with them, as `(CalendarEntry, visible)` pairs. Persisted in
/// AuthenticatedApp (not per-page) so all four pages (Calendar, Focus, Todos,
/// Tickler) share one fetch, one visibility model, and one `CalendarListPanel`
/// component instead of each maintaining its own.
pub type AllCalendarsCtx = RwSignal<Vec<(CalendarEntry, bool)>>;

/// The authenticated user's own username — needed client-side to key owned
/// calendars in `AllCalendarsCtx` as `(my_username, calendar_name)`, since
/// `CalendarInfo` (from `get_calendars()`) doesn't carry the owner's username
/// itself (it's implicitly "you").
///
/// Newtype, not a bare `RwSignal<String>` alias — see `LastUsedCalendarCtx`'s
/// doc comment in `calendar/event_form.rs` for why a bare alias here silently
/// broke event-visibility filtering for every user.
#[derive(Clone, Copy)]
pub struct CurrentUsernameCtx(pub RwSignal<String>);

/// Cached events — persisted in AuthenticatedApp so CalendarView shows stale data
/// while the fresh fetch is in flight (Transition-like behaviour).
pub type CachedEventsCtx = RwSignal<Vec<Event>>;

/// Gate signal that becomes true once both cals_res and events_res have resolved
/// for the first time.  Persisted in AuthenticatedApp so subsequent CalendarView
/// mounts skip the loading state and show the previous data immediately.
///
/// Newtype, not a bare `RwSignal<bool>` alias — see `CurrentUsernameCtx`'s doc
/// comment above. This one collided with `IsAdminCtx` (also a bare
/// `RwSignal<bool>` alias, provided later in the same `AuthenticatedApp`
/// scope), so `use_context::<CalendarLoadedCtx>()` anywhere actually read the
/// admin flag instead of the loading gate.
#[derive(Clone, Copy)]
pub struct CalendarLoadedCtx(pub RwSignal<bool>);

/// Context type: the current user's three configurable priority curves
/// (backlog #68) — calendar time-left, due-date time-left, source
/// priority. Newtype, not a bare `RwSignal<PriorityCurves>` alias — see
/// `CurrentUsernameCtx`'s doc comment above for why.
#[derive(Clone, Copy)]
pub struct PriorityCurvesCtx(pub RwSignal<PriorityCurves>);

/// Opens the markdown file finder modal (backlog #75) — set from the filter
/// sidebar's Markdown section (next to its "+" add-source button, so the
/// entry point's scope reads as markdown-source-only rather than app-wide)
/// as well as toggled by the app-wide Ctrl/Cmd+K shortcut. Newtype, not a
/// bare `RwSignal<bool>` alias — see `CurrentUsernameCtx`'s doc comment
/// above for why.
#[derive(Clone, Copy)]
pub(crate) struct ShowMarkdownFinderCtx(pub RwSignal<bool>);

/// Shared open-state for the task/await edit modal (backlog #121), so the Focus
/// preview, note cards, and the note editor's chip row all open the same
/// dialog. Newtype, not a bare `RwSignal` alias — see `CurrentUsernameCtx`'s
/// doc comment above for why.
#[derive(Clone, Copy)]
pub(crate) struct TodoModalCtx(pub RwSignal<Option<crate::components::focus::TodoModalState>>);

/// The shared task/await edit-modal signal. Returns a detached signal when the
/// context is absent (e.g. an isolated component test) so callers never panic.
pub(crate) fn use_todo_modal() -> RwSignal<Option<crate::components::focus::TodoModalState>> {
    use_context::<TodoModalCtx>()
        .map(|c| c.0)
        .unwrap_or_else(|| RwSignal::new(None))
}

/// Open-state for the fullscreen inline-image lightbox (backlog #140): the
/// image URL to show, or `None`. Set from a note card's click delegation;
/// rendered once at the app root. Newtype, not a bare `RwSignal<Option<String>>`
/// alias — see `CurrentUsernameCtx`'s doc comment above.
#[derive(Clone, Copy)]
pub(crate) struct ImageLightboxCtx(pub RwSignal<Option<String>>);

/// The shared image-lightbox signal. Returns a detached signal when the
/// context is absent (e.g. an isolated component test) so callers never panic.
/// Only read client-side (the note card's click delegation), so it is unused
/// in the `ssr` build.
#[allow(dead_code)]
pub(crate) fn use_image_lightbox() -> RwSignal<Option<String>> {
    use_context::<ImageLightboxCtx>()
        .map(|c| c.0)
        .unwrap_or_else(|| RwSignal::new(None))
}

#[component]
pub fn App() -> impl IntoView {
    provide_meta_context();

    // Incrementing this signal causes `session` to refetch.
    let trigger = SessionTrigger(RwSignal::new(0u32));
    provide_context(trigger);

    // LocalResource always fetches client-side; Resource::new is SSR-aware and
    // waits for dehydrated data that never arrives in a pure-CSR setup.
    let session = LocalResource::new(move || {
        let _ = trigger.0.get(); // reactive: re-check on login/logout
        get_session_info()
    });

    view! {
        <Stylesheet id="leptos" href="/pkg/tidl.css"/>
        <Link rel="stylesheet" href="/assets/style.css"/>
        <Title text="tidl - CalDAV & Task Manager"/>

        <Transition fallback=|| ()>
            {move || {
                let val = session.get();
                val.map(|result| match result {
                    Ok(Some(info)) => {
                        view! { <AuthenticatedApp username=info.username is_admin=info.is_admin/> }.into_any()
                    }
                    _ => {
                        // Check if we're on the OIDC link page before falling back to login.
                        // use_location() requires a <Router> ancestor which doesn't exist in the
                        // unauthenticated path, so read the URL directly from the browser instead.
                        #[cfg(feature = "hydrate")]
                        {
                            let pathname = web_sys::window()
                                .and_then(|w| w.location().pathname().ok())
                                .unwrap_or_default();
                            if pathname.starts_with("/auth/oidc/link") {
                                return view! { <OidcLinkPage/> }.into_any();
                            }
                        }
                        view! { <LoginPage/> }.into_any()
                    }
                })
            }}
        </Transition>
    }
}

// ── Authenticated shell ────────────────────────────────────────────────────────

#[component]
fn AuthenticatedApp(username: String, is_admin: bool) -> impl IntoView {
    // ── Live-update triggers ─────────────────────────────────────────────────
    // Two separate signals so SSE notifications only refetch the resource(s)
    // that actually changed, avoiding a full redraw on every sync event.
    let sidebar_refresh = SidebarRefresh(RwSignal::new(0u32));
    let events_refresh = EventsRefresh(RwSignal::new(0u32));
    let contacts_refresh = ContactsRefresh(RwSignal::new(0u32));
    let contacts_needs_relayout = ContactsNeedsRelayoutCtx(RwSignal::new(false));
    let notes_refresh = NotesRefresh(RwSignal::new(0u32));
    provide_context(sidebar_refresh);
    provide_context(events_refresh);
    provide_context(contacts_refresh);
    provide_context(contacts_needs_relayout);
    provide_context(notes_refresh);

    let done_refresh = DoneRefresh(RwSignal::new(0u32));
    provide_context(done_refresh);

    #[cfg(feature = "hydrate")]
    {
        use std::cell::RefCell;
        use wasm_bindgen::prelude::*;

        // Thread-local holds the active EventSource across login/logout cycles.
        // On WASM everything is single-threaded, so this is safe.
        thread_local! {
            static ACTIVE_ES: RefCell<Option<web_sys::EventSource>> = const { RefCell::new(None) };
        }

        // Close any EventSource left by a previous login.
        ACTIVE_ES.with(|cell| {
            if let Some(old) = cell.borrow_mut().take() {
                old.close();
            }
        });

        match web_sys::EventSource::new("/api/stream") {
            Ok(es) => {
                let cb = Closure::<dyn Fn(web_sys::MessageEvent)>::new(
                    move |ev: web_sys::MessageEvent| {
                        // Message format: "<kind>:<username>" where kind is
                        // "cals", "events", or "all" (see api.rs ChangeKind).
                        let data = ev.data().as_string().unwrap_or_default();
                        let kind = data.split(':').next().unwrap_or("all");
                        match kind {
                            "cals" => sidebar_refresh.0.update(|n| *n += 1),
                            "events" => events_refresh.0.update(|n| *n += 1),
                            _ => {
                                sidebar_refresh.0.update(|n| *n += 1);
                                events_refresh.0.update(|n| *n += 1);
                            }
                        }
                    },
                );
                es.set_onmessage(Some(cb.as_ref().unchecked_ref()));
                // Leak the callback — its lifetime is tied to the EventSource.
                cb.forget();

                ACTIVE_ES.with(|cell| {
                    *cell.borrow_mut() = Some(es);
                });
            }
            Err(e) => {
                leptos::logging::warn!(
                    "[tidl] EventSource::new(\"/api/stream\") failed: {:?} — \
                     live updates disabled",
                    e
                );
            }
        }
    }

    // ── Calendar data — persisted so CalendarView remounts skip the blank
    // loading state and show the previous session's data immediately (Transition-
    // like behaviour without requiring <Transition>+Suspend::new restructuring).
    let calendar_loaded = CalendarLoadedCtx(RwSignal::new(false));
    let cached_events: CachedEventsCtx = RwSignal::new(vec![]);
    let all_calendars_ctx: AllCalendarsCtx = RwSignal::new(vec![]);
    let current_username = CurrentUsernameCtx(RwSignal::new(username.clone()));
    provide_context(calendar_loaded);
    provide_context(cached_events);
    provide_context(all_calendars_ctx);
    provide_context(current_username);

    // ── Filter sidebar state (shared across Calendar and Focus) ─────────────
    // Persists across route changes so toggling a calendar on /focus keeps
    // that state when the user navigates to the Calendar tab and back.
    let hidden_md = HiddenMdSourcesCtx(RwSignal::new(std::collections::HashSet::new()));
    provide_context(hidden_md);
    let hidden_notes = HiddenNoteCollectionsCtx(RwSignal::new(std::collections::HashSet::new()));
    provide_context(hidden_notes);
    let hidden_labels = HiddenNoteLabelsCtx(RwSignal::new(std::collections::HashSet::new()));
    provide_context(hidden_labels);
    let hidden_contact_collections = crate::components::contacts::HiddenContactCollectionsCtx(
        RwSignal::new(std::collections::HashSet::new()),
    );
    provide_context(hidden_contact_collections);

    // ── Focus suppressions — now permanent (backlog #69), fetched once per
    // app session (mirrors prefs_res below) rather than refetched on every
    // Focus mount, consistent with why this was lifted to context in the
    // first place (surviving route navigation, not just page reloads).
    let suppressed_focus_items =
        SuppressedFocusItemsCtx(RwSignal::new(std::collections::HashMap::new()));
    provide_context(suppressed_focus_items);

    let suppressed_res = Resource::new(|| (), move |_| list_suppressed_focus_items());
    Effect::new(move |_: Option<()>| {
        if let Some(Ok(keys)) = suppressed_res.get() {
            suppressed_focus_items
                .0
                .set(keys.into_iter().map(|k| (k, true)).collect());
        }
    });

    // ── Admin flag ───────────────────────────────────────────────────────────
    let admin_ctx = IsAdminCtx(RwSignal::new(is_admin));
    provide_context(admin_ctx);

    // ── Settings modal open/tab state — shared via context so any descendant
    // (e.g. a sidebar's share icon, backlog #65) can open Settings directly
    // on a specific tab, not just the gear button in the nav bar.
    let open_settings = OpenSettingsCtx(RwSignal::new(None));
    provide_context(open_settings);

    // Shared task/await edit modal (backlog #121). Rendered at the app root so
    // any route — Focus previews, note cards, the note editor — can open it.
    let todo_modal = TodoModalCtx(RwSignal::new(None));
    provide_context(todo_modal);

    // Fullscreen inline-image lightbox (backlog #140). Rendered at the app
    // root so a note card's transformed drag wrapper can't clip it.
    let image_lightbox = ImageLightboxCtx(RwSignal::new(None));
    provide_context(image_lightbox);

    // ── Preferences ──────────────────────────────────────────────────────────
    let prefs: UserPrefsCtx = RwSignal::new(UserPrefs::default());
    provide_context(prefs);

    // Per-browser editor/clone settings. Kept separate from `UserPrefsCtx`
    // because these values must not follow the account onto another device.
    let local_editor = LocalEditorCtx {
        prefs: RwSignal::new(LocalEditorPrefs::default()),
        hydrated: RwSignal::new(false),
        detected_on_server: RwSignal::new(false),
    };
    provide_context(local_editor);
    #[cfg(feature = "hydrate")]
    {
        Effect::new(move |_: Option<()>| {
            let host = web_sys::window()
                .and_then(|w| w.location().hostname().ok())
                .unwrap_or_default();
            local_editor.detected_on_server.set(host_is_loopback(&host));
            local_editor.prefs.set(load_prefs());
            local_editor.hydrated.set(true);
        });
    }

    let prefs_res = Resource::new(|| (), move |_| get_preferences());
    Effect::new(move |_: Option<()>| {
        if let Some(Ok(p)) = prefs_res.get() {
            prefs.set(p);
        }
    });

    // Apply/remove the `dark` class on <html> whenever the preference changes.
    // Also persists to localStorage so the anti-FOUC script in Shell can apply
    // dark mode immediately on the next page load, before WASM is ready.
    Effect::new(move |_: Option<()>| {
        let is_dark = prefs.get().dark_mode;
        #[cfg(target_arch = "wasm32")]
        {
            if let Some(win) = web_sys::window() {
                if let Some(el) = win.document().and_then(|d| d.document_element()) {
                    if is_dark {
                        let _ = el.class_list().add_1("dark");
                    } else {
                        let _ = el.class_list().remove_1("dark");
                    }
                }
                // Persist for anti-FOUC on next load.
                if let Ok(Some(ls)) = win.local_storage() {
                    let _ = ls.set_item("tidl-dark", if is_dark { "1" } else { "0" });
                }
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        let _ = is_dark;
    });

    // ── Priority curves (backlog #68) ────────────────────────────────────────
    let priority_curves: PriorityCurvesCtx =
        PriorityCurvesCtx(RwSignal::new(PriorityCurves::default()));
    provide_context(priority_curves);

    let priority_curves_res = Resource::new(|| (), move |_| get_priority_curves());
    Effect::new(move |_: Option<()>| {
        if let Some(Ok(c)) = priority_curves_res.get() {
            priority_curves.0.set(c);
        }
    });

    // ── Logout ───────────────────────────────────────────────────────────────
    let is_pending = RwSignal::new(false);

    let on_logout = move |_| {
        if is_pending.get_untracked() {
            return;
        }
        is_pending.set(true);
        leptos::task::spawn_local(async move {
            let _ = do_logout().await;
            // Reload the page so the server renders the login page with the
            // cleared cookie — same rationale as post_auth (full reload is
            // more reliable than a reactive trigger update).
            #[cfg(target_arch = "wasm32")]
            if let Some(window) = web_sys::window() {
                let _ = window.location().reload();
            }
        });
    };

    // ── Profile modal ────────────────────────────────────────────────────────
    let show_profile = RwSignal::new(false);

    // ── Markdown file finder (backlog #75) — Ctrl/Cmd+K quick-open, plus a
    // button next to "+" in the filter sidebar's Markdown section (its scope
    // is markdown-source files only, so it lives there rather than the main
    // navbar, which read as a global/app-wide search). Opening a file hands
    // its `PreviewState` up here rather than the finder rendering
    // `PreviewOverlay` itself, so closing the finder (which disposes it and
    // everything it owns) doesn't also tear down the preview it just opened.
    let show_file_finder = RwSignal::new(false);
    let file_finder_preview: RwSignal<Option<PreviewState>> = RwSignal::new(None);
    provide_context(ShowMarkdownFinderCtx(show_file_finder));

    // ── Escape closes the topmost modal (backlog #123) ───────────────────────
    #[allow(unused_variables)]
    let modal_esc = crate::modal_esc::provide_modal_esc();

    #[cfg(target_arch = "wasm32")]
    {
        let handle = window_event_listener(leptos::ev::keydown, move |ev| {
            // Modals win over the Ctrl/Cmd-K toggle: Escape is handled first,
            // and a nested chooser that already consumed the event (mention
            // popup, file finder) leaves it `default_prevented`, so this
            // listener leaves the modal open too.
            if ev.key() == "Escape" && !ev.default_prevented() {
                if modal_esc.close_topmost() {
                    ev.prevent_default();
                }
                return;
            }
            if (ev.ctrl_key() || ev.meta_key()) && ev.key().eq_ignore_ascii_case("k") {
                ev.prevent_default();
                show_file_finder.update(|s| *s = !*s);
            }
        });
        on_cleanup(move || handle.remove());
    }

    view! {
        <Router>
            <main class="container">
                <nav class="navbar">
                    <h1 class="logo"><Logo/></h1>
                    <ul class="nav-links">
                        <li><A href="/" exact=true>"Calendar"</A></li>
                        <li><A href="/focus">"Focus"</A></li>
                        <li><A href="/notes">"Notes"</A></li>
                        <li><A href="/contacts">"Contacts"</A></li>
                        <li><A href="/done">"Done"</A></li>
                    </ul>
                    <div class="nav-user">
                        <button
                            class="nav-username nav-username-btn"
                            title="Edit profile"
                            on:click=move |_| show_profile.set(true)
                        >
                            {move || current_username.0.get()}
                        </button>
                        <button
                            class="settings-btn"
                            title="Settings"
                            on:click=move |_| open_settings.0.set(Some("general"))
                        >
                            "⚙"
                        </button>
                        <button
                            class="logout-btn"
                            on:click=on_logout
                            disabled=move || is_pending.get()
                        >
                            "Sign out"
                        </button>
                    </div>
                </nav>

                <Routes fallback=|| view! { <p>"Page not found."</p> }>
                    <Route path=path!("/") view=CalendarView/>
                    <Route path=path!("/focus") view=FocusView/>
                    <Route path=path!("/notes") view=NotesView/>
                    <Route path=path!("/contacts") view=ContactsView/>
                    <Route path=path!("/done") view=DoneView/>
                </Routes>

                <footer class="app-footer">
                    <span title="tidl version">
                        {concat!("tidl ", env!("CARGO_PKG_VERSION"), " · ", env!("TIDL_GIT_SHA"))}
                    </span>
                </footer>
            </main>

            {move || open_settings.0.get().map(|tab| view! {
                <SettingsModal initial_tab=tab on_close=move || open_settings.0.set(None)/>
            })}

            {move || show_profile.get().then(|| view! {
                <ProfileModal on_close=move || show_profile.set(false)/>
            })}

            {move || show_file_finder.get().then(|| view! {
                <MarkdownFileFinder
                    on_close=move || show_file_finder.set(false)
                    on_open=move |ps| {
                        file_finder_preview.set(Some(ps));
                        show_file_finder.set(false);
                    }
                />
            })}

            {move || file_finder_preview.get().map(|ps| view! {
                <PreviewOverlay state=ps on_close=move || file_finder_preview.set(None)/>
            })}

            {move || todo_modal.0.get().map(|state| view! {
                <crate::components::focus::TodoItemModal
                    state=state
                    refresh=events_refresh.0
                    on_close=move || todo_modal.0.set(None)
                />
            })}

            {move || image_lightbox.0.get().map(|src| view! {
                <crate::components::image_lightbox::ImageLightbox
                    src=src
                    on_close=move || image_lightbox.0.set(None)
                />
            })}
        </Router>
    }
}

// ── Login page ────────────────────────────────────────────────────────────────

/// After successful authentication, transition to the authenticated shell.
///
/// Common case (no `login_next`): increment the session trigger so the
/// `session` Resource re-fetches and mounts `AuthenticatedApp` reactively.
/// WASM stays live — no page reload, no re-download, instant transition.
///
/// The `Set-Cookie` from the login response is committed by the browser
/// before the `async` block's `.await` returns, so it is present in the
/// cookie jar when the trigger fires and `get_session_info` runs.
///
/// `login_next` case: a full navigation is still used because the target
/// path may differ from the current one and the router needs to mount the
/// right route.
fn post_auth(_trigger: SessionTrigger) {
    #[cfg(target_arch = "wasm32")]
    {
        // Prefer a full page navigation over a reactive trigger.update() so
        // that the Router, context providers, and event handlers are all
        // rebuilt cleanly by SSR+hydration rather than via a reactive
        // transition (which is fragile across logout/login cycles in tachys).
        // The WASM bundle is cached by the browser so the reload is fast.
        let login_next = web_sys::window()
            .and_then(|w| w.location().search().ok())
            .and_then(|q| {
                if q.is_empty() {
                    return None;
                }
                web_sys::UrlSearchParams::new_with_str(&q)
                    .ok()
                    .and_then(|p| p.get("login_next"))
                    .filter(|v| v.starts_with('/'))
            });

        if let Some(window) = web_sys::window() {
            match login_next {
                Some(next) => {
                    let _ = window.location().set_href(&next);
                }
                None => {
                    let _ = window.location().reload();
                }
            }
        }
    }
}

/// State machine for the unauthenticated page.
#[derive(Clone, PartialEq)]
enum LoginView {
    Login,
    Signup,
    /// (username, email) — waiting for email verification code after signup.
    VerifyEmail(String, String),
    ForgotPassword,
    /// email — waiting for password-reset code.
    ResetPassword(String),
}

#[component]
fn LoginPage() -> impl IntoView {
    let trigger = use_context::<SessionTrigger>().expect("SessionTrigger in context");
    let view_state: RwSignal<LoginView> = RwSignal::new(LoginView::Login);
    let reg_res = Resource::new(|| (), |_| get_registration_enabled());
    let oidc_res = Resource::new(|| (), |_| get_oidc_config());

    view! {
        <div class="login-container">
            <div class="login-box">
                <h1 class="login-logo"><Logo/></h1>
                {move || match view_state.get() {
                    LoginView::VerifyEmail(username, email) => view! {
                        <VerifyEmailForm
                            trigger
                            username
                            email
                            on_back=move || view_state.set(LoginView::Login)
                        />
                    }.into_any(),
                    LoginView::Signup => view! {
                        <SignupForm
                            on_verified=move |username, email| {
                                view_state.set(LoginView::VerifyEmail(username, email));
                            }
                            on_sign_in=move || view_state.set(LoginView::Login)
                        />
                    }.into_any(),
                    LoginView::ForgotPassword => view! {
                        <ForgotPasswordForm
                            on_sent=move |email| view_state.set(LoginView::ResetPassword(email))
                            on_back=move || view_state.set(LoginView::Login)
                        />
                    }.into_any(),
                    LoginView::ResetPassword(email) => view! {
                        <ResetPasswordForm
                            trigger
                            email
                            on_back=move || view_state.set(LoginView::ForgotPassword)
                        />
                    }.into_any(),
                    LoginView::Login => {
                        let reg_enabled = reg_res.get()
                            .and_then(|r| r.ok())
                            .unwrap_or(true);
                        let oidc_provider = oidc_res.get()
                            .and_then(|r| r.ok())
                            .flatten();
                        view! {
                            <LoginForm
                                trigger
                                on_create_account=move || view_state.set(LoginView::Signup)
                                on_forgot_password=move || view_state.set(LoginView::ForgotPassword)
                                show_create_account=reg_enabled
                                oidc_provider=oidc_provider
                            />
                        }.into_any()
                    }
                }}
            </div>
        </div>
    }
}

#[component]
fn LoginForm(
    trigger: SessionTrigger,
    on_create_account: impl Fn() + 'static,
    on_forgot_password: impl Fn() + 'static,
    show_create_account: bool,
    oidc_provider: Option<String>,
) -> impl IntoView {
    let email = RwSignal::new(String::new());
    let password = RwSignal::new(String::new());
    let error = RwSignal::new(Option::<String>::None);
    let pending = RwSignal::new(false);

    let on_submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        if pending.get_untracked() {
            return;
        }
        let e = email.get_untracked();
        let p = password.get_untracked();
        if e.is_empty() || p.is_empty() {
            return;
        }
        pending.set(true);
        error.set(None);
        leptos::task::spawn_local(async move {
            match do_login(e, p).await {
                Ok(()) => {
                    pending.set(false);
                    post_auth(trigger);
                }
                Err(e) => {
                    pending.set(false);
                    error.set(Some(format_server_fn_error(&e.to_string())));
                }
            }
        });
    };

    view! {
        {move || error.get().filter(|e| !e.is_empty()).map(|e| view! { <p class="login-error">{e}</p> })}
        {oidc_provider.map(|name| view! {
            <a href="/auth/oidc/start" class="login-btn login-btn-sso">
                {format!("Sign in with {}", name)}
            </a>
            <div class="login-divider"><span>"or"</span></div>
        })}
        <form on:submit=on_submit>
            <input
                type="email"
                class="login-input"
                placeholder="Email"
                autocomplete="email"
                prop:value=move || email.get()
                on:input=move |ev| email.set(event_target_value(&ev))
            />
            <input
                type="password"
                class="login-input"
                placeholder="Password"
                autocomplete="current-password"
                prop:value=move || password.get()
                on:input=move |ev| password.set(event_target_value(&ev))
            />
            <button
                type="submit"
                class="login-btn"
                disabled=move || pending.get()
            >
                {move || if pending.get() { "Signing in…" } else { "Sign in" }}
            </button>
        </form>
        <p class="login-switch">
            <button class="login-switch-link" on:click=move |_| on_forgot_password()>
                "Forgot password?"
            </button>
        </p>
        {show_create_account.then(|| view! {
            <p class="login-switch">
                "Don't have an account? "
                <button class="login-switch-link" on:click=move |_| on_create_account()>
                    "Create account"
                </button>
            </p>
        })}
    }
}

#[component]
fn SignupForm(
    /// Called with `(username, email)` on successful signup to transition to
    /// the email verification step.
    on_verified: impl Fn(String, String) + 'static,
    on_sign_in: impl Fn() + 'static,
) -> impl IntoView {
    let username = RwSignal::new(String::new());
    let display = RwSignal::new(String::new());
    let email = RwSignal::new(String::new());
    let password = RwSignal::new(String::new());
    let confirm = RwSignal::new(String::new());
    let error = RwSignal::new(Option::<String>::None);
    let pending = RwSignal::new(false);

    let on_verified = std::rc::Rc::new(on_verified);

    let on_submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        if pending.get_untracked() {
            return;
        }
        let u = username.get_untracked();
        let d = display.get_untracked();
        let e = email.get_untracked();
        let p = password.get_untracked();
        let c = confirm.get_untracked();
        if u.is_empty() || e.is_empty() || p.is_empty() {
            error.set(Some("Please fill in all required fields".to_string()));
            return;
        }
        if p != c {
            error.set(Some("Passwords do not match".to_string()));
            return;
        }
        pending.set(true);
        error.set(None);
        let on_verified = std::rc::Rc::clone(&on_verified);
        leptos::task::spawn_local(async move {
            match do_signup(u, e.clone(), d, p).await {
                Ok(returned_username) => {
                    pending.set(false);
                    on_verified(returned_username, e);
                }
                Err(err) => {
                    pending.set(false);
                    error.set(Some(format_server_fn_error(&err.to_string())));
                }
            }
        });
    };

    view! {
        {move || error.get().filter(|e| !e.is_empty()).map(|e| view! { <p class="login-error">{e}</p> })}
        <form on:submit=on_submit>
            <input
                type="text"
                class="login-input"
                placeholder="Username"
                autocomplete="username"
                prop:value=move || username.get()
                on:input=move |ev| username.set(event_target_value(&ev))
            />
            <input
                type="text"
                class="login-input"
                placeholder="Display name (optional)"
                prop:value=move || display.get()
                on:input=move |ev| display.set(event_target_value(&ev))
            />
            <input
                type="email"
                class="login-input"
                placeholder="Email"
                autocomplete="email"
                prop:value=move || email.get()
                on:input=move |ev| email.set(event_target_value(&ev))
            />
            <input
                type="password"
                class="login-input"
                placeholder="Password (min. 8 characters)"
                autocomplete="new-password"
                prop:value=move || password.get()
                on:input=move |ev| password.set(event_target_value(&ev))
            />
            <input
                type="password"
                class="login-input"
                placeholder="Confirm password"
                autocomplete="new-password"
                prop:value=move || confirm.get()
                on:input=move |ev| confirm.set(event_target_value(&ev))
            />
            <button
                type="submit"
                class="login-btn"
                disabled=move || pending.get()
            >
                {move || if pending.get() { "Creating account…" } else { "Create account" }}
            </button>
        </form>
        <p class="login-switch">
            "Already have an account? "
            <button class="login-switch-link" on:click=move |_| on_sign_in()>
                "Sign in"
            </button>
        </p>
    }
}

// ── Forgot / reset password forms ────────────────────────────────────────────

#[component]
fn ForgotPasswordForm(
    /// Called with the email address once the reset code has been sent.
    on_sent: impl Fn(String) + 'static,
    on_back: impl Fn() + 'static,
) -> impl IntoView {
    let email = RwSignal::new(String::new());
    let error = RwSignal::new(Option::<String>::None);
    let pending = RwSignal::new(false);

    let on_sent = std::rc::Rc::new(on_sent);

    let on_submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        if pending.get_untracked() {
            return;
        }
        let e = email.get_untracked();
        if e.trim().is_empty() {
            error.set(Some("Please enter your email address".to_string()));
            return;
        }
        pending.set(true);
        error.set(None);
        let email_clone = e.clone();
        let on_sent = std::rc::Rc::clone(&on_sent);
        leptos::task::spawn_local(async move {
            match do_request_password_reset(email_clone.clone()).await {
                Ok(()) => {
                    pending.set(false);
                    on_sent(email_clone);
                }
                Err(e) => {
                    pending.set(false);
                    error.set(Some(format_server_fn_error(&e.to_string())));
                }
            }
        });
    };

    view! {
        <p class="verify-hint">"Enter your email address and we'll send you a reset code."</p>
        {move || error.get().filter(|e| !e.is_empty()).map(|e| view! { <p class="login-error">{e}</p> })}
        <form on:submit=on_submit>
            <input
                type="email"
                class="login-input"
                placeholder="Email"
                autocomplete="email"
                prop:value=move || email.get()
                on:input=move |ev| email.set(event_target_value(&ev))
            />
            <button
                type="submit"
                class="login-btn"
                disabled=move || pending.get()
            >
                {move || if pending.get() { "Sending…" } else { "Send reset code" }}
            </button>
        </form>
        <p class="login-switch">
            <button class="login-switch-link" on:click=move |_| on_back()>
                "← Back to sign in"
            </button>
        </p>
    }
}

#[component]
fn ResetPasswordForm(
    trigger: SessionTrigger,
    email: String,
    on_back: impl Fn() + 'static,
) -> impl IntoView {
    let code = RwSignal::new(String::new());
    let new_password = RwSignal::new(String::new());
    let confirm = RwSignal::new(String::new());
    let error = RwSignal::new(Option::<String>::None);
    let pending = RwSignal::new(false);

    let masked_email = {
        let at = email.find('@').unwrap_or(email.len());
        let local = &email[..at];
        let domain = &email[at..];
        if local.len() <= 2 {
            format!("{}***{}", &local[..1], domain)
        } else {
            format!("{}***{}", &local[..2], domain)
        }
    };

    let email_for_submit = email.clone();
    let on_submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        if pending.get_untracked() {
            return;
        }
        let c = code.get_untracked();
        let p = new_password.get_untracked();
        let cf = confirm.get_untracked();
        if c.trim().is_empty() {
            error.set(Some("Please enter the reset code".to_string()));
            return;
        }
        if p != cf {
            error.set(Some("Passwords do not match".to_string()));
            return;
        }
        let e = email_for_submit.clone();
        pending.set(true);
        error.set(None);
        leptos::task::spawn_local(async move {
            match do_reset_password(e, c, p).await {
                Ok(()) => {
                    pending.set(false);
                    post_auth(trigger);
                }
                Err(e) => {
                    pending.set(false);
                    error.set(Some(format_server_fn_error(&e.to_string())));
                }
            }
        });
    };

    view! {
        <p class="verify-hint">
            "If an account exists for "
            <strong>{masked_email}</strong>
            ", we've sent it a 6-digit code. Enter it below along with your new password."
        </p>
        {move || error.get().filter(|e| !e.is_empty()).map(|e| view! { <p class="login-error">{e}</p> })}
        <form on:submit=on_submit>
            <input
                type="text"
                class="login-input verify-code-input"
                placeholder="000000"
                inputmode="numeric"
                maxlength="6"
                autocomplete="one-time-code"
                prop:value=move || code.get()
                on:input=move |ev| code.set(event_target_value(&ev))
            />
            <input
                type="password"
                class="login-input"
                placeholder="New password (min. 8 characters)"
                autocomplete="new-password"
                prop:value=move || new_password.get()
                on:input=move |ev| new_password.set(event_target_value(&ev))
            />
            <input
                type="password"
                class="login-input"
                placeholder="Confirm new password"
                autocomplete="new-password"
                prop:value=move || confirm.get()
                on:input=move |ev| confirm.set(event_target_value(&ev))
            />
            <button
                type="submit"
                class="login-btn"
                disabled=move || pending.get()
            >
                {move || if pending.get() { "Resetting…" } else { "Reset password" }}
            </button>
        </form>
        <p class="login-switch">
            <button class="login-switch-link" on:click=move |_| on_back()>
                "← Try a different email"
            </button>
        </p>
    }
}

// ── OIDC account-link page ────────────────────────────────────────────────────

/// Rendered at /auth/oidc/link?state=… — prompts the user to enter their
/// local password to link an existing account with an OIDC identity.
#[component]
fn OidcLinkPage() -> impl IntoView {
    // No <Router> in the unauthenticated component tree, so read search from the
    // browser directly rather than via use_location().
    let (state_param, error_param) = {
        let search = {
            #[cfg(target_arch = "wasm32")]
            {
                web_sys::window()
                    .and_then(|w| w.location().search().ok())
                    .unwrap_or_default()
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                String::new()
            }
        };
        let params: std::collections::HashMap<_, _> = search
            .trim_start_matches('?')
            .split('&')
            .filter_map(|p| {
                let mut kv = p.splitn(2, '=');
                Some((kv.next()?.to_string(), kv.next()?.to_string()))
            })
            .collect();
        (
            params.get("state").cloned().unwrap_or_default(),
            params.get("error").cloned(),
        )
    };

    view! {
        <div class="login-container">
            <div class="login-box">
                <h1 class="login-logo"><Logo/></h1>
                <p class="login-subtitle">
                    "An account with this email already exists. "
                    "Enter your password to link your identity."
                </p>
                {error_param.map(|_| view! {
                    <p class="login-error">"Incorrect password. Please try again."</p>
                })}
                <form method="post" action="/auth/oidc/link">
                    <input type="hidden" name="state" value=state_param/>
                    <input
                        type="password"
                        name="password"
                        class="login-input"
                        placeholder="Your current password"
                        autocomplete="current-password"
                        required
                    />
                    <button type="submit" class="login-btn">"Link account"</button>
                </form>
                <p class="login-switch">
                    <a href="/login">"Cancel"</a>
                </p>
            </div>
        </div>
    }
}

// ── Email verification form ───────────────────────────────────────────────────

#[component]
fn VerifyEmailForm(
    trigger: SessionTrigger,
    username: String,
    email: String,
    on_back: impl Fn() + 'static,
) -> impl IntoView {
    let code = RwSignal::new(String::new());
    let error = RwSignal::new(Option::<String>::None);
    let pending = RwSignal::new(false);

    let on_submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        if pending.get_untracked() {
            return;
        }
        let c = code.get_untracked();
        let u = username.clone();
        if c.trim().is_empty() {
            error.set(Some("Please enter the verification code".to_string()));
            return;
        }
        pending.set(true);
        error.set(None);
        leptos::task::spawn_local(async move {
            match do_verify_email(u, c).await {
                Ok(()) => {
                    pending.set(false);
                    post_auth(trigger);
                }
                Err(e) => {
                    pending.set(false);
                    error.set(Some(format_server_fn_error(&e.to_string())));
                }
            }
        });
    };

    let masked_email = {
        // Show e.g. "a***@example.com" so the user knows where to look.
        let at = email.find('@').unwrap_or(email.len());
        let local = &email[..at];
        let domain = &email[at..];
        if local.len() <= 2 {
            format!("{}***{}", &local[..1], domain)
        } else {
            format!("{}***{}", &local[..2], domain)
        }
    };

    view! {
        <p class="verify-hint">
            "We sent a 6-digit code to "
            <strong>{masked_email}</strong>
            ". Enter it below to activate your account."
        </p>
        {move || error.get().filter(|e| !e.is_empty()).map(|e| view! { <p class="login-error">{e}</p> })}
        <form on:submit=on_submit>
            <input
                type="text"
                class="login-input verify-code-input"
                placeholder="000000"
                inputmode="numeric"
                maxlength="6"
                autocomplete="one-time-code"
                prop:value=move || code.get()
                on:input=move |ev| code.set(event_target_value(&ev))
            />
            <button
                type="submit"
                class="login-btn"
                disabled=move || pending.get()
            >
                {move || if pending.get() { "Verifying…" } else { "Verify email" }}
            </button>
        </form>
        <p class="login-switch">
            <button class="login-switch-link" on:click=move |_| on_back()>
                "← Use a different email"
            </button>
        </p>
    }
}
