//! Shared 1-100 priority slider (1 = highest) — the canonical task-priority
//! scale used by notes' task/await fields and Focus's "New task"
//! dialog alike (backlog #70 / 2026-07-30 investigation). Extracted into one
//! component so the inverted-domain math (the control's native value runs
//! low-to-high left-to-right as usual, but the stored priority runs the
//! opposite way) is written once rather than risking a second, subtly
//! different copy — which is exactly the class of bug this was pulled out
//! of.

use leptos::prelude::*;

use crate::app::PriorityCurvesCtx;
use crate::components::focus::{eval_curve, urgency_color, urgency_label};
use crate::types::{note_priority_urgency_color, note_priority_urgency_label, PriorityCurves};

/// What the slider's live preview should describe.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum PrioritySliderMode {
    /// The priority's own task urgency (the default everywhere else).
    #[default]
    Task,
    /// The maximum urgency a calendar event may reach (backlog #156) — its
    /// `calendar_event_priority` attenuation ceiling, not its task score.
    Event,
}

#[component]
pub fn PrioritySlider(
    /// 1-100, 1 = highest.
    priority: RwSignal<u32>,
    #[prop(optional, into)] disabled: Signal<bool>,
    #[prop(optional)] mode: PrioritySliderMode,
) -> impl IntoView {
    let curves = StoredValue::new(
        use_context::<PriorityCurvesCtx>()
            .map(|c| c.0.get_untracked())
            .unwrap_or_default(),
    );

    let score = move |p: u32| {
        curves.with_value(|c: &PriorityCurves| match mode {
            PrioritySliderMode::Task => note_priority_urgency_label(p, c),
            PrioritySliderMode::Event => {
                urgency_label(eval_curve(&c.calendar_event_priority, p as f64))
            }
        })
    };
    let color = move |p: u32| {
        curves.with_value(|c: &PriorityCurves| match mode {
            PrioritySliderMode::Task => note_priority_urgency_color(p, c),
            PrioritySliderMode::Event => {
                urgency_color(eval_curve(&c.calendar_event_priority, p as f64))
            }
        })
    };

    view! {
        <div class="form-group">
            <label class="form-label">
                {move || match mode {
                    PrioritySliderMode::Task => format!(
                        "Priority: {} · {}",
                        score(priority.get()),
                        priority.get(),
                    ),
                    PrioritySliderMode::Event => format!(
                        "Priority: max {} · {}",
                        score(priority.get()),
                        priority.get(),
                    ),
                }}
            </label>
            <input
                type="range"
                min="1"
                max="100"
                class="note-priority-slider"
                style=move || format!("accent-color:{};", color(priority.get()))
                // right = high priority (value 1), left = low priority
                // (value 100) — see this module's doc comment.
                prop:value=move || (101 - priority.get()).to_string()
                on:input=move |ev| {
                    if let Ok(v) = event_target_value(&ev).parse::<u32>() {
                        priority.set(101 - v);
                    }
                }
                disabled=move || disabled.get()
            />
        </div>
    }
}
