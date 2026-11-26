use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use http::{header, HeaderValue, Uri};
use http_body_util::BodyExt;
use wasmtime_wasi_http::{RequestOptions, WasiBody, WasiHttpHooks};

fn canonical_ip(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6
            .to_ipv4_mapped()
            .map(IpAddr::V4)
            .unwrap_or(IpAddr::V6(v6)),
        v4 @ IpAddr::V4(_) => v4,
    }
}

pub(super) fn is_prohibited_outbound_ip(ip: IpAddr) -> bool {
    match canonical_ip(ip) {
        IpAddr::V4(v4) => {
            let octets = v4.octets();
            let is_cgnat = octets[0] == 100 && (octets[1] & 0b1100_0000) == 64;
            let is_this_net = octets[0] == 0;
            let is_benchmark = octets[0] == 198 && matches!(octets[1], 18 | 19);
            let is_reserved = octets[0] >= 240;
            v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_unspecified()
                || v4.is_multicast()
                || is_cgnat
                || is_this_net
                || is_benchmark
                || is_reserved
        }
        IpAddr::V6(v6) => {
            let segs = v6.segments();
            let is_link_local = (segs[0] & 0xffc0) == 0xfe80;
            let is_unique_local = (segs[0] & 0xfe00) == 0xfc00;
            let is_documentation = segs[0] == 0x2001 && segs[1] == 0x0db8;
            let is_ipv4_compat =
                segs[..6] == [0, 0, 0, 0, 0, 0] && !v6.is_loopback() && !v6.is_unspecified();
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || is_link_local
                || is_unique_local
                || is_documentation
                || is_ipv4_compat
        }
    }
}

pub(super) fn is_blocked_socket_addr(
    addr: SocketAddr,
    blocked_upstream: Option<SocketAddr>,
    allow_private_outbound: bool,
) -> bool {
    if let Some(blocked) = blocked_upstream {
        if addr.port() == blocked.port() && canonical_ip(addr.ip()) == canonical_ip(blocked.ip()) {
            return true;
        }
    }
    !allow_private_outbound && is_prohibited_outbound_ip(addr.ip())
}

pub(super) struct FerroHttpHooks {
    pub(super) blocked_upstream: Option<SocketAddr>,
    pub(super) allow_private_outbound: bool,
    pub(super) outbound_http_used: Arc<AtomicU32>,
    pub(super) max_outbound_http: u32,
}

impl WasiHttpHooks for FerroHttpHooks {
    fn default_scheme(&mut self) -> Option<http::uri::Scheme> {
        Some(http::uri::Scheme::HTTP)
    }

    fn send_request(
        &mut self,
        mut request: http::Request<WasiBody>,
        options: Option<RequestOptions>,
        _fut: Box<
            dyn std::future::Future<Output = std::result::Result<(), wasmtime_wasi_http::Error>>
                + Send,
        >,
    ) -> Box<
        dyn std::future::Future<
                Output = std::result::Result<
                    (
                        http::Response<WasiBody>,
                        Box<
                            dyn std::future::Future<
                                    Output = std::result::Result<(), wasmtime_wasi_http::Error>,
                                > + Send,
                        >,
                    ),
                    wasmtime_wasi_http::Error,
                >,
            > + Send,
    > {
        let blocked_upstream = self.blocked_upstream;
        let allow_private_outbound = self.allow_private_outbound;
        let outbound_http_used = self.outbound_http_used.clone();
        let max_outbound_http = self.max_outbound_http;
        Box::new(async move {
            let used = outbound_http_used.fetch_add(1, Ordering::Relaxed);
            if used >= max_outbound_http {
                return Err(wasmtime_wasi_http::Error::HttpRequestDenied);
            }
            let Some(authority) = request.uri().authority().cloned() else {
                return Err(wasmtime_wasi_http::Error::HttpRequestUriInvalid);
            };
            let is_https = request.uri().scheme() == Some(&http::uri::Scheme::HTTPS);
            let port = authority
                .port_u16()
                .unwrap_or(if is_https { 443 } else { 80 });
            let raw_host = authority.host();
            let host = raw_host
                .strip_prefix('[')
                .and_then(|s| s.strip_suffix(']'))
                .unwrap_or(raw_host);

            if let Ok(ip) = host.parse::<IpAddr>() {
                if is_blocked_socket_addr(
                    SocketAddr::new(ip, port),
                    blocked_upstream,
                    allow_private_outbound,
                ) {
                    return Err(wasmtime_wasi_http::Error::DestinationIpProhibited);
                }
            } else {
                let host_lower = host.trim_end_matches('.').to_ascii_lowercase();
                if !allow_private_outbound
                    && (host_lower == "localhost"
                        || host_lower.ends_with(".localhost")
                        || host_lower.ends_with(".internal"))
                {
                    return Err(wasmtime_wasi_http::Error::DestinationIpProhibited);
                }
                let resolved: Vec<SocketAddr> = tokio::net::lookup_host((host, port))
                    .await
                    .map_err(|_| wasmtime_wasi_http::Error::DestinationNotFound)?
                    .collect();
                if resolved.is_empty() {
                    return Err(wasmtime_wasi_http::Error::DestinationNotFound);
                }
                if resolved.iter().any(|addr| {
                    is_blocked_socket_addr(*addr, blocked_upstream, allow_private_outbound)
                }) {
                    return Err(wasmtime_wasi_http::Error::DestinationIpProhibited);
                }
                // For plaintext HTTP, pin the validated resolved address to prevent DNS rebinding
                // while preserving the original Host header for virtual hosting.
                if !is_https {
                    if !request.headers().contains_key(header::HOST) {
                        if let Ok(hv) = HeaderValue::from_str(authority.as_str()) {
                            request.headers_mut().insert(header::HOST, hv);
                        }
                    }
                    let pinned = resolved
                        .iter()
                        .find(|addr| addr.is_ipv4())
                        .copied()
                        .unwrap_or(resolved[0]);
                    if let Ok(pinned_auth) = pinned.to_string().parse::<http::uri::Authority>() {
                        let mut parts = request.uri().clone().into_parts();
                        parts.authority = Some(pinned_auth);
                        if let Ok(pinned_uri) = Uri::from_parts(parts) {
                            *request.uri_mut() = pinned_uri;
                        }
                    }
                }
            }

            let (res, io) = wasmtime_wasi_http::default_send_request(request, options).await?;
            Ok((
                res.map(BodyExt::boxed_unsync),
                Box::new(io)
                    as Box<
                        dyn std::future::Future<
                                Output = std::result::Result<(), wasmtime_wasi_http::Error>,
                            > + Send,
                    >,
            ))
        })
    }
}
