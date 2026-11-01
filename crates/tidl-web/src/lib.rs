//! tidl-web — Leptos 0.8 web UI
#![recursion_limit = "512"]

#[cfg(feature = "ssr")]
pub mod alarm_parse;
#[cfg(feature = "hydrate")]
pub mod alarm_scheduler;
pub mod api;
pub mod app;
#[cfg(feature = "client-debug")]
pub mod client_debug;
pub mod components;
pub mod grid;
pub mod local_editor;
pub mod markdown_tokens;
pub mod modal_esc;
pub mod note_tokens;
pub mod ssr_types;
pub mod types;
pub mod util;

pub use app::App;

/// CSR entry point — called automatically by wasm-bindgen when the module loads.
#[cfg(feature = "hydrate")]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() {
    use leptos::mount::mount_to_body;

    #[cfg(feature = "client-debug")]
    client_debug::install();
    #[cfg(not(feature = "client-debug"))]
    console_error_panic_hook::set_once();

    leptos::logging::log!("[tidl] WASM hydrate() called — JS/WASM is running");
    mount_to_body(App);
}
