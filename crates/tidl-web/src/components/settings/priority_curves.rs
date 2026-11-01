//! Priority Curves settings tab (backlog #68) — four CurveEditor instances,
//! one per curve kind.

use leptos::prelude::StoredValue;
use leptos::prelude::*;

use super::CurveEditor;
use crate::api::save_priority_curve;
use crate::app::PriorityCurvesCtx;
use crate::types::{CurvePoint, PriorityCurves};
use crate::util::format_server_fn_error;

#[component]
pub(super) fn PriorityCurvesPanel(on_close: impl Fn() + Send + Sync + 'static) -> impl IntoView {
    let on_close =
        StoredValue::new(std::sync::Arc::new(on_close) as std::sync::Arc<dyn Fn() + Send + Sync>);

    let curves_ctx = use_context::<PriorityCurvesCtx>();
    let current = curves_ctx.map(|c| c.0.get_untracked()).unwrap_or_default();

    let calendar_points = RwSignal::new(current.calendar_time_left.clone());
    let due_date_points = RwSignal::new(current.due_date_time_left.clone());
    let priority_points = RwSignal::new(current.source_priority.clone());
    let event_priority_points = RwSignal::new(current.calendar_event_priority.clone());

    let is_saving = RwSignal::new(false);
    let error = RwSignal::new(Option::<String>::None);

    let defaults = PriorityCurves::default();

    let on_save = move |_: leptos::ev::MouseEvent| {
        if is_saving.get_untracked() {
            return;
        }
        is_saving.set(true);
        error.set(None);
        let cal: Vec<CurvePoint> = calendar_points.get_untracked();
        let due: Vec<CurvePoint> = due_date_points.get_untracked();
        let pri: Vec<CurvePoint> = priority_points.get_untracked();
        let ev_pri: Vec<CurvePoint> = event_priority_points.get_untracked();
        let cb = on_close.get_value();
        leptos::task::spawn_local(async move {
            let saves = [
                save_priority_curve("calendar_time_left".to_string(), cal.clone()).await,
                save_priority_curve("due_date_time_left".to_string(), due.clone()).await,
                save_priority_curve("source_priority".to_string(), pri.clone()).await,
                save_priority_curve("calendar_event_priority".to_string(), ev_pri.clone()).await,
            ];
            if let Some(Err(e)) = saves.into_iter().find(|r| r.is_err()) {
                is_saving.set(false);
                error.set(Some(format_server_fn_error(&e.to_string())));
                return;
            }
            if let Some(ctx) = curves_ctx {
                ctx.0.set(PriorityCurves {
                    calendar_time_left: cal,
                    due_date_time_left: due,
                    source_priority: pri,
                    calendar_event_priority: ev_pri,
                });
            }
            is_saving.set(false);
            cb();
        });
    };

    view! {
        {move || error.get().map(|e| view! { <div class="form-error">{e}</div> })}

        <div class="form-group">
            <label class="form-label">"Calendar events (time until start)"</label>
            <CurveEditor
                points=calendar_points
                x_min=-24.0
                x_max=168.0
                x_ticks=vec![(-24.0, "-24h"), (0.0, "0h"), (24.0, "24h"), (72.0, "72h"), (168.0, "168h")]
                default_points=defaults.calendar_time_left.clone()
            />
        </div>

        <div class="form-group">
            <label class="form-label">"Due dates (days until due)"</label>
            <CurveEditor
                points=due_date_points
                x_min=-7.0
                x_max=14.0
                x_ticks=vec![(-7.0, "-7d"), (0.0, "0d"), (3.0, "3d"), (7.0, "7d"), (14.0, "14d")]
                default_points=defaults.due_date_time_left.clone()
            />
        </div>

        <div class="form-group">
            <label class="form-label">"Note & task priority"</label>
            <CurveEditor
                points=priority_points
                x_min=0.0
                x_max=100.0
                x_ticks=vec![(0.0, "0"), (25.0, "25"), (50.0, "50"), (75.0, "75"), (100.0, "100")]
                default_points=defaults.source_priority.clone()
            />
        </div>

        <div class="form-group">
            <label class="form-label">"Calendar event priority (urgency cap)"</label>
            <CurveEditor
                points=event_priority_points
                x_min=1.0
                x_max=100.0
                x_ticks=vec![(1.0, "1"), (25.0, "25"), (50.0, "50"), (75.0, "75"), (100.0, "100")]
                default_points=defaults.calendar_event_priority.clone()
            />
        </div>

        <div class="dialog-actions">
            <button class="btn-cancel" on:click=move |_| on_close.with_value(|f| f())>"Cancel"</button>
            <button
                class="btn-primary"
                on:click=on_save
                disabled=move || is_saving.get()
            >
                {move || if is_saving.get() { "Saving…" } else { "Save" }}
            </button>
        </div>
    }
}
