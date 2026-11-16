//! All app windows share the same top-level navigation and popup policy.

use reqwest::Url;
use tauri::{Manager, WebviewWindowBuilder};

fn same_origin(candidate: &Url, app_origin: &Url) -> bool {
    candidate.scheme() == app_origin.scheme()
        && candidate.host_str() == app_origin.host_str()
        && candidate.port_or_known_default() == app_origin.port_or_known_default()
        && candidate.username().is_empty()
        && candidate.password().is_none()
}

fn allowed_navigation(url: &Url, origin: &Url) -> bool {
    // WebKit also reports the inherited documents of sandboxed mail iframes.
    matches!(url.as_str(), "about:blank" | "about:srcdoc") || same_origin(url, origin)
}

fn app_origin(app: &tauri::AppHandle) -> Url {
    if tauri::is_dev() {
        if let Some(url) = &app.config().build.dev_url {
            return url.clone();
        }
    }
    #[cfg(any(target_os = "windows", target_os = "android"))]
    let origin = "http://tauri.localhost";
    #[cfg(not(any(target_os = "windows", target_os = "android")))]
    let origin = "tauri://localhost";
    Url::parse(origin).expect("app origin")
}

/// Anything outside the app is refused, not handed to the browser: external
/// links go through `open_external_url`, behind the link confirmation.
fn route_navigation(url: &Url, origin: &Url) -> bool {
    allowed_navigation(url, origin)
}

pub fn init() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri::plugin::Builder::new("navigation-boundary")
        .on_navigation(|webview, url| route_navigation(url, &app_origin(webview.app_handle())))
        .build()
}

pub fn guard<'a, M: Manager<tauri::Wry>>(
    builder: WebviewWindowBuilder<'a, tauri::Wry, M>,
    app: &tauri::AppHandle,
) -> WebviewWindowBuilder<'a, tauri::Wry, M> {
    let origin = app_origin(app);
    // Tauri's drag-drop handler answers every drag itself and never passes it
    // on to the webview: on macOS wry overrides `draggingEntered:`,
    // `draggingUpdated:` and `performDragOperation:` of the WKWebView and
    // skips `super` whenever the handler returns true, which Tauri's always
    // does. The page then never sees `dragover` or `drop`, for files from the
    // Finder as much as for a mail dragged onto the composer. Nothing here
    // listens to Tauri's drag-drop events; the pages use HTML5 drag and drop.
    let builder = builder
        .disable_drag_drop_handler()
        .on_navigation(move |url| route_navigation(url, &origin));
    // App popouts are created by commands with their own capabilities.
    #[cfg(desktop)]
    let builder = builder.on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny);
    builder
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_the_exact_app_origin_can_load() {
        for inherited in ["about:blank", "about:srcdoc"] {
            assert!(allowed_navigation(
                &Url::parse(inherited).unwrap(),
                &Url::parse("tauri://localhost").unwrap()
            ));
        }
        for origin in [
            "tauri://localhost",
            "http://tauri.localhost",
            "http://localhost:5173",
        ] {
            let origin = Url::parse(origin).unwrap();
            assert!(same_origin(
                &origin.join("/viewer.html?p=abc").unwrap(),
                &origin
            ));
            for rejected in [
                "https://example.com",
                "https://localhost:5173",
                "http://localhost:5174",
                "http://tauri.localhost.evil.test",
                "tauri://other",
                "file:///tmp/mail.html",
                "data:text/html,test",
                "javascript:alert(1)",
                "https://user:pass@localhost:5173",
            ] {
                assert!(
                    !same_origin(&Url::parse(rejected).unwrap(), &origin),
                    "{rejected}"
                );
            }
        }
    }
    #[test]
    fn external_navigation_is_refused_without_opening_anything() {
        let origin = Url::parse("tauri://localhost").unwrap();
        for external in [
            "https://example.com/mail",
            "http://example.com/",
            "mailto:someone@example.com",
        ] {
            assert!(!route_navigation(&Url::parse(external).unwrap(), &origin));
        }
        assert!(route_navigation(
            &origin.join("/index.html").unwrap(),
            &origin
        ));
        let source = include_str!("navigation.rs");
        let production = source.split("#[cfg(test)]").next().unwrap();
        assert!(
            !production.contains(concat!("open_external", "_url(")),
            "navigation must not hand URLs to the browser"
        );
    }
    #[test]
    fn the_guard_hands_drags_to_the_page() {
        let source = include_str!("navigation.rs");
        let guard = source
            .split("pub fn guard<")
            .nth(1)
            .and_then(|rest| rest.split("#[cfg(test)]").next())
            .unwrap();
        assert!(guard.contains(".disable_drag_drop_handler()"));
    }

    #[test]
    fn every_window_builder_uses_the_shared_guard() {
        for source in [
            include_str!("../lib.rs"),
            include_str!("../commands/message_file.rs"),
            include_str!("../commands/notifications.rs"),
            include_str!("../commands/tray.rs"),
        ] {
            let source = source.split("#[cfg(test)]").next().unwrap();
            let builders = source.matches("WebviewWindowBuilder::new(").count()
                + source.matches("WebviewWindowBuilder::from_config(").count();
            assert!(builders > 0);
            assert_eq!(
                builders,
                source.matches("utils::navigation::guard(").count()
            );
        }
    }
}
