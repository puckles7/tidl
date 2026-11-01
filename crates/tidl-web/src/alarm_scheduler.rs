//! Browser-side alarm scheduler.
//!
//! Call `start_alarm_scheduler` once after hydration.  It:
//!   1. Requests browser Notification permission (if not already granted/denied).
//!   2. Creates a shared `AudioContext` (so it's created while the page is
//!      interactive, giving it the best chance of starting in "running" state).
//!   3. Sets up a `setInterval` every 30 seconds that checks the cached event
//!      list for due alarms.
//!
//! Call `update_events` whenever the events resource resolves so the cached list
//! stays current.  `check_alarms` is also called immediately on every update so
//! alarms fire without waiting up to 30 s.
//!
//! An alarm fires when:
//!   now ≥ event_start - trigger_offset_minutes
//!   AND now < event_start - trigger_offset_minutes + 1.5 minutes
//!
//! Fired alarms are tracked in a `HashSet<String>` keyed by
//! `"{event_uid}/{alarm_index}"` so they don't repeat within the session.
//!
//! This file is compiled only for WASM (hydrate feature).

#![cfg(target_arch = "wasm32")]

use std::cell::RefCell;
use std::collections::HashSet;

use wasm_bindgen::prelude::*;

use crate::api::Event;

// ── Notification permission ───────────────────────────────────────────────────

/// iOS Safari (outside an installed home-screen PWA) doesn't define the
/// `Notification` global at all, so any static call on it — even
/// `Notification::permission()` — throws a `ReferenceError` that wasm-bindgen
/// can't recover from and takes down the whole app. Feature-detect first.
fn notifications_supported() -> bool {
    js_sys::Reflect::has(&js_sys::global(), &JsValue::from_str("Notification")).unwrap_or(false)
}

/// True when the browser supports the Notification API but permission is
/// denied. Chromium-based browsers auto-deny `requestPermission()` (no
/// prompt shown) when the page is served over insecure HTTP from anything
/// but `localhost`/`127.0.0.1`, which silently disables alarm notifications —
/// this lets the UI surface that instead of failing invisibly.
pub fn notification_permission_denied() -> bool {
    if !notifications_supported() {
        return false;
    }
    use leptos::web_sys::{Notification, NotificationPermission};
    Notification::permission() == NotificationPermission::Denied
}

/// Request browser Notification permission if not already decided.
pub fn request_notification_permission() {
    if !notifications_supported() {
        return;
    }

    use leptos::web_sys::{Notification, NotificationPermission};

    if Notification::permission() == NotificationPermission::Default {
        if let Ok(promise) = Notification::request_permission() {
            leptos::task::spawn_local(async move {
                let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
            });
        }
    }
}

// ── Shared AudioContext ───────────────────────────────────────────────────────
//
// Created once in start_alarm_scheduler so it is instantiated while the page
// is interactive (highest chance of starting in "running" state). Re-using one
// context avoids the per-bell overhead of creating and GC'ing a new one.

thread_local! {
    static AUDIO_CTX: RefCell<Option<leptos::web_sys::AudioContext>> = RefCell::new(None);
}

fn init_audio_ctx() {
    use leptos::web_sys::AudioContext;
    AUDIO_CTX.with(|slot| {
        if slot.borrow().is_none() {
            if let Ok(ctx) = AudioContext::new() {
                *slot.borrow_mut() = Some(ctx);
            }
        }
    });
}

// ── Event cache ───────────────────────────────────────────────────────────────

thread_local! {
    static CURRENT_EVENTS: RefCell<Vec<Event>> = RefCell::new(Vec::new());
    static FIRED: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
}

/// Replace the cached event list.  Call this whenever the events resource
/// resolves so the 30-second interval always operates on fresh data.
pub fn update_events(events: Vec<Event>) {
    CURRENT_EVENTS.with(|e| *e.borrow_mut() = events);
}

// ── Scheduler ────────────────────────────────────────────────────────────────

/// Start the alarm scheduler.  Must be called exactly once after hydration.
pub fn start_alarm_scheduler() {
    leptos::logging::log!("[alarm] scheduler starting");
    request_notification_permission();
    init_audio_ctx();

    let tick = Closure::<dyn FnMut()>::new(|| {
        CURRENT_EVENTS.with(|e| {
            let events = e.borrow();
            leptos::logging::log!("[alarm] interval tick — {} events cached", events.len());
            check_alarms(&events);
        });
    });

    if let Some(win) = leptos::web_sys::window() {
        let _ = win.set_interval_with_callback_and_timeout_and_arguments_0(
            tick.as_ref().unchecked_ref(),
            30_000,
        );
    }
    tick.forget();
}

// ── Audio / visual output ─────────────────────────────────────────────────────

/// Play a short synthesised bell using the shared Web Audio context.
pub fn play_bell() {
    use leptos::web_sys::{AudioContextState, OscillatorType};

    AUDIO_CTX.with(|slot| {
        let borrow = slot.borrow();
        let Some(ctx) = borrow.as_ref() else { return };

        // Resume the context if it was suspended by the autoplay policy.
        // `resume()` returns a Promise; fire-and-forget is intentional — the
        // audio is scheduled slightly in the future so it plays after resume.
        if ctx.state() == AudioContextState::Suspended {
            if let Ok(promise) = ctx.resume() {
                leptos::task::spawn_local(async move {
                    let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
                });
            }
        }

        let Ok(osc) = ctx.create_oscillator() else {
            return;
        };
        let Ok(gain) = ctx.create_gain() else { return };
        let _ = osc.connect_with_audio_node(&gain);
        let _ = gain.connect_with_audio_node(&ctx.destination());

        osc.set_type(OscillatorType::Sine);
        let _ = osc.frequency().set_value(880.0); // A5

        // Schedule 150 ms in the future so the resumed context has time to start.
        let now = ctx.current_time();
        let t = now + 0.15;
        gain.gain().set_value_at_time(0.0, t).ok();
        gain.gain().linear_ramp_to_value_at_time(0.4, t + 0.01).ok();
        gain.gain()
            .exponential_ramp_to_value_at_time(0.001, t + 1.2)
            .ok();

        osc.start_with_when(t).ok();
        osc.stop_with_when(t + 1.2).ok();
    });
}

/// Show a browser Notification if permission has been granted.
pub fn show_notification(title: &str, body: &str) {
    if !notifications_supported() {
        return;
    }

    use leptos::web_sys::{Notification, NotificationPermission};
    let perm = Notification::permission();
    leptos::logging::log!("[alarm] notification permission: {:?}", perm);
    if perm == NotificationPermission::Granted {
        let opts = leptos::web_sys::NotificationOptions::new();
        opts.set_body(body);
        let result = Notification::new_with_options(title, &opts);
        leptos::logging::log!("[alarm] Notification::new result ok={}", result.is_ok());
    }
}

// ── Core alarm check ──────────────────────────────────────────────────────────

fn alarm_key(uid: &str, idx: usize) -> String {
    format!("{uid}/{idx}")
}

/// Check all alarms and fire any that are due now.
pub fn check_alarms(events: &[Event]) {
    let now_mins = {
        let d = js_sys::Date::new_0();
        d.get_time() / 60_000.0
    };

    let total_alarms: usize = events.iter().map(|e| e.alarms.len()).sum();
    if total_alarms > 0 {
        leptos::logging::log!(
            "[alarm] check_alarms: {} events, {} alarms total, now={:.2}",
            events.len(),
            total_alarms,
            now_mins
        );
    }

    FIRED.with(|fired| {
        let mut fired = fired.borrow_mut();
        for ev in events {
            // Prefer the server-supplied UTC epoch (immune to browser TZ spoofing).
            // Fall back to parsing ev.start as local time for old cached data.
            let start_mins = if ev.start_epoch_ms != 0 {
                ev.start_epoch_ms as f64 / 60_000.0
            } else if ev.start.len() >= 16 {
                let s = &ev.start;
                let year: u32 = s[..4].parse().unwrap_or(0);
                let month: i32 = s[5..7].parse::<i32>().unwrap_or(1) - 1;
                let day: i32 = s[8..10].parse().unwrap_or(1);
                let hour: i32 = s[11..13].parse().unwrap_or(0);
                let minute: i32 = s[14..16].parse().unwrap_or(0);
                let d = js_sys::Date::new_with_year_month_day_hr_min_sec(
                    year, month, day, hour, minute, 0,
                );
                let ms = d.get_time();
                if ms.is_nan() {
                    continue;
                }
                ms / 60_000.0
            } else {
                continue;
            };
            for (i, alarm) in ev.alarms.iter().enumerate() {
                if alarm.is_raw {
                    leptos::logging::log!("[alarm] skipping raw alarm for '{}'", ev.summary);
                    continue;
                }
                let offset_mins: f64 = {
                    let a = alarm.amount as f64;
                    match alarm.unit.as_str() {
                        "hours" => a * 60.0,
                        "days" => a * 24.0 * 60.0,
                        "weeks" => a * 7.0 * 24.0 * 60.0,
                        _ => a, // "minutes"
                    }
                };
                let trigger_mins = start_mins - offset_mins;
                let key = alarm_key(&ev.id, i);
                leptos::logging::log!(
                    "[alarm] '{}' alarm[{}]: trigger={:.2}, now={:.2}, delta={:.2}min",
                    ev.summary,
                    i,
                    trigger_mins,
                    now_mins,
                    now_mins - trigger_mins
                );
                if now_mins >= trigger_mins && now_mins < trigger_mins + 1.5 {
                    if fired.insert(key) {
                        leptos::logging::log!("[alarm] FIRING alarm for '{}'", ev.summary);
                        let body = format!("{}  ·  starts at {}", ev.summary, &ev.start[11..16]);
                        if alarm.plays_sound() {
                            play_bell();
                        }
                        show_notification(&ev.summary, &body);

                        // Handle REPEAT: schedule additional firings.
                        if let (Some(count), Some(delay_m)) =
                            (alarm.repeat_count, alarm.repeat_delay_m)
                        {
                            let summary = ev.summary.clone();
                            let plays_sound = alarm.plays_sound();
                            for r in 1..=count {
                                let delay_ms = (delay_m as f64 * 60_000.0 * r as f64) as i32;
                                let summary2 = summary.clone();
                                let closure = Closure::once(move || {
                                    let body = format!("{} (repeat {})", summary2, r);
                                    if plays_sound {
                                        play_bell();
                                    }
                                    show_notification(&summary2, &body);
                                });
                                if let Some(win) = leptos::web_sys::window() {
                                    let _ = win
                                        .set_timeout_with_callback_and_timeout_and_arguments_0(
                                            closure.as_ref().unchecked_ref(),
                                            delay_ms,
                                        );
                                    closure.forget();
                                }
                            }
                        }
                    }
                }
            }
        }
    });
}
