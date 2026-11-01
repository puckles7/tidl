//! Reusable drag-only SVG curve editor for the Priority Curves settings tab
//! (backlog #68). Purely a `points` editor — knows nothing about which
//! curve kind it's editing or how to persist it.

use leptos::prelude::*;

use crate::types::CurvePoint;

const SVG_W: f64 = 560.0;
const SVG_H: f64 = 220.0;

/// Maps a curve-space `(x, y)` value to SVG pixel coordinates. `y` is
/// inverted (SVG grows downward; a higher score draws higher up).
fn to_px(x: f64, y: f64, x_min: f64, x_max: f64) -> (f64, f64) {
    let px = (x - x_min) / (x_max - x_min).max(f64::EPSILON) * SVG_W;
    let py = SVG_H - (y / 100.0) * SVG_H;
    (px.clamp(0.0, SVG_W), py.clamp(0.0, SVG_H))
}

/// Inverse of `to_px` — SVG pixel coordinates back to curve-space `(x, y)`.
fn from_px(px: f64, py: f64, x_min: f64, x_max: f64) -> (f64, f64) {
    let x = x_min + (px / SVG_W) * (x_max - x_min);
    let y = (1.0 - py / SVG_H) * 100.0;
    (x, y.clamp(0.0, 100.0))
}

/// Reads `ev.current_target()`'s on-screen bounding rect and converts the
/// pointer event's client coordinates into local SVG pixel coordinates.
/// Takes the event's own `current_target` — the element the listener is
/// attached to — rather than a separate `NodeRef`, the same way
/// `notes/mod.rs`'s `pointer_down_geometry` reads `ev.current_target()`
/// directly instead of holding a ref to the dragged card. This avoids
/// needing a typed `NodeRef` over an SVG element, which nothing elsewhere
/// in this codebase does (only `NodeRef<leptos::html::Div>` is used
/// anywhere today — see `notes/mod.rs`, `calendar/week_grid.rs`,
/// `settings/mod.rs`). Returns `(0, 0)` outside wasm32 (SSR never handles
/// pointer events).
#[allow(unused_variables)]
fn event_to_local_px(ev: &leptos::web_sys::PointerEvent) -> (f64, f64) {
    #[cfg(target_arch = "wasm32")]
    {
        use leptos::wasm_bindgen::JsCast;
        if let Some(target) = ev.current_target() {
            let el: web_sys::Element = target.unchecked_into();
            let r = el.get_bounding_client_rect();
            let scale_x = SVG_W / r.width().max(1.0);
            let scale_y = SVG_H / r.height().max(1.0);
            return (
                (ev.client_x() as f64 - r.left()) * scale_x,
                (ev.client_y() as f64 - r.top()) * scale_y,
            );
        }
    }
    (0.0, 0.0)
}

#[component]
pub(super) fn CurveEditor(
    points: RwSignal<Vec<CurvePoint>>,
    x_min: f64,
    x_max: f64,
    x_ticks: Vec<(f64, &'static str)>,
    default_points: Vec<CurvePoint>,
) -> impl IntoView {
    let dragging_idx: RwSignal<Option<usize>> = RwSignal::new(None);

    let polyline_points = move || {
        points
            .get()
            .iter()
            .map(|p| {
                let (px, py) = to_px(p.x, p.y, x_min, x_max);
                format!("{px},{py}")
            })
            .collect::<Vec<_>>()
            .join(" ")
    };

    // Background bands for the URGENT(>=85)/SOON(>=65)/UPCOMING(>=40)/LATER
    // regions, so a drag's effect against the label thresholds is visible
    // while editing — same thresholds `urgency_label` classifies scores by.
    let band = move |y_lo: f64, y_hi: f64| {
        let (_, top) = to_px(0.0, y_hi, x_min, x_max);
        let (_, bottom) = to_px(0.0, y_lo, x_min, x_max);
        (top, bottom - top)
    };

    let on_point_pointerdown = move |idx: usize| {
        move |ev: leptos::web_sys::PointerEvent| {
            #[cfg(target_arch = "wasm32")]
            {
                use leptos::wasm_bindgen::JsCast;
                if let Some(target) = ev.current_target() {
                    let el: web_sys::Element = target.unchecked_into();
                    let _ = el.set_pointer_capture(ev.pointer_id());
                }
            }
            dragging_idx.set(Some(idx));
            ev.stop_propagation();
        }
    };

    let on_svg_pointermove = move |ev: leptos::web_sys::PointerEvent| {
        let Some(idx) = dragging_idx.get_untracked() else {
            return;
        };
        let (px, py) = event_to_local_px(&ev);
        let (new_x, new_y) = from_px(px, py, x_min, x_max);
        points.update(|pts| {
            if idx >= pts.len() {
                return;
            }
            pts[idx].x = new_x;
            pts[idx].y = new_y;
            // Keep the list sorted by x — re-sorting mid-drag lets a point
            // slide past its neighbor's x, matching the design's "dragging
            // a point past a neighbor's x re-sorts the point list" rule.
            // `dragging_idx` tracks the same point by identity across a
            // re-sort by finding its new position after sorting.
            pts.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal));
        });
        if let Some(new_idx) = points.with_untracked(|pts| {
            pts.iter().position(|p| {
                (p.x - new_x).abs() < f64::EPSILON && (p.y - new_y).abs() < f64::EPSILON
            })
        }) {
            dragging_idx.set(Some(new_idx));
        }
    };

    let on_svg_pointerup = move |_: leptos::web_sys::PointerEvent| {
        dragging_idx.set(None);
    };

    // Click empty chart space to add a point at that (x, y).
    let on_svg_click = move |ev: leptos::web_sys::MouseEvent| {
        if dragging_idx.get_untracked().is_some() {
            return;
        }
        #[cfg(target_arch = "wasm32")]
        {
            use leptos::wasm_bindgen::JsCast;
            if let Some(target) = ev.current_target() {
                let el: web_sys::Element = target.unchecked_into();
                let r = el.get_bounding_client_rect();
                let scale_x = SVG_W / r.width().max(1.0);
                let scale_y = SVG_H / r.height().max(1.0);
                let px = (ev.client_x() as f64 - r.left()) * scale_x;
                let py = (ev.client_y() as f64 - r.top()) * scale_y;
                let (x, y) = from_px(px, py, x_min, x_max);
                points.update(|pts| {
                    pts.push(CurvePoint { x, y });
                    pts.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal));
                });
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        let _ = ev;
    };

    let remove_point = move |idx: usize| {
        move |ev: leptos::web_sys::MouseEvent| {
            ev.stop_propagation();
            points.update(|pts| {
                // A curve needs at least 2 points — silently refuse to drop
                // below that instead of leaving the graph in a state
                // `eval_curve` can't handle.
                if pts.len() > 2 && idx < pts.len() {
                    pts.remove(idx);
                }
            });
        }
    };

    let on_reset = {
        let default_points = default_points.clone();
        move |_: leptos::ev::MouseEvent| {
            points.set(default_points.clone());
        }
    };

    let (urgent_top, urgent_h) = band(85.0, 100.0);
    let (soon_top, soon_h) = band(65.0, 85.0);
    let (upcoming_top, upcoming_h) = band(40.0, 65.0);
    let (later_top, later_h) = band(0.0, 40.0);

    view! {
        <div class="curve-editor">
            <svg
                class="curve-editor-svg"
                viewBox=format!("0 0 {SVG_W} {SVG_H}")
                on:pointermove=on_svg_pointermove
                on:pointerup=on_svg_pointerup
                on:pointercancel=on_svg_pointerup
                on:click=on_svg_click
            >
                <rect x="0" y=urgent_top width=SVG_W height=urgent_h class="curve-band curve-band--urgent"></rect>
                <rect x="0" y=soon_top width=SVG_W height=soon_h class="curve-band curve-band--soon"></rect>
                <rect x="0" y=upcoming_top width=SVG_W height=upcoming_h class="curve-band curve-band--upcoming"></rect>
                <rect x="0" y=later_top width=SVG_W height=later_h class="curve-band curve-band--later"></rect>

                // Thin vertical gridlines at each axis tick's true x position, so a
                // point's precise x-value can be read against the axis without
                // guessing — same positions the tick labels below are aligned to.
                {x_ticks.iter().map(|(val, _)| {
                    let (px, _) = to_px(*val, 0.0, x_min, x_max);
                    view! {
                        <line x1=px y1="0" x2=px y2=SVG_H class="curve-editor-gridline"></line>
                    }
                }).collect_view()}

                <polyline points=polyline_points class="curve-editor-line"></polyline>

                {move || points.get().into_iter().enumerate().map(|(idx, p)| {
                    let (px, py) = to_px(p.x, p.y, x_min, x_max);
                    view! {
                        <circle
                            cx=px
                            cy=py
                            r="6"
                            class="curve-editor-point"
                            on:pointerdown=on_point_pointerdown(idx)
                            on:click=|ev: leptos::web_sys::MouseEvent| ev.stop_propagation()
                            on:dblclick=remove_point(idx)
                        ></circle>
                    }
                }).collect_view()}
            </svg>
            <div class="curve-editor-axis">
                {
                    let last_idx = x_ticks.len().saturating_sub(1);
                    x_ticks.into_iter().enumerate().map(|(i, (val, label))| {
                        // Position each tick at its true proportional value along
                        // x_min..x_max — these are not evenly spaced numerically
                        // (e.g. -24h/0h/24h/72h/168h), so a flexbox space-between
                        // row would visually misalign every label except the two
                        // endpoints against the curve/points, which *are* plotted
                        // on this same true linear scale via `to_px`.
                        let pct = (val - x_min) / (x_max - x_min).max(f64::EPSILON) * 100.0;
                        let translate = if i == 0 {
                            "0%"
                        } else if i == last_idx {
                            "-100%"
                        } else {
                            "-50%"
                        };
                        view! {
                            <span
                                class="curve-editor-tick"
                                style=format!("left:{pct}%; transform: translateX({translate});")
                            >{label}</span>
                        }
                    }).collect_view()
                }
            </div>
            <button type="button" class="btn-secondary-small curve-editor-reset" on:click=on_reset>
                "Reset to default"
            </button>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_px_and_from_px_round_trip() {
        let (px, py) = to_px(50.0, 75.0, 0.0, 100.0);
        let (x, y) = from_px(px, py, 0.0, 100.0);
        assert!((x - 50.0).abs() < 0.01);
        assert!((y - 75.0).abs() < 0.01);
    }

    #[test]
    fn to_px_maps_y_100_to_top_and_y_0_to_bottom() {
        let (_, top_py) = to_px(0.0, 100.0, 0.0, 10.0);
        let (_, bottom_py) = to_px(0.0, 0.0, 0.0, 10.0);
        assert_eq!(top_py, 0.0);
        assert_eq!(bottom_py, SVG_H);
    }

    #[test]
    fn to_px_clamps_out_of_range_x() {
        let (px_below, _) = to_px(-100.0, 50.0, 0.0, 10.0);
        let (px_above, _) = to_px(100.0, 50.0, 0.0, 10.0);
        assert_eq!(px_below, 0.0);
        assert_eq!(px_above, SVG_W);
    }
}
