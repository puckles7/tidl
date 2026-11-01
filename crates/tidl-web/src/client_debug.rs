//! Client-side debug logging — gate behind the `client-debug` Cargo feature.
//!
//! Call [`install()`] once in the WASM entry point to wire up:
//! - A custom panic hook that POSTs the panic message to `/debug-log`
//! - A `window.onerror` handler that POSTs unhandled JS errors to `/debug-log`
//! - A `console.error` override that forwards calls to `/debug-log` (captures
//!   tachys hydration mismatch details logged just before a panic)
//!
//! A "wasm-init" ping is sent at install time so you can verify the feature
//! is active even when nothing goes wrong.

use js_sys::Array;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsValue;
use web_sys::{Blob, BlobPropertyBag, Navigator};

/// Install the panic hook, `window.onerror` handler, and `console.error`
/// override, then fire a startup ping.
pub fn install() {
    install_panic_hook();
    install_onerror_hook();
    install_console_error_hook();
    // Startup ping — confirms WASM + client-debug are both active.
    send_log("wasm-init", "hydrate() reached client_debug::install()");
}

/// POST `{"level":…,"message":…,"ts":…}` JSON to `/debug-log`.
///
/// Uses `navigator.sendBeacon` with a JSON `Blob` — no `eval()` required, so
/// this works even under a strict Content-Security-Policy.
/// A recursion guard prevents re-entrant calls (e.g. a panic inside this fn).
pub fn send_log(level: &str, message: &str) {
    use std::cell::Cell;
    thread_local! {
        static SENDING: Cell<bool> = Cell::new(false);
    }
    SENDING.with(|guard| {
        if guard.get() {
            return;
        }
        guard.set(true);
        do_send(level, message);
        guard.set(false);
    });
}

fn do_send(level: &str, message: &str) {
    let ts = js_sys::Date::now() as u64;

    let lv_json = serde_json::to_string(level).unwrap_or_else(|_| "\"error\"".into());
    let msg_json = serde_json::to_string(message).unwrap_or_else(|_| "\"error\"".into());
    let body = format!(
        "{{\"level\":{lv},\"message\":{msg},\"ts\":{ts}}}",
        lv = lv_json,
        msg = msg_json,
        ts = ts
    );

    let Some(window) = web_sys::window() else {
        return;
    };

    // Build a Blob from the JSON string.
    let parts = Array::of1(&JsValue::from_str(&body));
    let opts = BlobPropertyBag::new();
    opts.set_type("application/json");
    let Ok(blob) = Blob::new_with_str_sequence_and_options(&parts, &opts) else {
        return;
    };

    // sendBeacon is fire-and-forget and survives page unload.
    let nav: Navigator = window.navigator();
    let _ = nav.send_beacon_with_opt_blob("/debug-log", Some(&blob));
}

fn install_console_error_hook() {
    let Some(window) = web_sys::window() else {
        return;
    };
    let console = match js_sys::Reflect::get(window.as_ref(), &JsValue::from_str("console")) {
        Ok(v) => v,
        Err(_) => return,
    };

    // Save the original console.error so we can still call it.
    let original_error = match js_sys::Reflect::get(&console, &JsValue::from_str("error")) {
        Ok(v) => v,
        Err(_) => return,
    };

    let closure = Closure::<dyn Fn(JsValue)>::new(move |args: JsValue| {
        // Collect the argument (tachys passes a single string).
        let msg = args.as_string().unwrap_or_else(|| {
            js_sys::JSON::stringify(&args)
                .ok()
                .and_then(|s| s.as_string())
                .unwrap_or_else(|| "[non-string console.error]".into())
        });
        send_log("console-error", &msg);

        // Still call the real console.error so DevTools shows it too.
        let _ = js_sys::Reflect::apply(
            original_error.unchecked_ref::<js_sys::Function>(),
            &console,
            &Array::of1(&JsValue::from_str(&msg)),
        );
    });

    let win2 = web_sys::window().unwrap();
    let console2 = js_sys::Reflect::get(win2.as_ref(), &JsValue::from_str("console")).unwrap();
    let _ = js_sys::Reflect::set(&console2, &JsValue::from_str("error"), closure.as_ref());

    closure.forget();
}

fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let msg = info.to_string();
        leptos::logging::error!("[tidl] WASM panic: {}", msg);
        send_log("panic", &msg);
    }));
}

fn install_onerror_hook() {
    // window.onerror = function(message, source, lineno, colno, error) { … }
    let closure = Closure::<dyn Fn(JsValue, JsValue, JsValue, JsValue, JsValue)>::new(
        |msg: JsValue, src: JsValue, line: JsValue, col: JsValue, _err: JsValue| {
            let message = format!(
                "{} @ {}:{}:{}",
                msg.as_string().unwrap_or_else(|| "?".into()),
                src.as_string().unwrap_or_else(|| "?".into()),
                line.as_f64().unwrap_or(0.0) as u32,
                col.as_f64().unwrap_or(0.0) as u32,
            );
            leptos::logging::error!("[tidl] JS error: {}", message);
            send_log("js-error", &message);
        },
    );

    if let Some(win) = web_sys::window() {
        let _ = js_sys::Reflect::set(
            win.as_ref(),
            &JsValue::from_str("onerror"),
            closure.as_ref(),
        );
    }

    closure.forget();
}
