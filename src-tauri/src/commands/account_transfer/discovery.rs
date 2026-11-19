//! Discovery is a convenience. A literal address on the invitation also
//! works on networks that filter multicast.
use super::Nearby;

pub(super) fn local_address(port: u16) -> Option<String> {
    // UDP connect selects a route without sending a packet.
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("192.0.2.1:9").ok()?;
    Some(std::net::SocketAddr::new(socket.local_addr().ok()?.ip(), port).to_string())
}

#[cfg(desktop)]
pub(super) mod platform {
    use super::Nearby;
    use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
    use std::collections::BTreeMap;
    const SERVICE: &str = "_satchel-setup._tcp.local.";
    pub(crate) struct Discovery {
        daemon: ServiceDaemon,
        offer: Option<String>,
        events: Option<mdns_sd::Receiver<ServiceEvent>>,
        peers: BTreeMap<String, Nearby>,
    }
    impl Discovery {
        pub(crate) async fn start(
            _: &tauri::AppHandle,
            id: &str,
            port: Option<u16>,
        ) -> Option<Self> {
            let daemon = ServiceDaemon::new().ok()?;
            let mut d = Self {
                daemon,
                offer: None,
                events: None,
                peers: BTreeMap::new(),
            };
            if let Some(port) = port {
                let name: String = whoami::devicename()
                    .chars()
                    .filter(|c| !c.is_control())
                    .take(50)
                    .collect();
                let info = ServiceInfo::new(
                    SERVICE,
                    id,
                    &format!("satchel-{id}.local."),
                    "",
                    port,
                    &[("name", name.as_str())][..],
                )
                .ok()?
                .enable_addr_auto();
                d.offer = Some(info.get_fullname().to_owned());
                d.daemon.register(info).ok()?;
            } else {
                d.events = Some(d.daemon.browse(SERVICE).ok()?);
            }
            Some(d)
        }
        pub(crate) async fn peers(&mut self) -> Vec<Nearby> {
            if let Some(events) = &self.events {
                while let Ok(event) = events.try_recv() {
                    match event {
                        ServiceEvent::ServiceResolved(info) if self.peers.len() < 64 => {
                            if let (Some(ip), Some(name)) = (
                                info.get_addresses_v4().into_iter().min(),
                                info.get_property_val_str("name"),
                            ) {
                                self.peers.insert(
                                    info.get_fullname().into(),
                                    Nearby {
                                        name: name
                                            .chars()
                                            .filter(|c| !c.is_control())
                                            .take(60)
                                            .collect(),
                                        address: format!("{ip}:{}", info.get_port()),
                                    },
                                );
                            }
                        }
                        ServiceEvent::ServiceRemoved(_, name) => {
                            self.peers.remove(&name);
                        }
                        _ => {}
                    }
                }
            }
            self.peers.values().cloned().collect()
        }
    }
    impl Drop for Discovery {
        fn drop(&mut self) {
            if let Some(offer) = &self.offer {
                let _ = self.daemon.unregister(offer);
            }
            let _ = self.daemon.shutdown();
        }
    }
}

#[cfg(target_os = "android")]
pub(super) mod platform {
    use super::Nearby;
    use tauri::Manager;
    pub(crate) struct Discovery {
        app: tauri::AppHandle,
        id: String,
    }
    impl Discovery {
        async fn call(&self, op: &str, port: Option<u16>) -> Option<Vec<Nearby>> {
            #[derive(serde::Deserialize)]
            struct Result {
                peers: Vec<Nearby>,
            }
            self.app
                .state::<crate::commands::background_sync::AndroidBackgroundSync<tauri::Wry>>()
                .0
                .run_mobile_plugin_async::<Result>(
                    "accountDiscovery",
                    serde_json::json!({"id":self.id,"op":op,"port":port}),
                )
                .await
                .ok()
                .map(|response| response.peers)
        }
        pub(crate) async fn start(
            app: &tauri::AppHandle,
            id: &str,
            port: Option<u16>,
        ) -> Option<Self> {
            let d = Self {
                app: app.clone(),
                id: id.to_owned(),
            };
            d.call("start", port).await?;
            Some(d)
        }
        pub(crate) async fn peers(&mut self) -> Vec<Nearby> {
            self.call("status", None).await.unwrap_or_default()
        }
    }
    impl Drop for Discovery {
        fn drop(&mut self) {
            let app = self.app.clone();
            let id = self.id.clone();
            tauri::async_runtime::spawn(async move {
                let _ = app
                    .state::<crate::commands::background_sync::AndroidBackgroundSync<tauri::Wry>>()
                    .0
                    .run_mobile_plugin_async::<serde_json::Value>(
                        "accountDiscovery",
                        serde_json::json!({"id":id,"op":"stop"}),
                    )
                    .await;
            });
        }
    }
}
#[cfg(not(any(desktop, target_os = "android")))]
pub(super) mod platform {
    use super::Nearby;
    pub(crate) struct Discovery;
    impl Discovery {
        pub(crate) async fn start(_: &tauri::AppHandle, _: &str, _: Option<u16>) -> Option<Self> {
            None
        }
        pub(crate) async fn peers(&mut self) -> Vec<Nearby> {
            Vec::new()
        }
    }
}
