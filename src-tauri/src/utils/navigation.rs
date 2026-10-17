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

fn route_navigation(url: &Url, origin: &Url) -> bool {
    navigate(url, origin, open_external)
}

fn open_external(url: &Url) {
    // This command validates schemes before handing the URL to the OS browser.
    let _ = crate::commands::external_links::open_external_url(url.to_string());
}

fn navigate(url: &Url, origin: &Url, open: impl FnOnce(&Url)) -> bool {
    if allowed_navigation(url, origin) {
        return true;
    }
    open(url);
    false
}

#[cfg(desktop)]
fn popup(url: &Url, origin: &Url, open: impl FnOnce(&Url)) {
    // App popouts are created by commands with their own capabilities.
    if !allowed_navigation(url, origin) {
        open(url);
    }
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
    let popup_origin = origin.clone();
    let builder = builder.on_navigation(move |url| route_navigation(url, &origin));
    #[cfg(desktop)]
    let builder = builder.on_new_window(move |url, _| {
        popup(&url, &popup_origin, open_external);
        tauri::webview::NewWindowResponse::Deny
    });
    #[cfg(mobile)]
    let _ = popup_origin;
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
    fn external_navigation_and_popups_are_handed_to_the_browser() {
        let origin = Url::parse("tauri://localhost").unwrap();
        let external = Url::parse("https://example.com/mail").unwrap();
        let mut handed_off = Vec::new();
        assert!(!navigate(&external, &origin, |url| handed_off.push(url.clone())));
        assert_eq!(handed_off, vec![external.clone()]);
        assert!(navigate(&origin, &origin, |_| panic!(
            "app navigation opened a browser"
        )));
        #[cfg(desktop)]
        {
            popup(&external, &origin, |url| handed_off.push(url.clone()));
            popup(&origin, &origin, |_| panic!("app popup opened a browser"));
            assert_eq!(handed_off, vec![external.clone(), external]);
        }
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
