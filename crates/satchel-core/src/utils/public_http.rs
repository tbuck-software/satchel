//! Public HTTPS requests to targets supplied by mail senders.

use reqwest::Url;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::time::Duration;

#[derive(Debug, thiserror::Error)]
#[error("public HTTPS request rejected")]
pub struct PublicHttpError;

/// Scheme, port and literal host of a one-click POST target.
///
/// Only https on its default port: plain http offers nothing to verify, and a
/// non-standard port is how a header points at a service that never expected
/// to hear from a browser.
pub fn is_public_https_target(url: &Url) -> Result<(), PublicHttpError> {
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() {
        return Err(PublicHttpError);
    }
    if url.port().is_some_and(|port| port != 443) {
        return Err(PublicHttpError);
    }
    is_public_host(url)
}

/// Rejects a host that already names something unreachable from the internet.
/// A domain name passes here and is checked again after resolution.
pub fn is_public_host(url: &Url) -> Result<(), PublicHttpError> {
    let Some(host) = url.host_str() else {
        return Err(PublicHttpError);
    };
    let literal = host.trim_matches(['[', ']']);
    if let Ok(addr) = literal.parse::<IpAddr>() {
        return ensure_globally_routable(addr);
    }
    let name = host.trim_end_matches('.').to_ascii_lowercase();
    if name == "localhost" || name.ends_with(".localhost") {
        return Err(PublicHttpError);
    }
    Ok(())
}

pub async fn resolves_to_public_addresses_only(url: &Url) -> Result<(), PublicHttpError> {
    public_addresses(url).await.map(|_| ())
}

async fn public_addresses(url: &Url) -> Result<Vec<std::net::SocketAddr>, PublicHttpError> {
    is_public_host(url)?;
    let host = url
        .host_str()
        .ok_or(PublicHttpError)?
        .trim_matches(['[', ']']);
    let addresses: Vec<_> = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::net::lookup_host((host, url.port_or_known_default().unwrap_or(443))),
    )
    .await
    .map_err(|_| PublicHttpError)?
    .map_err(|_| PublicHttpError)?
    .collect();
    if addresses.is_empty() {
        return Err(PublicHttpError);
    }
    for address in &addresses {
        ensure_globally_routable(address.ip())?;
    }
    Ok(addresses)
}

/// Pin the checked answers so DNS cannot rebind between validation and dialing.
pub async fn client(url: &Url, timeout: Duration) -> Result<reqwest::Client, PublicHttpError> {
    is_public_https_target(url)?;
    let addresses = public_addresses(url).await?;
    guarded_builder(timeout)
        .resolve_to_addrs(
            url.host_str()
                .ok_or(PublicHttpError)?
                .trim_matches(['[', ']']),
            &addresses,
        )
        .build()
        .map_err(|_| PublicHttpError)
}

fn guarded_builder(timeout: Duration) -> reqwest::ClientBuilder {
    super::http::client_builder()
        .https_only(true)
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(timeout)
}

pub async fn read_limited(
    mut response: reqwest::Response,
    limit: usize,
) -> Result<Vec<u8>, PublicHttpError> {
    if response
        .content_length()
        .is_some_and(|size| size > limit as u64)
    {
        return Err(PublicHttpError);
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| PublicHttpError)? {
        if chunk.len() > limit.saturating_sub(bytes.len()) {
            return Err(PublicHttpError);
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn ensure_globally_routable(addr: IpAddr) -> Result<(), PublicHttpError> {
    let routable = match addr {
        IpAddr::V4(ip) => is_globally_routable_v4(ip),
        IpAddr::V6(ip) => is_globally_routable_v6(ip),
    };
    if routable {
        Ok(())
    } else {
        Err(PublicHttpError)
    }
}

fn is_globally_routable_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_documentation()
        || ip.is_multicast()
        || ip.is_unspecified()
        || a == 0
        // Carrier-grade NAT, 100.64.0.0/10.
        || (a == 100 && (64..128).contains(&b))
        // IETF protocol assignments, 192.0.0.0/24.
        || (a == 192 && b == 0 && c == 0)
        // Benchmarking networks, 198.18.0.0/15.
        || (a == 198 && (b == 18 || b == 19))
        // Reserved, 240.0.0.0/4.
        || a >= 240)
}

fn is_globally_routable_v6(ip: Ipv6Addr) -> bool {
    // `::ffff:10.0.0.1` and the deprecated `::10.0.0.1` form have to answer to
    // the IPv4 rules; read as plain IPv6 they would look like ordinary global
    // addresses. The zero-prefix test also covers `::` and `::1`.
    if let Some(mapped) = ip.to_ipv4_mapped() {
        return is_globally_routable_v4(mapped);
    }
    let segments = ip.segments();
    if segments[..6].iter().all(|segment| *segment == 0) {
        return false;
    }
    !(segments[0] & 0xe000 != 0x2000
        || (segments[0] == 0x2001 && segments[1] < 0x0200)
        || ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_multicast()
        // Unique local addresses, fc00::/7.
        || segments[0] & 0xfe00 == 0xfc00
        // Link-local unicast, fe80::/10.
        || segments[0] & 0xffc0 == 0xfe80
        // Documentation, 2001:db8::/32.
        || (segments[0] == 0x2001 && segments[1] == 0x0db8))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn public_client_rejects_private_targets_and_non_https_before_dialing() {
        for target in [
            "http://example.com",
            "https://localhost",
            "https://10.0.0.1",
            "https://[::ffff:127.0.0.1]",
            "https://example.com:8443",
            "https://user:pass@example.com",
        ] {
            assert!(client(&Url::parse(target).unwrap(), Duration::from_secs(1))
                .await
                .is_err());
        }
    }
    #[test]
    fn mixed_dns_answers_are_not_public() {
        for ip in [
            "127.0.0.1",
            "192.168.0.1",
            "100.64.0.1",
            "198.18.0.1",
            "fd00::1",
            "::1",
        ] {
            assert!(
                ensure_globally_routable(ip.parse().unwrap()).is_err(),
                "{ip}"
            );
        }
        assert!(ensure_globally_routable("93.184.215.14".parse().unwrap()).is_ok());
    }
    #[tokio::test]
    async fn sender_requests_never_follow_redirects() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buffer = [0; 1024];
            assert!(stream.read(&mut buffer).await.unwrap() > 0);
            stream.write_all(b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/private\r\nContent-Length: 0\r\n\r\n").await.unwrap();
        });
        let response = guarded_builder(Duration::from_secs(1))
            .https_only(false)
            .build()
            .unwrap()
            .get(format!("http://{address}"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::FOUND);
        task.await.unwrap();
    }

    #[tokio::test]
    async fn response_limit_applies_to_chunked_bodies_and_declared_lengths() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        for headers in [
            "Content-Length: 9\r\n\r\n123456789",
            "Transfer-Encoding: chunked\r\n\r\n9\r\n123456789\r\n0\r\n\r\n",
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let task = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut buffer = [0; 1024];
                assert!(stream.read(&mut buffer).await.unwrap() > 0);
                stream
                    .write_all(format!("HTTP/1.1 200 OK\r\n{headers}").as_bytes())
                    .await
                    .unwrap();
            });
            let response = super::super::http::client()
                .get(format!("http://{address}"))
                .send()
                .await
                .unwrap();
            assert!(read_limited(response, 8).await.is_err());
            task.await.unwrap();
        }
    }
}
