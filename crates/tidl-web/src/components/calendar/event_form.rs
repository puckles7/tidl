//! Event create/edit dialog, alarm editor, and date-time input.

use super::date_helpers::{
    bump_end_date, bump_end_time, format_hm, input_to_server_dt, parse_hhmm,
};
use crate::api::{
    create_event, delete_event, delete_event_occurrence, update_event, update_event_occurrence,
    Alarm, Event, NewEvent,
};
use crate::app::{AllCalendarsCtx, CurrentUsernameCtx};
use crate::components::priority_slider::{PrioritySlider, PrioritySliderMode};
use crate::components::settings::UserPrefsCtx;
use crate::util::{format_server_fn_error, mouse_event_targets_self};
use leptos::prelude::*;
use std::sync::Arc;

// ── LastUsedCalendarCtx (defined here, exported for CalendarView to provide) ──

// Newtype, not a bare `RwSignal<String>` alias — Leptos's context lookup keys
// on TypeId, and a plain alias is indistinguishable from any other
// `RwSignal<String>` context (this collided with `CurrentUsernameCtx` in
// `app.rs`: `CalendarView` providing this shadowed `AuthenticatedApp`'s
// username signal for every descendant, silently reading "" instead of the
// real username and breaking event-visibility filtering).
#[derive(Clone, Copy)]
pub struct LastUsedCalendarCtx(pub RwSignal<String>);

// ── Calendar-select composite key ─────────────────────────────────────────────

/// Separator joining `(owner_username, calendar_name)` into the `<select>`
/// option value. A bare calendar name is ambiguous once shared calendars are
/// listed alongside owned ones — `merge_calendar_entries` already keys
/// `CalendarEntry` by the pair to avoid same-name collisions across owners
/// (see its doc comment in `types.rs`); this mirrors that here so the
/// dropdown's selection survives the same collision.
const CAL_KEY_SEP: char = '\u{1}';

fn cal_key(owner: &str, name: &str) -> String {
    format!("{owner}{CAL_KEY_SEP}{name}")
}

/// Split a `<select>` value back into `(owner, calendar_name)`. Falls back to
/// `(my_username, key)` for a bare (non-composite) key — e.g. the dropdown
/// never rendered (single calendar) and `key` is still the plain name it was
/// initialised with before this composite scheme existed.
fn split_cal_key(key: &str, my_username: &str) -> (String, String) {
    match key.split_once(CAL_KEY_SEP) {
        Some((owner, name)) => (owner.to_string(), name.to_string()),
        None => (my_username.to_string(), key.to_string()),
    }
}

// ── AlarmRowState ─────────────────────────────────────────────────────────────

#[derive(Clone)]
pub(super) struct AlarmRowState {
    pub action: RwSignal<String>,
    pub amount: RwSignal<String>,
    pub unit: RwSignal<String>,
    pub repeat_enabled: RwSignal<bool>,
    pub repeat_count: RwSignal<String>,
    pub repeat_delay_m: RwSignal<String>,
    pub raw_ical: Option<String>,
    pub is_raw: bool,
}

impl AlarmRowState {
    pub(super) fn new_default() -> Self {
        Self {
            action: RwSignal::new("display".into()),
            amount: RwSignal::new("30".into()),
            unit: RwSignal::new("minutes".into()),
            repeat_enabled: RwSignal::new(false),
            repeat_count: RwSignal::new("1".into()),
            repeat_delay_m: RwSignal::new("5".into()),
            raw_ical: None,
            is_raw: false,
        }
    }

    pub(super) fn from_alarm(a: &Alarm) -> Self {
        Self {
            action: RwSignal::new(a.action.clone()),
            amount: RwSignal::new(a.amount.to_string()),
            unit: RwSignal::new(a.unit.clone()),
            repeat_enabled: RwSignal::new(a.repeat_count.is_some()),
            repeat_count: RwSignal::new(a.repeat_count.unwrap_or(1).to_string()),
            repeat_delay_m: RwSignal::new(a.repeat_delay_m.unwrap_or(5).to_string()),
            raw_ical: a.raw_ical.clone(),
            is_raw: a.is_raw,
        }
    }

    pub(super) fn to_alarm(&self) -> Option<Alarm> {
        let amount: u32 = self.amount.get().trim().parse().ok()?;
        let repeat_count = if self.repeat_enabled.get() {
            self.repeat_count.get().trim().parse().ok()
        } else {
            None
        };
        let repeat_delay_m = if self.repeat_enabled.get() {
            self.repeat_delay_m.get().trim().parse().ok()
        } else {
            None
        };
        Some(Alarm {
            action: self.action.get(),
            amount,
            unit: self.unit.get(),
            repeat_count,
            repeat_delay_m,
            raw_ical: self.raw_ical.clone(),
            is_raw: self.is_raw,
        })
    }
}

// ── AlarmEditor ───────────────────────────────────────────────────────────────

#[component]
pub(super) fn AlarmEditor(alarms: RwSignal<Vec<AlarmRowState>>) -> impl IntoView {
    let add_alarm = move |_| {
        alarms.update(|v| v.push(AlarmRowState::new_default()));
    };

    view! {
        <div class="form-group">
            <div class="form-label-row">
                <label class="form-label">"Notifications"</label>
                <button type="button" class="btn-add-alarm" on:click=add_alarm>"+ Add"</button>
            </div>
            <div class="alarm-list">
                {move || alarms.get().into_iter().enumerate().map(|(idx, row)| {
                    let remove = move |_| { alarms.update(|v| { if idx < v.len() { v.remove(idx); } }); };
                    view! {
                        <div class="alarm-block">
                            {if row.is_raw {
                                Some(view! {
                                    <div class="alarm-raw-badge">
                                        "⚠ Custom trigger (read-only) — timing fields ignored"
                                    </div>
                                })
                            } else { None }}
                            <div class="alarm-row">
                                <select class="form-input alarm-action"
                                    on:change=move |ev| row.action.set(event_target_value(&ev))>
                                    <option value="display" selected=move || row.action.get()=="display">"Display"</option>
                                    <option value="audio"   selected=move || row.action.get()=="audio">"Audio"</option>
                                    <option value="email"   selected=move || row.action.get()=="email">"Email"</option>
                                </select>
                                <input class="form-input alarm-amount" type="number" min="1"
                                    prop:value=move || row.amount.get()
                                    on:input=move |ev| row.amount.set(event_target_value(&ev))
                                    disabled=row.is_raw />
                                <select class="form-input alarm-unit"
                                    on:change=move |ev| row.unit.set(event_target_value(&ev))
                                    disabled=row.is_raw >
                                    <option value="minutes" selected=move || row.unit.get()=="minutes">"min"</option>
                                    <option value="hours"   selected=move || row.unit.get()=="hours">"hr"</option>
                                    <option value="days"    selected=move || row.unit.get()=="days">"days"</option>
                                    <option value="weeks"   selected=move || row.unit.get()=="weeks">"weeks"</option>
                                </select>
                                <span class="alarm-before">"before"</span>
                                <button type="button" class="btn-remove-alarm" on:click=remove>"✕"</button>
                            </div>
                            <div class="alarm-repeat-row">
                                <label class="alarm-repeat-label">
                                    <input
                                        type="checkbox"
                                        prop:checked=move || row.repeat_enabled.get()
                                        on:change=move |ev| {
                                            use leptos::wasm_bindgen::JsCast;
                                            let checked = ev.target()
                                                .and_then(|t| t.dyn_into::<leptos::web_sys::HtmlInputElement>().ok())
                                                .map(|el| el.checked())
                                                .unwrap_or(false);
                                            row.repeat_enabled.set(checked);
                                        }
                                    />
                                    " Repeat"
                                </label>
                                {move || row.repeat_enabled.get().then(|| view! {
                                    <span class="alarm-repeat-fields">
                                        <input class="form-input alarm-repeat-n" type="number" min="1"
                                            prop:value=move || row.repeat_count.get()
                                            on:input=move |ev| row.repeat_count.set(event_target_value(&ev)) />
                                        "×, every"
                                        <input class="form-input alarm-repeat-delay" type="number" min="1"
                                            prop:value=move || row.repeat_delay_m.get()
                                            on:input=move |ev| row.repeat_delay_m.set(event_target_value(&ev)) />
                                        "min"
                                    </span>
                                })}
                            </div>
                        </div>
                    }
                }).collect_view()}
            </div>
        </div>
    }
}

// ── DateTimeInput ─────────────────────────────────────────────────────────────

/// A date + time input that adapts to the user's time-format preference.
///
/// - `all_day = true`  → single `<input type="date">` bound to `dt`
/// - `all_day = false` → date input + time input + a formatted hint label
///
/// `dt` stores `"YYYY-MM-DD"` (all-day) or `"YYYY-MM-DDTHH:MM"` (timed).
///
/// `min`, when given, is the Start field's own `dt` value — used on the End
/// field so it can never be set earlier than Start (buglist item 2: "End
/// date/time should never be before begin date/time"). Its date portion
/// (`.get(..10)`) becomes the native date picker's `min` attribute; when
/// End's date matches that same date, its time portion additionally filters
/// the time popup and rejects a typed time at or before Start's. Both are UI
/// niceties, not the authority — `EventFormDialog`'s submit handler still
/// rejects an out-of-order end on its own.
#[component]
pub(super) fn DateTimeInput(
    dt: RwSignal<String>,
    #[prop(into)] default_time: &'static str,
    all_day: bool,
    #[prop(optional)] min: Option<Signal<String>>,
) -> impl IntoView {
    let time_fmt = move || {
        use_context::<UserPrefsCtx>()
            .map(|s| s.get().time_format.clone())
            .unwrap_or_else(|| "12h".to_string())
    };
    // `min` may carry a bare date ("YYYY-MM-DD") or a full timed datetime
    // ("YYYY-MM-DDTHH:MM") — either way its first 10 chars are the date.
    let min_attr = move || {
        min.map(|m| m.get().get(..10).unwrap_or_default().to_string())
            .unwrap_or_default()
    };

    if all_day {
        view! {
            <input class="form-input" type="date"
                prop:value=move || dt.get()
                min=min_attr
                on:input=move |ev| dt.set(event_target_value(&ev)) />
        }
        .into_any()
    } else {
        // Open/close state for the click-to-pick popup below the time
        // field. Created once here (not inside the reactive closure below)
        // so its identity — and thus open/closed state — survives dt
        // changing, e.g. picking an option (which sets both `dt` and
        // `open` together) or a manual edit committing on blur.
        let open: RwSignal<bool> = RwSignal::new(false);
        let list_ref: NodeRef<leptos::html::Div> = NodeRef::new();

        // Some((h, m)) only when `min` carries a *timed* datetime (i.e. this
        // is the End field) whose date matches this field's own current
        // date — the one case where Start and End share a calendar day and
        // a clock-time ordering between them actually means anything.
        let min_time_same_day = move || -> Option<(u32, u32)> {
            let min_full = min?.get();
            if min_full.len() < 16 {
                return None;
            }
            let cur_date = dt.get();
            if cur_date.get(..10) != Some(&min_full[..10]) {
                return None;
            }
            parse_hhmm(&min_full[11..16])
        };
        let is_time_allowed = move |h: u32, m: u32| match min_time_same_day() {
            Some((mh, mm)) => h * 60 + m > mh * 60 + mm,
            None => true,
        };

        // Scroll the popup to the option nearest the field's current value
        // every time it opens — `list_ref` goes `None` → `Some` on each
        // open (the popup is fully unmounted while closed), so this effect
        // fires fresh each time rather than only on first mount. Mirrors
        // the `.settings-tabs` deep-link scroll-into-view in
        // `settings/mod.rs`.
        Effect::new(move |_| {
            #[cfg(target_arch = "wasm32")]
            if let Some(container) = list_ref.get() {
                if let Ok(Some(current_el)) =
                    container.query_selector(".form-time-combo-option--current")
                {
                    let opts = web_sys::ScrollIntoViewOptions::new();
                    opts.set_block(web_sys::ScrollLogicalPosition::Center);
                    current_el.scroll_into_view_with_scroll_into_view_options(&opts);
                }
            }
        });

        view! {
            <div class="form-datetime-split">
                <input class="form-input" type="date"
                    prop:value=move || dt.get().get(..10).unwrap_or("").to_string()
                    min=min_attr
                    on:input=move |ev| {
                        let d = event_target_value(&ev);
                        let s = dt.get();
                        let t = if s.len() >= 16 { s[11..16].to_string() } else { default_time.to_string() };
                        dt.set(format!("{}T{}", d, t));
                    } />
                // A single free-text time field — typing a value directly
                // always works (the underlying commit is always plain
                // "HH:MM", parsed by `parse_hhmm`, same mechanism the old
                // 24h-only free-text field used and which was already known
                // to render fine on Firefox Linux and Safari iOS). Focusing
                // it additionally opens a custom-drawn popup of 15-minute
                // marks to click instead of typing — built from plain
                // buttons/divs (not a native `<select>`/`<datalist>`/
                // `<input type="time">`), so its appearance and behavior
                // are fully ours and don't depend on a browser's native
                // picker chrome, which is what made the previous native-
                // control attempts inconsistent across those two browsers.
                <div class="form-time-combo-wrap">
                    <input
                        class="form-input form-time-combo"
                        type="text"
                        inputmode="numeric"
                        placeholder="HH:MM"
                        prop:value=move || {
                            let s = dt.get();
                            if s.len() >= 16 { s[11..16].to_string() } else { default_time.to_string() }
                        }
                        on:focus=move |_| open.set(true)
                        on:change=move |ev| {
                            let raw = event_target_value(&ev);
                            let parsed = parse_hhmm(&raw)
                                .filter(|&(h, m)| h < 24 && m < 60 && is_time_allowed(h, m));
                            if let Some((h, m)) = parsed {
                                let s = dt.get();
                                let d = if s.len() >= 10 { s[..10].to_string() } else { s };
                                dt.set(format!("{}T{:02}:{:02}", d, h, m));
                            }
                            open.set(false);
                        } />
                    {move || open.get().then(|| {
                        let fmt = time_fmt();
                        let val = {
                            let s = dt.get();
                            if s.len() >= 16 { s[11..16].to_string() } else { default_time.to_string() }
                        };
                        let (cur_h, cur_m) = parse_hhmm(&val).unwrap_or((0, 0));
                        // Nearest 15-minute mark to the current value — the
                        // option the scroll-into-view effect above targets.
                        let nearest_idx = (((cur_h * 60 + cur_m) as f64) / 15.0)
                            .round()
                            .clamp(0.0, 95.0) as u32;
                        view! {
                            // Invisible full-page layer that closes the popup on
                            // any click outside it — same idiom as WeekGrid's
                            // `ctx_menu` context-menu backdrop.
                            <div class="form-time-combo-backdrop" on:click=move |_| open.set(false)></div>
                            <div class="form-time-combo-list" node_ref=list_ref
                                on:click=|ev: leptos::web_sys::MouseEvent| ev.stop_propagation()>
                                {(0u32..96).filter_map(|i| {
                                    let h = i / 4;
                                    let m = (i % 4) * 15;
                                    if !is_time_allowed(h, m) {
                                        return None;
                                    }
                                    let value = format!("{:02}:{:02}", h, m);
                                    let label = format_hm(h, m, &fmt);
                                    let cls = if i == nearest_idx {
                                        "form-time-combo-option form-time-combo-option--current"
                                    } else {
                                        "form-time-combo-option"
                                    };
                                    Some(view! {
                                        <button type="button" class=cls
                                            on:click=move |_| {
                                                let s = dt.get();
                                                let d = if s.len() >= 10 { s[..10].to_string() } else { s };
                                                dt.set(format!("{}T{}", d, value));
                                                open.set(false);
                                            }
                                        >{label}</button>
                                    })
                                }).collect_view()}
                            </div>
                        }
                    })}
                </div>
            </div>
        }.into_any()
    }
}

// ── Scope dialog action ───────────────────────────────────────────────────────

/// What action triggered the scope dialog — used to determine which server
/// function to call when the user picks a scope.
#[derive(Clone)]
enum ScopeDialogAction {
    Save(Box<NewEvent>),
    Delete,
}

// ── EventFormDialog ───────────────────────────────────────────────────────────

/// Unified create/edit event dialog.
///
/// - `event = None`       → "New Event"
/// - `event = Some(ev)`   → "Edit Event" (pre-fills all fields; shows recurrence scope dialog for series masters)
#[component]
pub fn EventFormDialog(
    event: Option<Event>,
    start: String,
    end: String,
    on_close: impl Fn() + Send + Sync + 'static,
    on_saved: impl Fn() + Send + Sync + 'static,
    /// When set, renders a "Show in calendar" button that navigates to the
    /// Calendar tab and flashes this event there. Only passed by Focus'
    /// event preview (backlog #157) — the Calendar tab's own usage leaves it
    /// `None` since the event is already on screen.
    #[prop(optional)]
    on_show_in_calendar: Option<Callback<()>>,
) -> impl IntoView {
    let is_new = event.is_none();
    let uid = event.as_ref().map(|e| e.id.clone());

    // Recurrence metadata from the incoming event (used for scope routing).
    let event_rrule = event.as_ref().and_then(|e| e.rrule.clone());
    let event_is_exception = event.as_ref().map(|e| e.is_exception).unwrap_or(false);
    let event_occurrence_start = event.as_ref().and_then(|e| e.occurrence_start.clone());

    let title = RwSignal::new(
        event
            .as_ref()
            .map(|e| e.summary.clone())
            .unwrap_or_default(),
    );
    let start_dt = RwSignal::new(start);
    let end_dt = RwSignal::new(end);

    // Keep End in lock-step with Start: shifting Start (date, time, or
    // both — via typing or the time popup) shifts End by the same delta,
    // preserving the event's length instead of silently collapsing or
    // inverting it (buglist item 2 — "End should be bumped when Start
    // changes"). Deliberately one-directional: this only watches
    // `start_dt`, so editing End never moves Start. Skipped entirely when
    // the all-day toggle just changed `dt`'s format underneath it (date-only
    // vs. timed have different lengths) — that transition already sets both
    // `start_dt`/`end_dt` explicitly in the toggle handler below.
    // The `min` prop on the End field (see `DateTimeInput`) is the other
    // half of "End should never be before Start" — this effect keeps it
    // consistent as Start moves, `min` stops the user from then also
    // dragging/typing End earlier than the new Start.
    let prev_start_dt = RwSignal::new(start_dt.get_untracked());
    Effect::new(move |_| {
        let s = start_dt.get();
        let old = prev_start_dt.get_untracked();
        if s != old && !old.is_empty() && !s.is_empty() && s.len() == old.len() {
            end_dt.update(|e| {
                *e = if s.len() >= 16 {
                    bump_end_time(&old, &s, e)
                } else {
                    bump_end_date(&old, &s, e)
                };
            });
        }
        prev_start_dt.set(s);
    });

    let all_day = RwSignal::new(event.as_ref().map(|e| e.all_day).unwrap_or(false));
    // Note: Event struct has no location field; location always starts empty when editing.
    let location = RwSignal::new(String::new());
    let description = RwSignal::new(
        event
            .as_ref()
            .and_then(|e| e.description.clone())
            .unwrap_or_default(),
    );
    let alarms: RwSignal<Vec<AlarmRowState>> = RwSignal::new(
        event
            .as_ref()
            .map(|e| e.alarms.iter().map(AlarmRowState::from_alarm).collect())
            .unwrap_or_default(),
    );
    // Event priority is opt-in (backlog #156): unchecked means no
    // `PRIORITY` property and no Focus attenuation.
    let priority_enabled = RwSignal::new(event.as_ref().is_some_and(|e| e.priority.is_some()));
    let priority = RwSignal::new(event.as_ref().and_then(|e| e.priority).unwrap_or(50));

    // Recurrence editor state — initialised from the event's RRULE if present.
    let recurrence = if let Some(ref rrule) = event_rrule {
        RecurrenceState::from_rrule(rrule)
    } else {
        RecurrenceState::default_state()
    };

    let saving = RwSignal::new(false);
    let deleting = RwSignal::new(false);
    let error = RwSignal::new(Option::<String>::None);

    // Scope dialog: Some when a recurring event save/delete needs user confirmation.
    let show_scope_dialog = RwSignal::new(Option::<ScopeDialogAction>::None);
    // Radio selection inside the scope dialog: "this" or "all".
    let scope_choice = RwSignal::new("this".to_string());

    let cals_ctx = use_context::<AllCalendarsCtx>();
    let last_used_cal_ctx = use_context::<LastUsedCalendarCtx>();
    let my_username = use_context::<CurrentUsernameCtx>()
        .map(|c| c.0.get_untracked())
        .unwrap_or_default();

    let initial_calendar = if is_new {
        last_used_cal_ctx
            .map(|s| s.0.get_untracked())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| cal_key(&my_username, "personal"))
    } else {
        event
            .as_ref()
            .map(|e| cal_key(&e.owner_username, &e.calendar))
            .unwrap_or_else(|| cal_key(&my_username, "personal"))
    };
    let selected_calendar = RwSignal::new(initial_calendar);

    let on_close: Arc<dyn Fn() + Send + Sync> = Arc::new(on_close);
    let on_saved: Arc<dyn Fn() + Send + Sync> = Arc::new(on_saved);
    let on_close2 = Arc::clone(&on_close);
    let on_close3 = Arc::clone(&on_close);
    let on_saved2 = Arc::clone(&on_saved);
    let on_saved3 = Arc::clone(&on_saved);
    let on_saved4 = Arc::clone(&on_saved);
    // Pre-clone uid so `submit` and the delete button closure can both use it.
    let uid_for_delete = uid.clone();

    // ── Submit handler ────────────────────────────────────────────────────────

    let event_rrule_for_submit = event_rrule.clone();
    let event_occ_start_for_sub = event_occurrence_start.clone();
    let uid_for_submit = uid.clone();
    // Clone recurrence so the submit closure can read it via move without
    // consuming the original (which is still needed for <RecurrenceEditor>).
    let recurrence_for_submit = recurrence.clone();

    let submit = move |ev: leptos::web_sys::SubmitEvent| {
        ev.prevent_default();
        let summary = title.get();
        if summary.trim().is_empty() {
            error.set(Some("Title is required.".into()));
            return;
        }
        let is_all_day = all_day.get();
        let (start, end) = if is_all_day {
            (start_dt.get(), end_dt.get())
        } else {
            (
                input_to_server_dt(&start_dt.get()),
                input_to_server_dt(&end_dt.get()),
            )
        };
        let invalid = if is_all_day {
            start > end
        } else {
            start >= end
        };
        if invalid {
            error.set(Some("End must be after start.".into()));
            return;
        }
        let loc = location.get();
        let desc = description.get();
        let alarm_list: Vec<Alarm> = alarms.get().iter().filter_map(|r| r.to_alarm()).collect();
        let cal_key_val = selected_calendar.get();
        let (cal_owner, cal_name) = split_cal_key(&cal_key_val, &my_username);
        let rrule = to_rrule(&recurrence_for_submit);
        let new_ev = NewEvent {
            summary,
            start,
            end,
            all_day: is_all_day,
            location: if loc.is_empty() { None } else { Some(loc) },
            description: if desc.is_empty() { None } else { Some(desc) },
            priority: priority_enabled.get().then(|| priority.get()),
            alarms: alarm_list,
            calendar: cal_name,
            calendar_owner: if cal_owner == my_username {
                None
            } else {
                Some(cal_owner)
            },
            rrule,
        };

        // For recurring master events, show the scope dialog instead of saving immediately.
        // Exception rows always edit "this occurrence only" — no dialog needed.
        if !is_new && event_rrule_for_submit.is_some() && !event_is_exception {
            scope_choice.set("this".into());
            show_scope_dialog.set(Some(ScopeDialogAction::Save(Box::new(new_ev))));
            return;
        }

        // Non-recurring, or new event, or editing an exception row directly.
        saving.set(true);
        error.set(None);
        let uid_for_save = uid_for_submit.clone();
        let occ_start = event_occ_start_for_sub.clone();
        let on_saved_c = Arc::clone(&on_saved2);
        leptos::task::spawn_local(async move {
            let result: Result<(), _> = if let Some(uid) = uid_for_save {
                if event_is_exception {
                    // Exception row — always update this occurrence.
                    if let Some(occ) = occ_start {
                        let series = uid
                            .rfind("_R")
                            .map(|pos| uid[..pos].to_string())
                            .unwrap_or_else(|| uid.clone());
                        update_event_occurrence(series, occ, new_ev).await
                    } else {
                        update_event(uid, new_ev).await
                    }
                } else {
                    update_event(uid, new_ev).await
                }
            } else {
                create_event(new_ev).await.map(|_| ())
            };
            match result {
                Ok(_) => {
                    saving.set(false);
                    if is_new {
                        if let Some(ctx) = last_used_cal_ctx {
                            ctx.0.set(cal_key_val);
                        }
                    }
                    on_saved_c();
                }
                Err(e) => {
                    saving.set(false);
                    error.set(Some(format_server_fn_error(&e.to_string())));
                }
            }
        });
    };

    let event_rrule_for_del = event_rrule.clone();
    let event_occ_for_del = event_occurrence_start.clone();

    // Scope dialog OK is defined inline in the view! macro to avoid clone on
    // closure issues — see the on:click handler on the "OK" button below.
    let uid_for_scope = uid.clone();
    let event_occ_start_scope = event_occurrence_start.clone();
    let backdrop_mousedown = RwSignal::new(false);
    let on_close_esc = Arc::clone(&on_close);
    let esc_id = crate::modal_esc::use_escape_close(move || {
        // Escape closes the innermost surface first: the recurrence scope
        // sub-dialog if it's up, otherwise the event form itself.
        if show_scope_dialog.get_untracked().is_some() {
            show_scope_dialog.set(None);
        } else {
            on_close_esc();
        }
    });

    view! {
        <div class="dialog-backdrop"
            data-esc-id=esc_id.to_string()
            on:mousedown=move |ev: leptos::web_sys::MouseEvent| backdrop_mousedown.set(mouse_event_targets_self(&ev))
            on:click=move |_| if backdrop_mousedown.get() { on_close() }
        >
            <div class="dialog" on:click=|ev: leptos::web_sys::MouseEvent| ev.stop_propagation()>
                <div class="dialog-header">
                    <h2 class="dialog-title">{if is_new { "New Event" } else { "Edit Event" }}</h2>
                    <button class="dialog-close" on:click=move |_| on_close2()>"✕"</button>
                </div>
                <form class="dialog-form" on:submit=submit>
                    <div class="form-group">
                        <label class="form-label">"Title"</label>
                        <input class="form-input" type="text"
                            placeholder=if is_new { "Event title" } else { "" }
                            autofocus=true
                            prop:value=move || title.get()
                            on:input=move |ev| title.set(event_target_value(&ev)) />
                    </div>
                    // ── Calendar selector ────────────────────────────────────────
                    {move || cals_ctx.map(|ctx| {
                        // Owned calendars always; shared calendars only when
                        // creating a brand-new event, and only at read-write
                        // permission (`c.read_only` is false for those) —
                        // `create_event` re-verifies the share server-side
                        // before writing under the owner's username. Editing
                        // an *existing* event stays owned-only here: its
                        // cross-owner write path (resolve_event_write_owner)
                        // resolves ownership from the event's current uid,
                        // not from whatever this dropdown shows, so letting
                        // an edit reassign to an arbitrary shared calendar
                        // isn't supported yet.
                        let cals: Vec<_> = ctx.get().into_iter()
                            .filter(|(c, _)| c.is_owned() || (is_new && !c.read_only))
                            .collect();
                        if cals.len() <= 1 {
                            None
                        } else {
                            Some(view! {
                                <div class="form-group">
                                    <label class="form-label">"Calendar"</label>
                                    <select class="form-input"
                                        on:change=move |ev| selected_calendar.set(event_target_value(&ev))>
                                        {cals.into_iter().map(|(cal, _)| {
                                            let key = cal_key(&cal.owner_username, &cal.calendar_name);
                                            let selected = selected_calendar.get() == key;
                                            view! {
                                                <option value=key selected=selected>
                                                    {cal.display_name}
                                                </option>
                                            }
                                        }).collect_view()}
                                    </select>
                                </div>
                            })
                        }
                    })}
                    // ── All-day toggle + date/time inputs ────────────────────────
                    {move || {
                        let is_all_day = all_day.get();
                        view! {
                            <div class="form-group">
                                <label class="allday-toggle-label">
                                    <input
                                        type="checkbox"
                                        prop:checked=move || all_day.get()
                                        on:change=move |ev| {
                                            use leptos::wasm_bindgen::JsCast;
                                            let checked = ev.target()
                                                .and_then(|t| t.dyn_into::<leptos::web_sys::HtmlInputElement>().ok())
                                                .map(|el| el.checked())
                                                .unwrap_or(false);
                                            if checked {
                                                let s = start_dt.get();
                                                let e = end_dt.get();
                                                start_dt.set(s.get(..10).unwrap_or(&s).to_string());
                                                end_dt.set(e.get(..10).unwrap_or(&e).to_string());
                                            } else {
                                                let s = start_dt.get();
                                                let e = end_dt.get();
                                                start_dt.set(format!("{}T09:00", s.get(..10).unwrap_or(&s)));
                                                end_dt.set(format!("{}T10:00", e.get(..10).unwrap_or(&e)));
                                            }
                                            all_day.set(checked);
                                        }
                                    />
                                    " All day"
                                </label>
                            </div>
                            <div class="form-row form-row--even">
                                <div class="form-group">
                                    <label class="form-label">"Start"</label>
                                    <DateTimeInput dt=start_dt default_time="09:00" all_day=is_all_day />
                                </div>
                                <div class="form-group">
                                    <label class="form-label">"End"</label>
                                    <DateTimeInput
                                        dt=end_dt
                                        default_time="10:00"
                                        all_day=is_all_day
                                        min=Signal::derive(move || start_dt.get())
                                    />
                                </div>
                            </div>
                        }
                    }}
                    // ── Recurrence editor ────────────────────────────────────────
                    <RecurrenceEditor state=recurrence.clone() />
                    <div class="form-group">
                        <label class="allday-toggle-label">
                            <input
                                type="checkbox"
                                prop:checked=move || priority_enabled.get()
                                on:change=move |ev| {
                                    use leptos::wasm_bindgen::JsCast;
                                    let checked = ev.target()
                                        .and_then(|t| t.dyn_into::<leptos::web_sys::HtmlInputElement>().ok())
                                        .map(|el| el.checked())
                                        .unwrap_or(false);
                                    priority_enabled.set(checked);
                                }
                            />
                            " Set priority (caps Focus urgency)"
                        </label>
                    </div>
                    {move || priority_enabled.get().then(|| view! {
                        <PrioritySlider priority=priority mode=PrioritySliderMode::Event />
                    })}
                    <div class="form-group">
                        <label class="form-label">"Location"</label>
                        <input class="form-input" type="text" placeholder="Optional location"
                            prop:value=move || location.get()
                            on:input=move |ev| location.set(event_target_value(&ev)) />
                    </div>
                    <div class="form-group">
                        <label class="form-label">"Description"</label>
                        <textarea class="form-input form-textarea" placeholder="Optional notes"
                            prop:value=move || description.get()
                            on:input=move |ev| description.set(event_target_value(&ev))>
                        </textarea>
                    </div>
                    <AlarmEditor alarms=alarms />
                    {move || error.get().map(|e| view!{<div class="form-error">{e}</div>})}
                    <div class="dialog-actions">
                        {(!is_new).then(|| {
                            let on_saved_del = Arc::clone(&on_saved4);
                            let uid_del      = uid_for_delete.clone().unwrap_or_default();
                            let rrule_del    = event_rrule_for_del.clone();
                            let occ_del      = event_occ_for_del.clone();
                            view! {
                            <div style="display:flex;gap:0.5rem;flex-wrap:wrap;margin-right:auto">
                                <button
                                    type="button" class="btn-danger"
                                    disabled=move || saving.get() || deleting.get()
                                    on:click=move |_| {
                                        // Recurring master: show scope dialog.
                                        if rrule_del.is_some() && !event_is_exception {
                                            scope_choice.set("this".into());
                                            show_scope_dialog.set(Some(ScopeDialogAction::Delete));
                                            return;
                                        }
                                        // Non-recurring or exception row: delete directly.
                                        let uid = uid_del.clone();
                                        deleting.set(true); error.set(None);
                                        let on_saved_d = Arc::clone(&on_saved_del);
                                        let occ = occ_del.clone();
                                        leptos::task::spawn_local(async move {
                                            let result: Result<(), _> = if event_is_exception {
                                                if let Some(occ_start) = occ {
                                                    let series = uid.rfind("_R")
                                                        .map(|pos| uid[..pos].to_string())
                                                        .unwrap_or_else(|| uid.clone());
                                                    delete_event_occurrence(series, occ_start).await
                                                } else {
                                                    delete_event(uid).await
                                                }
                                            } else {
                                                delete_event(uid).await
                                            };
                                            match result {
                                                Ok(_)  => { deleting.set(false); on_saved_d(); }
                                                Err(e) => { deleting.set(false); error.set(Some(format_server_fn_error(&e.to_string()))); }
                                            }
                                        });
                                    }
                                >
                                    {move || if deleting.get() { "Deleting…" } else { "Delete" }}
                                </button>
                                {on_show_in_calendar.map(|cb| view! {
                                    <button
                                        type="button"
                                        class="btn-secondary"
                                        disabled=move || saving.get() || deleting.get()
                                        on:click=move |_| cb.run(())
                                    >
                                        "Show in calendar"
                                    </button>
                                })}
                            </div>
                            }
                        })}
                        <button type="button" class="btn-cancel" on:click=move |_| on_close3()>"Cancel"</button>
                        <button type="submit" class="btn-primary"
                            disabled=move || saving.get() || deleting.get()>
                            {move || if saving.get() { "Saving…" } else if is_new { "Save Event" } else { "Save Changes" }}
                        </button>
                    </div>
                </form>

                // ── Edit-scope sub-dialog ────────────────────────────────────────
                // uid_for_scope and event_occ_start_scope are stored as signals
                // so the reactive closure can read them as Copy values each call.
                {move || show_scope_dialog.get().map(|action| {
                    let is_delete = matches!(action, ScopeDialogAction::Delete);
                    // Clone the Option<String> values here so the inner on:click closure
                    // can move them without consuming the outer closure's captures
                    // (outer must be FnMut; inner can be FnOnce).
                    let uid_s_click      = uid_for_scope.clone().unwrap_or_default();
                    let occ_start_click  = event_occ_start_scope.clone().unwrap_or_default();
                    let on_saved_click   = Arc::clone(&on_saved3);
                    view! {
                        <div class="scope-dialog-backdrop">
                            <div class="scope-dialog">
                                <p class="scope-dialog-title">"This is a recurring event."</p>
                                <div class="scope-options">
                                    <label class="scope-option">
                                        <input type="radio" name="scope"
                                            prop:checked=move || scope_choice.get()=="this"
                                            on:change=move |_| scope_choice.set("this".into()) />
                                        " This event only"
                                    </label>
                                    <label class="scope-option">
                                        <input type="radio" name="scope"
                                            prop:checked=move || scope_choice.get()=="all"
                                            on:change=move |_| scope_choice.set("all".into()) />
                                        " All events in the series"
                                    </label>
                                </div>
                                <div class="scope-dialog-actions">
                                    <button type="button" class="btn-cancel"
                                        on:click=move |_| show_scope_dialog.set(None)>"Cancel"</button>
                                    <button type="button"
                                        class=if is_delete { "btn-danger" } else { "btn-primary" }
                                        on:click=move |_| {
                                            // Inline scope-OK logic — closures don't implement Clone.
                                            let inner_action = match show_scope_dialog.get_untracked() {
                                                Some(a) => a,
                                                None    => return,
                                            };
                                            show_scope_dialog.set(None);
                                            let choice = scope_choice.get_untracked();
                                            saving.set(true); error.set(None);
                                            let uid_s       = uid_s_click.clone();
                                            let occ_start_s = occ_start_click.clone();
                                            let on_saved_c  = Arc::clone(&on_saved_click);
                                            leptos::task::spawn_local(async move {
                                                let result: Result<(), _> = match inner_action {
                                                    ScopeDialogAction::Save(new_ev) => {
                                                        if choice == "all" {
                                                            update_event(uid_s, *new_ev).await
                                                        } else {
                                                            // "this" — create/replace exception occurrence.
                                                            let series = uid_s.rfind("_R")
                                                                .map(|pos| uid_s[..pos].to_string())
                                                                .unwrap_or_else(|| uid_s.clone());
                                                            update_event_occurrence(series, occ_start_s, *new_ev).await
                                                        }
                                                    }
                                                    ScopeDialogAction::Delete => {
                                                        if choice == "all" {
                                                            delete_event(uid_s).await
                                                        } else {
                                                            let series = uid_s.rfind("_R")
                                                                .map(|pos| uid_s[..pos].to_string())
                                                                .unwrap_or_else(|| uid_s.clone());
                                                            delete_event_occurrence(series, occ_start_s).await
                                                        }
                                                    }
                                                };
                                                match result {
                                                    Ok(_) => { saving.set(false); on_saved_c(); }
                                                    Err(e) => { saving.set(false); error.set(Some(format_server_fn_error(&e.to_string()))); }
                                                }
                                            });
                                        }
                                    >"OK"</button>
                                </div>
                            </div>
                        </div>
                    }
                })}
            </div>
        </div>
    }
}

// ── RRULE string helpers ──────────────────────────────────────────────────────

/// Decomposed representation of a parsed or authored RRULE string.
/// All fields are plain strings for easy binding to RwSignal.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct RruleParts {
    /// "none" | "daily" | "weekly" | "monthly" | "yearly"
    pub freq: String,
    /// "1", "2", … (default "1")
    pub interval: String,
    /// Day codes when freq="weekly" — e.g. ["MO", "WE", "FR"]
    pub byday: Vec<String>,
    /// "never" | "count" | "until"
    pub end_type: String,
    /// Number of occurrences when end_type="count"
    pub count: String,
    /// Date in "YYYY-MM-DD" format when end_type="until"
    pub until: String,
}

impl Default for RruleParts {
    fn default() -> Self {
        Self {
            freq: "none".to_string(),
            interval: "1".to_string(),
            byday: vec![],
            end_type: "never".to_string(),
            count: "10".to_string(),
            until: String::new(),
        }
    }
}

/// Parse a stored RRULE value string (without "RRULE:" prefix) into `RruleParts`.
/// Unrecognised parameters (BYMONTH, BYSETPOS, …) are silently ignored.
pub(super) fn parse_rrule_parts(s: &str) -> RruleParts {
    let mut parts = RruleParts::default();
    for segment in s.split(';') {
        let Some((key, val)) = segment.split_once('=') else {
            continue;
        };
        match key.to_ascii_uppercase().as_str() {
            "FREQ" => {
                parts.freq = match val.to_ascii_uppercase().as_str() {
                    "DAILY" => "daily",
                    "WEEKLY" => "weekly",
                    "MONTHLY" => "monthly",
                    "YEARLY" => "yearly",
                    _ => "none",
                }
                .to_string();
            }
            "INTERVAL" => {
                parts.interval = val.to_string();
            }
            "BYDAY" => {
                parts.byday = val.split(',').map(|d| d.to_ascii_uppercase()).collect();
            }
            "COUNT" => {
                parts.end_type = "count".to_string();
                parts.count = val.to_string();
            }
            "UNTIL" => {
                // UNTIL may be "YYYYMMDDTHHmmSSZ" or "YYYYMMDD".
                // Convert to "YYYY-MM-DD" for the date input.
                let d = &val[..val.len().min(8)];
                if d.len() == 8 {
                    parts.until = format!("{}-{}-{}", &d[..4], &d[4..6], &d[6..8]);
                } else {
                    parts.until = val.to_string();
                }
                parts.end_type = "until".to_string();
            }
            _ => {}
        }
    }
    parts
}

/// Serialise `RruleParts` into a valid RRULE value string (without "RRULE:" prefix).
/// Returns `None` when `freq` is "none" (event does not repeat).
pub(super) fn build_rrule_string(parts: &RruleParts) -> Option<String> {
    if parts.freq == "none" {
        return None;
    }
    let freq = parts.freq.to_ascii_uppercase();
    let mut s = format!("FREQ={freq}");

    // Only emit INTERVAL when it is not the default of 1.
    if parts.interval != "1" && !parts.interval.is_empty() {
        s.push_str(&format!(";INTERVAL={}", parts.interval));
    }

    // BYDAY only makes sense for weekly recurrences.
    if parts.freq == "weekly" && !parts.byday.is_empty() {
        s.push_str(&format!(";BYDAY={}", parts.byday.join(",")));
    }

    match parts.end_type.as_str() {
        "count" if !parts.count.is_empty() => {
            s.push_str(&format!(";COUNT={}", parts.count));
        }
        "until" if !parts.until.is_empty() => {
            // Convert "YYYY-MM-DD" → "YYYYMMDD" for iCal.
            let compact: String = parts.until.chars().filter(|c| c.is_ascii_digit()).collect();
            s.push_str(&format!(";UNTIL={}", compact));
        }
        _ => {} // "never" or empty — no end constraint
    }

    Some(s)
}

// ── RecurrenceState ───────────────────────────────────────────────────────────

/// Client-only recurrence editor state — lives in WASM signals, never sent
/// over the wire directly.  `to_rrule` serialises it into `NewEvent.rrule`.
#[derive(Clone)]
pub(super) struct RecurrenceState {
    pub freq: RwSignal<String>,
    pub interval: RwSignal<String>,
    pub byday: RwSignal<Vec<String>>,
    pub end_type: RwSignal<String>,
    pub count: RwSignal<String>,
    pub until: RwSignal<String>,
}

impl RecurrenceState {
    pub(super) fn default_state() -> Self {
        let p = RruleParts::default();
        Self {
            freq: RwSignal::new(p.freq),
            interval: RwSignal::new(p.interval),
            byday: RwSignal::new(p.byday),
            end_type: RwSignal::new(p.end_type),
            count: RwSignal::new(p.count),
            until: RwSignal::new(p.until),
        }
    }

    pub(super) fn from_rrule(s: &str) -> Self {
        let p = parse_rrule_parts(s);
        Self {
            freq: RwSignal::new(p.freq),
            interval: RwSignal::new(p.interval),
            byday: RwSignal::new(p.byday),
            end_type: RwSignal::new(p.end_type),
            count: RwSignal::new(p.count),
            until: RwSignal::new(p.until),
        }
    }
}

/// Serialise `RecurrenceState` to an RRULE value string, or `None` when freq
/// is "none".  This is what gets stored in `NewEvent.rrule`.
pub(super) fn to_rrule(state: &RecurrenceState) -> Option<String> {
    build_rrule_string(&RruleParts {
        freq: state.freq.get(),
        interval: state.interval.get(),
        byday: state.byday.get(),
        end_type: state.end_type.get(),
        count: state.count.get(),
        until: state.until.get(),
    })
}

// ── RecurrenceEditor ──────────────────────────────────────────────────────────

#[component]
pub(super) fn RecurrenceEditor(state: RecurrenceState) -> impl IntoView {
    static WEEKDAYS: &[(&str, &str)] = &[
        ("MO", "Mo"),
        ("TU", "Tu"),
        ("WE", "We"),
        ("TH", "Th"),
        ("FR", "Fr"),
        ("SA", "Sa"),
        ("SU", "Su"),
    ];

    view! {
        <div class="form-group recurrence-editor">
            <label class="form-label">"Recurrence"</label>
            <div class="recurrence-row">
                <label class="form-label-inline">"Repeats"</label>
                <select class="form-input recurrence-freq"
                    on:change=move |ev| state.freq.set(event_target_value(&ev))>
                    <option value="none"    selected=move || state.freq.get()=="none">"Does not repeat"</option>
                    <option value="daily"   selected=move || state.freq.get()=="daily">"Daily"</option>
                    <option value="weekly"  selected=move || state.freq.get()=="weekly">"Weekly"</option>
                    <option value="monthly" selected=move || state.freq.get()=="monthly">"Monthly"</option>
                    <option value="yearly"  selected=move || state.freq.get()=="yearly">"Yearly"</option>
                </select>
            </div>

            {move || (state.freq.get() != "none").then(|| {
                let state2 = state.clone();
                view! {
                    <div class="recurrence-details">
                        // ── Interval ─────────────────────────────────────────
                        <div class="recurrence-row">
                            <label class="form-label-inline">"Every"</label>
                            <input class="form-input recurrence-interval" type="number" min="1"
                                prop:value=move || state.interval.get()
                                on:input=move |ev| state.interval.set(event_target_value(&ev)) />
                            <span class="recurrence-freq-label">
                                {move || match state.freq.get().as_str() {
                                    "daily"   => "day(s)",
                                    "weekly"  => "week(s)",
                                    "monthly" => "month(s)",
                                    "yearly"  => "year(s)",
                                    _         => "",
                                }}
                            </span>
                        </div>

                        // ── Weekday picker (weekly only) ──────────────────────
                        {
                            // Clone state2 so we can still use it in "Ends" below after
                            // it's been moved into the weekly-picker reactive closure.
                            let state2_for_weekly = state2.clone();
                            move || (state2_for_weekly.freq.get() == "weekly").then(|| {
                                let state3 = state2_for_weekly.clone();
                                view! {
                                    <div class="recurrence-byday">
                                        {WEEKDAYS.iter().map(|(code, label)| {
                                            let code_str = code.to_string();
                                            let code_str2 = code_str.clone();
                                            let state_c = state3.clone();
                                            let state_click = state3.clone();
                                            view! {
                                                <button
                                                    type="button"
                                                    class=move || {
                                                        let active = state_c.byday.get()
                                                            .contains(&code_str2.clone());
                                                        if active { "byday-btn byday-active" } else { "byday-btn" }
                                                    }
                                                    on:click=move |_| {
                                                        let code = code_str.clone();
                                                        state_click.byday.update(|v| {
                                                            if let Some(pos) = v.iter().position(|x| x == &code) {
                                                                v.remove(pos);
                                                            } else {
                                                                v.push(code);
                                                            }
                                                        });
                                                    }
                                                >{*label}</button>
                                            }
                                        }).collect_view()}
                                    </div>
                                }
                            })
                        }

                        // ── End condition ─────────────────────────────────────
                        <div class="recurrence-ends">
                            <label class="form-label-inline">"Ends"</label>
                            <div class="recurrence-end-options">
                                <label class="recurrence-end-option">
                                    <input type="radio" name="recurrence_end"
                                        prop:checked=move || state2.end_type.get()=="never"
                                        on:change=move |_| state2.end_type.set("never".into()) />
                                    " Never"
                                </label>
                                <label class="recurrence-end-option">
                                    <input type="radio" name="recurrence_end"
                                        prop:checked=move || state2.end_type.get()=="count"
                                        on:change=move |_| state2.end_type.set("count".into()) />
                                    " After "
                                    <input class="form-input recurrence-count" type="number" min="1"
                                        prop:value=move || state2.count.get()
                                        on:input=move |ev| {
                                            state2.end_type.set("count".into());
                                            state2.count.set(event_target_value(&ev));
                                        } />
                                    " occurrences"
                                </label>
                                <label class="recurrence-end-option">
                                    <input type="radio" name="recurrence_end"
                                        prop:checked=move || state2.end_type.get()=="until"
                                        on:change=move |_| state2.end_type.set("until".into()) />
                                    " On "
                                    <input class="form-input recurrence-until" type="date"
                                        prop:value=move || state2.until.get()
                                        on:input=move |ev| {
                                            state2.end_type.set("until".into());
                                            state2.until.set(event_target_value(&ev));
                                        } />
                                </label>
                            </div>
                        </div>
                    </div>
                }
            })}
        </div>
    }
}

#[cfg(test)]
mod rrule_helper_tests {
    use super::{build_rrule_string, parse_rrule_parts, RruleParts};

    fn rt(s: &str) {
        let parts = parse_rrule_parts(s);
        let rebuilt = build_rrule_string(&parts).unwrap_or_default();
        let parts2 = parse_rrule_parts(&rebuilt);
        assert_eq!(
            parts, parts2,
            "round-trip failed for {:?}: rebuilt as {:?}",
            s, rebuilt
        );
    }

    #[test]
    fn daily_no_end() {
        rt("FREQ=DAILY");
        let parts = parse_rrule_parts("FREQ=DAILY");
        assert_eq!(parts.freq, "daily");
        assert_eq!(parts.interval, "1");
        assert_eq!(parts.end_type, "never");
        assert_eq!(build_rrule_string(&parts), Some("FREQ=DAILY".to_string()));
    }

    #[test]
    fn daily_with_interval() {
        rt("FREQ=DAILY;INTERVAL=2");
        let parts = parse_rrule_parts("FREQ=DAILY;INTERVAL=2");
        assert_eq!(parts.interval, "2");
        assert_eq!(
            build_rrule_string(&parts),
            Some("FREQ=DAILY;INTERVAL=2".to_string())
        );
    }

    #[test]
    fn weekly_with_byday() {
        rt("FREQ=WEEKLY;BYDAY=MO,WE,FR");
        let parts = parse_rrule_parts("FREQ=WEEKLY;BYDAY=MO,WE,FR");
        assert_eq!(parts.freq, "weekly");
        assert_eq!(parts.byday, vec!["MO", "WE", "FR"]);
        let result = build_rrule_string(&parts).unwrap();
        assert!(result.contains("BYDAY=MO,WE,FR"), "{}", result);
    }

    #[test]
    fn monthly_with_count() {
        rt("FREQ=MONTHLY;COUNT=10");
        let parts = parse_rrule_parts("FREQ=MONTHLY;COUNT=10");
        assert_eq!(parts.end_type, "count");
        assert_eq!(parts.count, "10");
        assert_eq!(
            build_rrule_string(&parts),
            Some("FREQ=MONTHLY;COUNT=10".to_string())
        );
    }

    #[test]
    fn yearly_with_until() {
        rt("FREQ=YEARLY;UNTIL=20261231");
        let parts = parse_rrule_parts("FREQ=YEARLY;UNTIL=20261231");
        assert_eq!(parts.end_type, "until");
        assert_eq!(parts.until, "2026-12-31");
        let result = build_rrule_string(&parts).unwrap();
        assert!(result.contains("UNTIL=20261231"), "{}", result);
    }

    #[test]
    fn until_strips_time_component() {
        let parts = parse_rrule_parts("FREQ=WEEKLY;UNTIL=20261231T000000Z");
        assert_eq!(parts.until, "2026-12-31");
    }

    #[test]
    fn none_freq_returns_none() {
        let parts = RruleParts::default(); // freq = "none"
        assert_eq!(build_rrule_string(&parts), None);
    }

    #[test]
    fn byday_not_emitted_for_monthly() {
        // BYDAY on a monthly rule (set via some external source) is dropped in output.
        let parts = RruleParts {
            freq: "monthly".to_string(),
            interval: "1".to_string(),
            byday: vec!["MO".to_string()], // UI only shows byday for weekly
            end_type: "never".to_string(),
            count: "10".to_string(),
            until: String::new(),
        };
        let result = build_rrule_string(&parts).unwrap();
        assert!(
            !result.contains("BYDAY"),
            "BYDAY must not appear for monthly: {}",
            result
        );
    }
}
