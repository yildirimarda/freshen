//! Network reachability with proxy and custom certificate support.
//!
//! The preflight check performs a TLS handshake with a well-known host. When a
//! proxy is configured through the environment, it connects to the proxy and
//! opens a tunnel with `CONNECT` first; `NO_PROXY` bypasses the proxy. Custom
//! certificate bundles are added to the platform trust store so a TLS
//! inspection proxy or a private CA does not make the check fail.
//!
//! Everything here is deliberately environment-driven: the same variables are
//! inherited by the commands every update step runs.

use std::fmt;
use std::fs;
use std::io::{self, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, ServerName};

/// Maximum size of a proxy `CONNECT` response head.
const MAX_PROXY_HEADER_BYTES: usize = 8 * 1024;

/// One host the preflight check tries to reach.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Endpoint {
    /// Name used for TLS validation and for the proxy `CONNECT` request.
    pub(crate) host: &'static str,
    /// Port used for TLS validation and for the proxy `CONNECT` request.
    pub(crate) port: u16,
    /// Address used when connecting directly, so the check works without DNS.
    pub(crate) address: SocketAddr,
}

/// The well-known hosts tried by the preflight check.
///
/// The IP address avoids a DNS lookup on the direct path; the host name gives
/// the proxy a name to resolve and gives TLS a stable identity to validate.
pub(crate) fn default_endpoints() -> [Endpoint; 2] {
    [
        Endpoint {
            host: "one.one.one.one",
            port: 443,
            address: SocketAddr::from(([1, 1, 1, 1], 443)),
        },
        Endpoint {
            host: "dns.google",
            port: 443,
            address: SocketAddr::from(([8, 8, 8, 8], 443)),
        },
    ]
}

/// Proxy and certificate settings read from the environment.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Environment {
    http_proxy: Option<String>,
    https_proxy: Option<String>,
    no_proxy: Option<String>,
    ca_bundles: Vec<PathBuf>,
}

impl Environment {
    /// Reads the settings from the process environment.
    pub(crate) fn from_process() -> Self {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    /// Reads the settings from an injected lookup, used by tests.
    fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Self {
        Self {
            http_proxy: first_value(&lookup, &["HTTP_PROXY", "http_proxy"]),
            https_proxy: first_value(&lookup, &["HTTPS_PROXY", "https_proxy"]),
            no_proxy: first_value(&lookup, &["NO_PROXY", "no_proxy"]),
            ca_bundles: ca_bundle_paths(&lookup),
        }
    }

    /// The proxy to use for `endpoint`, or `None` for a direct connection.
    fn proxy_for(&self, endpoint: &Endpoint) -> Result<Option<Proxy>, String> {
        if self.bypasses(endpoint) {
            return Ok(None);
        }

        let raw = self.https_proxy.as_deref().or(self.http_proxy.as_deref());
        raw.map(parse_proxy).transpose()
    }

    /// Whether `NO_PROXY` exempts `endpoint` from the proxy.
    fn bypasses(&self, endpoint: &Endpoint) -> bool {
        let Some(no_proxy) = self.no_proxy.as_deref() else {
            return false;
        };
        let address = endpoint.address.ip().to_string();
        no_proxy_matches(no_proxy, endpoint.host, endpoint.port)
            || no_proxy_matches(no_proxy, &address, endpoint.port)
    }
}

/// Why the network preflight check failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NetworkCheckError {
    /// A proxy variable could not be parsed.
    InvalidProxy(String),
    /// The system trust store or a custom CA bundle could not be loaded.
    TrustStore(String),
    /// The TLS handshake with the endpoint failed.
    TlsHandshake { host: String, error: String },
    /// The proxy refused to open a tunnel to the endpoint.
    ProxyTunnel { endpoint: String, detail: String },
    /// No endpoint could be reached at all.
    Unreachable,
}

impl fmt::Display for NetworkCheckError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidProxy(detail) => write!(f, "invalid proxy URL: {detail}"),
            Self::TrustStore(detail) => f.write_str(detail),
            Self::TlsHandshake { host, error } => {
                write!(f, "TLS handshake with {host} failed: {error}")
            }
            Self::ProxyTunnel { endpoint, detail } => {
                write!(f, "proxy could not open a tunnel to {endpoint}: {detail}")
            }
            Self::Unreachable => f.write_str("network is unreachable"),
        }
    }
}

fn first_value(lookup: &impl Fn(&str) -> Option<String>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        lookup(key)
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
    })
}

fn ca_bundle_paths(lookup: &impl Fn(&str) -> Option<String>) -> Vec<PathBuf> {
    const KEYS: &[&str] = &[
        "SSL_CERT_FILE",
        "SSL_CERT_DIR",
        "CURL_CA_BUNDLE",
        "REQUESTS_CA_BUNDLE",
        "NIX_SSL_CERT_FILE",
        "NODE_EXTRA_CA_CERTS",
    ];

    let mut paths = Vec::new();
    for key in KEYS {
        let Some(value) = lookup(key) else {
            continue;
        };
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        let path = PathBuf::from(value);
        if !paths.contains(&path) {
            paths.push(path);
        }
    }
    paths
}

/// A parsed proxy URL.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Proxy {
    secure: bool,
    host: String,
    port: u16,
    authorization: Option<String>,
}

fn parse_proxy(raw: &str) -> Result<Proxy, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("empty proxy URL".to_owned());
    }

    let (scheme, rest) = match raw.split_once("://") {
        Some((scheme, rest)) => (scheme.to_ascii_lowercase(), rest),
        None => ("http".to_owned(), raw),
    };
    let secure = match scheme.as_str() {
        "http" => false,
        "https" => true,
        other => return Err(format!("unsupported proxy scheme '{other}'")),
    };

    let rest = rest.trim_end_matches('/');
    let (userinfo, host_port) = match rest.rsplit_once('@') {
        Some((userinfo, host_port)) => (Some(userinfo), host_port),
        None => (None, rest),
    };
    let (host, port) = parse_host_port(host_port, if secure { 443 } else { 80 })?;
    let authorization = userinfo
        .map(|userinfo| base64::engine::general_purpose::STANDARD.encode(userinfo.as_bytes()));

    Ok(Proxy {
        secure,
        host,
        port,
        authorization,
    })
}

fn parse_host_port(value: &str, default_port: u16) -> Result<(String, u16), String> {
    let value = value.trim();
    if value.is_empty() {
        return Err("proxy URL has no host".to_owned());
    }

    if let Some(rest) = value.strip_prefix('[') {
        let Some((host, after)) = rest.split_once(']') else {
            return Err("proxy URL has an unterminated IPv6 address".to_owned());
        };
        let port = match after.strip_prefix(':') {
            Some(port) => parse_port(port)?,
            None if after.is_empty() => default_port,
            None => return Err("proxy URL has an invalid port".to_owned()),
        };
        return Ok((host.to_owned(), port));
    }

    if let Some((host, port)) = value.rsplit_once(':') {
        if !host.contains(':') {
            return Ok((host.to_owned(), parse_port(port)?));
        }
    }

    Ok((value.to_owned(), default_port))
}

fn parse_port(value: &str) -> Result<u16, String> {
    value
        .parse::<u16>()
        .map_err(|_| format!("proxy URL has an invalid port '{value}'"))
}

/// Returns `true` when `no_proxy` exempts `host:port` from proxying.
fn no_proxy_matches(no_proxy: &str, host: &str, port: u16) -> bool {
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();

    no_proxy
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|entry| !entry.is_empty())
        .any(|entry| no_proxy_entry_matches(entry.trim(), &host, port))
}

fn no_proxy_entry_matches(entry: &str, host: &str, port: u16) -> bool {
    if entry == "*" {
        return true;
    }

    let entry = entry
        .split_once("://")
        .map_or(entry, |(_, rest)| rest)
        .trim_end_matches('/');
    let (pattern, pattern_port) = split_no_proxy_port(entry);
    if let Some(pattern_port) = pattern_port {
        if pattern_port != port {
            return false;
        }
    }

    let pattern = pattern.trim_end_matches('.').to_ascii_lowercase();
    if pattern.is_empty() {
        return false;
    }

    if pattern.contains('/') {
        return cidr_matches(&pattern, host);
    }

    if let Some(suffix) = pattern.strip_prefix("*.") {
        return host.len() > suffix.len()
            && host.ends_with(suffix)
            && host.as_bytes()[host.len() - suffix.len() - 1] == b'.';
    }

    let pattern = pattern.strip_prefix('.').unwrap_or(&pattern);
    host == pattern || host.ends_with(&format!(".{pattern}"))
}

/// Returns `true` when `host` is inside the CIDR `pattern`, such as
/// `10.0.0.0/8` or `fd00::/8`. Hostnames never match a CIDR range.
fn cidr_matches(pattern: &str, host: &str) -> bool {
    let Some((network, prefix)) = pattern.split_once('/') else {
        return false;
    };
    let (Ok(network), Ok(host), Ok(prefix)) = (
        network.parse::<IpAddr>(),
        host.parse::<IpAddr>(),
        prefix.parse::<u8>(),
    ) else {
        return false;
    };

    match (network, host) {
        (IpAddr::V4(network), IpAddr::V4(host)) => {
            prefix <= 32 && prefix_matches(&network.octets(), &host.octets(), prefix)
        }
        (IpAddr::V6(network), IpAddr::V6(host)) => {
            prefix <= 128 && prefix_matches(&network.octets(), &host.octets(), prefix)
        }
        _ => false,
    }
}

fn prefix_matches(network: &[u8], host: &[u8], prefix: u8) -> bool {
    let full_bytes = prefix as usize / 8;
    let remaining_bits = prefix % 8;

    if network[..full_bytes] != host[..full_bytes] {
        return false;
    }
    if remaining_bits == 0 {
        return true;
    }

    let mask = 0xffu8 << (8 - remaining_bits);
    network[full_bytes] & mask == host[full_bytes] & mask
}

fn split_no_proxy_port(entry: &str) -> (&str, Option<u16>) {
    if let Some(rest) = entry.strip_prefix('[') {
        if let Some((host, after)) = rest.split_once(']') {
            let port = after.strip_prefix(':').and_then(|port| port.parse().ok());
            return (host, port);
        }
        return (entry, None);
    }

    if let Some((host, port)) = entry.rsplit_once(':') {
        if !host.contains(':') {
            if let Ok(port) = port.parse::<u16>() {
                return (host, Some(port));
            }
        }
    }

    (entry, None)
}

/// Checks whether any of the default endpoints is reachable.
pub(crate) fn check() -> Result<(), NetworkCheckError> {
    let environment = Environment::from_process();
    check_with(&environment, &default_endpoints(), Duration::from_secs(2))
}

fn check_with(
    environment: &Environment,
    endpoints: &[Endpoint],
    timeout: Duration,
) -> Result<(), NetworkCheckError> {
    for endpoint in endpoints {
        if let Err(error) = environment.proxy_for(endpoint) {
            return Err(NetworkCheckError::InvalidProxy(error));
        }
    }

    let roots = root_store(environment).map_err(NetworkCheckError::TrustStore)?;
    let config = Arc::new(
        ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth(),
    );

    let mut failure = None;
    for endpoint in endpoints {
        match endpoint_reachable(environment, endpoint, timeout, &config) {
            Ok(()) => return Ok(()),
            Err(error) => {
                let replace = matches!(
                    failure.as_ref(),
                    None | Some(NetworkCheckError::Unreachable)
                );
                if replace {
                    failure = Some(error);
                }
            }
        }
    }
    Err(failure.unwrap_or(NetworkCheckError::Unreachable))
}

fn root_store(environment: &Environment) -> Result<RootCertStore, String> {
    let mut roots = RootCertStore::empty();

    // The native store can fail partially: some certificates are loaded and
    // others are not. Only a store that yielded nothing usable is an error,
    // and only when no custom bundle can take over.
    let native = rustls_native_certs::load_native_certs();
    roots.add_parsable_certificates(native.certs);
    if roots.is_empty() && !native.errors.is_empty() && environment.ca_bundles.is_empty() {
        let details = native
            .errors
            .iter()
            .map(|error| error.to_string())
            .collect::<Vec<_>>()
            .join("; ");
        return Err(format!("cannot load the system trust store: {details}"));
    }

    for path in &environment.ca_bundles {
        add_bundle(&mut roots, path)?;
    }

    if roots.is_empty() {
        return Err("no trusted certificates are available".to_owned());
    }
    Ok(roots)
}

fn add_bundle(roots: &mut RootCertStore, path: &Path) -> Result<(), String> {
    if path.is_dir() {
        let entries = fs::read_dir(path)
            .map_err(|error| format!("cannot read CA directory {}: {error}", path.display()))?;
        let mut added = false;
        for entry in entries.flatten() {
            let file = entry.path();
            if !file.is_file() {
                continue;
            }
            if let Ok(certificates) = certificates_from_file(&file) {
                if add_certificates(roots, certificates, &file).is_ok() {
                    added = true;
                }
            }
        }
        if !added {
            return Err(format!(
                "no certificates found in CA directory {}",
                path.display()
            ));
        }
        return Ok(());
    }

    let certificates = certificates_from_file(path)?;
    if certificates.is_empty() {
        return Err(format!("no certificates found in {}", path.display()));
    }
    add_certificates(roots, certificates, path)
}

fn add_certificates(
    roots: &mut RootCertStore,
    certificates: Vec<CertificateDer<'static>>,
    path: &Path,
) -> Result<(), String> {
    for certificate in certificates {
        roots.add(certificate).map_err(|error| {
            format!(
                "cannot parse certificate in CA bundle {}: {error}",
                path.display()
            )
        })?;
    }
    Ok(())
}

fn certificates_from_file(path: &Path) -> Result<Vec<CertificateDer<'static>>, String> {
    let data = fs::read(path)
        .map_err(|error| format!("cannot read CA bundle {}: {error}", path.display()))?;
    let mut certificates = Vec::new();
    for certificate in CertificateDer::pem_slice_iter(&data) {
        certificates.push(
            certificate
                .map_err(|error| format!("cannot parse CA bundle {}: {error}", path.display()))?,
        );
    }

    if certificates.is_empty() && !data.is_empty() {
        certificates.push(CertificateDer::from(data));
    }
    Ok(certificates)
}

fn endpoint_reachable(
    environment: &Environment,
    endpoint: &Endpoint,
    timeout: Duration,
    config: &Arc<ClientConfig>,
) -> Result<(), NetworkCheckError> {
    match environment.proxy_for(endpoint) {
        Ok(Some(proxy)) => proxy_reachable(&proxy, endpoint, timeout, config),
        Ok(None) => {
            let mut stream =
                connect(endpoint.address, timeout).ok_or(NetworkCheckError::Unreachable)?;
            tls_handshake(&mut stream, endpoint.host, config)
        }
        Err(error) => Err(NetworkCheckError::InvalidProxy(error)),
    }
}

fn proxy_reachable(
    proxy: &Proxy,
    endpoint: &Endpoint,
    timeout: Duration,
    config: &Arc<ClientConfig>,
) -> Result<(), NetworkCheckError> {
    let Ok(addresses) = (proxy.host.as_str(), proxy.port).to_socket_addrs() else {
        return Err(NetworkCheckError::Unreachable);
    };

    let mut failure = None;
    for address in addresses {
        let Some(mut stream) = connect(address, timeout) else {
            if failure.is_none() {
                failure = Some(NetworkCheckError::Unreachable);
            }
            continue;
        };

        let result = if proxy.secure {
            match tls_stream(stream, &proxy.host, config) {
                Ok(mut tunnel) => {
                    connect_tunnel(&mut tunnel, endpoint, proxy.authorization.as_deref())
                        .and_then(|()| tls_handshake(&mut tunnel, endpoint.host, config))
                }
                Err(error) => Err(error),
            }
        } else {
            connect_tunnel(&mut stream, endpoint, proxy.authorization.as_deref())
                .and_then(|()| tls_handshake(&mut stream, endpoint.host, config))
        };

        match result {
            Ok(()) => return Ok(()),
            Err(error) => {
                let replace = matches!(
                    failure.as_ref(),
                    None | Some(NetworkCheckError::Unreachable)
                );
                if replace {
                    failure = Some(error);
                }
            }
        }
    }

    Err(failure.unwrap_or(NetworkCheckError::Unreachable))
}

fn connect(address: SocketAddr, timeout: Duration) -> Option<TcpStream> {
    let stream = TcpStream::connect_timeout(&address, timeout).ok()?;
    stream.set_read_timeout(Some(timeout)).ok()?;
    stream.set_write_timeout(Some(timeout)).ok()?;
    Some(stream)
}

fn tls_stream(
    mut stream: TcpStream,
    host: &str,
    config: &Arc<ClientConfig>,
) -> Result<StreamOwned<ClientConnection, TcpStream>, NetworkCheckError> {
    let name =
        ServerName::try_from(host.to_owned()).map_err(|error| NetworkCheckError::TlsHandshake {
            host: host.to_owned(),
            error: error.to_string(),
        })?;
    let mut connection = ClientConnection::new(config.clone(), name).map_err(|error| {
        NetworkCheckError::TlsHandshake {
            host: host.to_owned(),
            error: error.to_string(),
        }
    })?;
    connection
        .complete_io(&mut stream)
        .map_err(|error| NetworkCheckError::TlsHandshake {
            host: host.to_owned(),
            error: error.to_string(),
        })?;
    Ok(StreamOwned::new(connection, stream))
}

fn tls_handshake<S>(
    stream: &mut S,
    host: &str,
    config: &Arc<ClientConfig>,
) -> Result<(), NetworkCheckError>
where
    S: Read + Write,
{
    let name =
        ServerName::try_from(host.to_owned()).map_err(|error| NetworkCheckError::TlsHandshake {
            host: host.to_owned(),
            error: error.to_string(),
        })?;
    let mut connection = ClientConnection::new(config.clone(), name).map_err(|error| {
        NetworkCheckError::TlsHandshake {
            host: host.to_owned(),
            error: error.to_string(),
        }
    })?;
    connection
        .complete_io(stream)
        .map_err(|error| NetworkCheckError::TlsHandshake {
            host: host.to_owned(),
            error: error.to_string(),
        })
        .map(|_| ())
}

fn connect_tunnel<S>(
    stream: &mut S,
    endpoint: &Endpoint,
    authorization: Option<&str>,
) -> Result<(), NetworkCheckError>
where
    S: Read + Write,
{
    let authority = if endpoint.host.contains(':') {
        format!("[{}]:{}", endpoint.host, endpoint.port)
    } else {
        format!("{}:{}", endpoint.host, endpoint.port)
    };

    let mut request = format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n");
    if let Some(authorization) = authorization {
        request.push_str(&format!("Proxy-Authorization: Basic {authorization}\r\n"));
    }
    request.push_str("\r\n");

    if stream.write_all(request.as_bytes()).is_err() || stream.flush().is_err() {
        return Err(NetworkCheckError::ProxyTunnel {
            endpoint: authority,
            detail: "cannot send the CONNECT request".to_owned(),
        });
    }

    read_connect_response(stream).map_err(|detail| NetworkCheckError::ProxyTunnel {
        endpoint: authority,
        detail,
    })
}

fn read_connect_response<S>(stream: &mut S) -> Result<(), String>
where
    S: Read,
{
    let mut response = Vec::new();
    let mut chunk = [0_u8; 512];

    loop {
        if response.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
        if response.len() >= MAX_PROXY_HEADER_BYTES {
            return Err("the CONNECT response head is too large".to_owned());
        }
        match stream.read(&mut chunk) {
            Ok(0) => return Err("the proxy closed the connection before replying".to_owned()),
            Ok(read) => response.extend_from_slice(&chunk[..read]),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => return Err("cannot read the CONNECT response".to_owned()),
        }
    }

    let Some(line_end) = response.windows(2).position(|window| window == b"\r\n") else {
        return Err("the CONNECT response has no status line".to_owned());
    };
    let Ok(status_line) = std::str::from_utf8(&response[..line_end]) else {
        return Err("the CONNECT status line is not UTF-8".to_owned());
    };
    let mut parts = status_line.split_whitespace();
    let Some(_version) = parts.next() else {
        return Err("the CONNECT response has no status line".to_owned());
    };
    let Some(code) = parts.next().and_then(|code| code.parse::<u16>().ok()) else {
        return Err("the CONNECT response has no status code".to_owned());
    };
    if (200..300).contains(&code) {
        Ok(())
    } else {
        Err(format!("the proxy replied with HTTP status {code}"))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        certificates_from_file, check_with, no_proxy_matches, parse_proxy, root_store, Endpoint,
        Environment, NetworkCheckError,
    };
    use base64::Engine as _;
    use rustls::{ServerConfig, ServerConnection, StreamOwned};
    use rustls_pki_types::pem::PemObject;
    use rustls_pki_types::{CertificateDer, PrivateKeyDer};
    use std::io::{self, Read, Write};
    use std::net::{SocketAddr, TcpListener, TcpStream};
    use std::sync::Arc;
    use std::thread;
    use std::time::{Duration, Instant};

    /// The common name of the test CA certificate.
    const CA_SUBJECT: &[u8] = b"freshen test CA";

    const CA_CERTIFICATE: &str = r#"-----BEGIN CERTIFICATE-----
MIIDJzCCAg+gAwIBAgIUZAVNl6ELS3/Rrd9j7sMH6uJGNFQwDQYJKoZIhvcNAQEL
BQAwGjEYMBYGA1UEAwwPZnJlc2hlbiB0ZXN0IENBMCAXDTI2MTAyMTEwMDQyNVoY
DzIxMjYwOTI3MTAwNDI1WjAaMRgwFgYDVQQDDA9mcmVzaGVuIHRlc3QgQ0EwggEi
MA0GCSqGSIb3DQEBAQUAA4IBDwAwggEKAoIBAQC8YC4YEouLb0vZ3NzvGgP83/Ee
vYVtJqelurpMXjYghpCt8XQfW8+PTTnO2ILkDXOdCV4i1LmYpgTuMFAq1f8+KHZZ
cugmZcWAkVi5ygIkI4DTQTSZi5aCTST3NPIlCr+kVdmgfgU14mBHHVtf/PxUGh5H
2VFK2n4VAyJJOzZI4Wk2zg4pUR+1mtWEZCJwsq1HTX7h6ubzPZJ+jeBliv2fAd/9
n3JoEblHDmhyC6+HM1S1b3w+LENHLk/aMNyoEUrs3aiJ4RftXPJRjsZ45aa1LYSA
C7e8XwoXDSAKSmsSFRWNY09qc1Qw29JBlU8YjCO41dabxVrJLRoV8t756H1pAgMB
AAGjYzBhMB0GA1UdDgQWBBR7RL4JtTy91OB4MbAtboNEovxE6DAfBgNVHSMEGDAW
gBR7RL4JtTy91OB4MbAtboNEovxE6DAPBgNVHRMBAf8EBTADAQH/MA4GA1UdDwEB
/wQEAwIBBjANBgkqhkiG9w0BAQsFAAOCAQEAaIEnKa3oteLWuHpjbLCYnV1OuB27
OdqFxut1b89tjfJ2yhjD4rRIckbgcKthRRin0WTdMF07TggdEks9sD+EA7O2DsPv
034OSrbE0+nkQo7n9oHqzd8334DaqA31baIwjKcMSC/pcjwoqYu3vbjIFAyCRKv6
m6RxgGbRL7o5Rnoc/SYJCCdbwtWmm8W2C7SnHlWZUHwzh/CM37fF6bjTRTe5WST0
EC9G8Q9HzBYq2eWjmfLv8OP0o9XQSeNyHihEKKGRr/YeutQ10vqGrvR/742R2IRU
n9sTIrFzyIrAI3vs2onsZuWdyCYz3NoyHxv03v6JY9EOmkS3+Fz9ZGbdew==
-----END CERTIFICATE-----
"#;

    const SERVER_CERTIFICATE: &str = r#"-----BEGIN CERTIFICATE-----
MIIDTjCCAjagAwIBAgIUGYZ6oVm84wm0Ig4WFw87vnVGxDwwDQYJKoZIhvcNAQEL
BQAwGjEYMBYGA1UEAwwPZnJlc2hlbiB0ZXN0IENBMCAXDTI2MTAyMTEwMDQyNVoY
DzIxMjYwOTI3MTAwNDI1WjAUMRIwEAYDVQQDDAlsb2NhbGhvc3QwggEiMA0GCSqG
SIb3DQEBAQUAA4IBDwAwggEKAoIBAQCanZcXW3SQ3my3E4MXqLHmTCQkLaNRxHIS
NSjJ2PhfPfn9RO2bOy9yAR6wkDQHP591t4rVf7nLUk1sBnkdFhdWSSi5IqxELcz7
D7C6Gk1v55WiqDdh4uOLPS4vTu0fR+u3PntbdyQ0tioLCH6nxuhUTObKM0Wh6FHL
NkmR3pF3vvcPTdeerCZLfccBZckIJPIqLCslgKpHIRv4pDam5aU/76aokcYiPpYc
pmV1KZjLWSmCI7oy9GCOH7tCjemcRqR99bEh0kp0FXBIBEydYBnRRaiD9sWPm84F
qgMSvh2qKqa8xws4NxVQXA85qqFzbdPyLGJ3pI277a/1aG7vAFHXAgMBAAGjgY8w
gYwwGgYDVR0RBBMwEYIJbG9jYWxob3N0hwR/AAABMAkGA1UdEwQCMAAwDgYDVR0P
AQH/BAQDAgWgMBMGA1UdJQQMMAoGCCsGAQUFBwMBMB0GA1UdDgQWBBQ6E+M08E/c
dmG+PHMmjNRhN4OTVDAfBgNVHSMEGDAWgBR7RL4JtTy91OB4MbAtboNEovxE6DAN
BgkqhkiG9w0BAQsFAAOCAQEAEGV2U9hYSVbS2Y6v/hwUPsS38JeCjpXX6K1adz0C
RkzJCrfgbP/CQ8MYWtA77poimTqeZeWSWXqWhXZLSGp8nLYH7B3HQuUP6Onw9Iyh
JrK/t0NO+H7n0mjikON6rkDIALedKUZGjevV8Cd5/lmxos+IdDUKenRsPMAtAFkU
ridd0CAcozXIrjdTeH49uosXI7JimKNhK55zebyspQtCTdTWFP7zNvpQk2uoZJuo
B6+yT/FkMX1rSz/HsLnAiHY4Nq/dExGgaq3zCD00xTkpcgQCipuW4F5RHgzcKLXT
Aksir+urSNmgILyN6DkfCm3y/Ar9tWV6vAgHra1gd0dMJQ==
-----END CERTIFICATE-----
"#;

    const SERVER_KEY: &str = r#"-----BEGIN PRIVATE KEY-----
MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQCanZcXW3SQ3my3
E4MXqLHmTCQkLaNRxHISNSjJ2PhfPfn9RO2bOy9yAR6wkDQHP591t4rVf7nLUk1s
BnkdFhdWSSi5IqxELcz7D7C6Gk1v55WiqDdh4uOLPS4vTu0fR+u3PntbdyQ0tioL
CH6nxuhUTObKM0Wh6FHLNkmR3pF3vvcPTdeerCZLfccBZckIJPIqLCslgKpHIRv4
pDam5aU/76aokcYiPpYcpmV1KZjLWSmCI7oy9GCOH7tCjemcRqR99bEh0kp0FXBI
BEydYBnRRaiD9sWPm84FqgMSvh2qKqa8xws4NxVQXA85qqFzbdPyLGJ3pI277a/1
aG7vAFHXAgMBAAECggEAIOzWM/8hv4feFs5s6CjgWW51c87ULKFsuTvgx7sQiRel
mK2I+7mKzPg4FNlgevqtdl/zjKt6YTXRjuOB/xOpPJKvvV0I2iv2rrAUrkzTmLT0
RIMV6B5s3kXVA/YGwNaL96WylzOz3hW8tkTE0+2jo/AMZZqthWD3qhKRcCK5T4pi
PfKqUGLnjK887keOcY9XdnaOs1zvkAvu2bAvyAUr4LxvZbRpeDvO8y68KWS+GEp2
jjNReeYFpRLRN5aZVz571kcp9ktgAIaVGvK8KWBISYvS/sOAOW0aW0pT8SgUbJEy
TdO7U7WcHwGNJRcib6BSGDUe9YLEB0Dw0Q89kFOioQKBgQDO1gJZcAD41vKN5vyX
z+rpe09CfmWOXPR2UKji27+ofatrVrTJOTRT7DfQ+21BbMZc8o8qg4BmqrlJgIS+
QxE+bltyHUlldVmj3SLTWadNuotD7yZ/Uy0/BqlbAx4hCk8X1JLhQfIUY3pSUBFZ
gCHaMjmIFWiIKvMEfgeaB8j/qwKBgQC/XfcJBbQ0I8gqsbKy/y7J/TBQJcFR04Lh
kpMJY5nYjP3dNztvSaZD9/oTMGtF4zjwD7q4rcZPPPsOlhanK0evfmouoPFDSCIN
GtVPXcLkakxlahB+pKvUU6m5F9el9WGh2vWOIIP2DgQS7lznRFLqHI9ygxL6TZrE
9q4egTR6hQKBgQCXrnAIoM/FrPQxxkxFFLAES26jp8fYMCkCv0IrI9YYvRhB3lM0
IiRSyy9frZLA4QFTVnpRRiCaqea5lWtDkDGMJCghtoKFyhpTxi7s6Zl8sinBRxdn
VlsfFkaKPO4Sctnv8u9IDDF4bD9fL+KZTRMGTcDNIE4JVd3j1HzEo6LugwKBgBNG
WqB9A5aQ2BN8zHv8+fB8YQZhlyQvwqoHwmmar59BLDzA2GqwJ2AHNRgDs3pZlZ2Q
aAU785J9mOfyNBvoXxMND5LcwRA+zrc1MbEJx5yMHF/GnBy1Z10+hATRovXjTk8o
J72GBuScqsphWdgtaJCOCeDF1P6DHIikNAt8hhWlAoGAR3VvB7yVzObliGm8YQR/
eDIiqkOLvVb+saApzRfwH6VGiQnN3LdeJyHSv6eQRxmrr9A0vSLLp6fV6GTfV2Xc
5OM9+muI7rcNKM765wU/ya5Uks00/LepbwI3XpnGior8f3UQFSNuy6W/ahHjmiVc
pT8yRnSiD29WLvz412hxX/k=
-----END PRIVATE KEY-----
"#;

    fn environment(pairs: &[(&str, &str)]) -> Environment {
        Environment::from_lookup(|key| {
            pairs
                .iter()
                .find(|(name, _)| *name == key)
                .map(|(_, value)| (*value).to_owned())
        })
    }

    fn endpoint(host: &'static str, address: SocketAddr) -> Endpoint {
        Endpoint {
            host,
            port: address.port(),
            address,
        }
    }

    fn check_failure(
        environment: &Environment,
        endpoints: &[Endpoint],
        timeout: Duration,
    ) -> NetworkCheckError {
        match check_with(environment, endpoints, timeout) {
            Ok(()) => panic!("the network check should have failed"),
            Err(error) => error,
        }
    }

    fn tls_server() -> (SocketAddr, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .unwrap_or_else(|error| panic!("failed to bind TLS test server: {error}"));
        let address = listener
            .local_addr()
            .unwrap_or_else(|error| panic!("failed to read TLS test server address: {error}"));
        listener
            .set_nonblocking(true)
            .unwrap_or_else(|error| panic!("failed to make TLS test server non-blocking: {error}"));

        let mut certificates = Vec::new();
        for certificate in CertificateDer::pem_slice_iter(SERVER_CERTIFICATE.as_bytes()) {
            certificates.push(
                certificate
                    .unwrap_or_else(|error| panic!("failed to parse test certificate: {error}")),
            );
        }
        let key = PrivateKeyDer::from_pem_slice(SERVER_KEY.as_bytes())
            .unwrap_or_else(|error| panic!("failed to parse test key: {error}"));
        let config = Arc::new(
            ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(certificates, key)
                .unwrap_or_else(|error| panic!("failed to build test TLS server: {error}")),
        );

        let handle = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(3);
            loop {
                match listener.accept() {
                    Ok((stream, _)) => {
                        stream.set_nonblocking(false).unwrap_or_else(|error| {
                            panic!("failed to block test connection: {error}")
                        });
                        let connection = ServerConnection::new(config)
                            .unwrap_or_else(|error| panic!("failed to start TLS: {error}"));
                        let mut stream = StreamOwned::new(connection, stream);
                        let _ = stream.conn.complete_io(&mut stream.sock);
                        return;
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        if Instant::now() >= deadline {
                            return;
                        }
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => return,
                }
            }
        });

        (address, handle)
    }

    fn tunneling_proxy() -> (SocketAddr, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .unwrap_or_else(|error| panic!("failed to bind proxy test server: {error}"));
        let address = listener
            .local_addr()
            .unwrap_or_else(|error| panic!("failed to read proxy test server address: {error}"));

        let handle = thread::spawn(move || {
            let Ok((mut client, _)) = listener.accept() else {
                return;
            };
            let mut request = Vec::new();
            let mut chunk = [0_u8; 256];
            while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                match client.read(&mut chunk) {
                    Ok(0) | Err(_) => return,
                    Ok(read) => request.extend_from_slice(&chunk[..read]),
                }
            }
            let Ok(request) = std::str::from_utf8(&request) else {
                return;
            };
            let authority = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1));
            let Some(authority) = authority else {
                return;
            };
            let Ok(mut upstream) = TcpStream::connect(authority) else {
                return;
            };
            if client
                .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
                .is_err()
            {
                return;
            }
            let Ok(mut client_reader) = client.try_clone() else {
                return;
            };
            let upstream_reader = upstream
                .try_clone()
                .unwrap_or_else(|error| panic!("failed to clone proxy stream: {error}"));
            let forward = thread::spawn(move || {
                let mut upstream_reader = upstream_reader;
                let _ = io::copy(&mut upstream_reader, &mut client_reader);
            });
            let _ = io::copy(&mut client, &mut upstream);
            let _ = forward.join();
        });

        (address, handle)
    }

    /// A path to a file in the temporary directory that is unique per test.
    ///
    /// The name of a test thread contains `::`, which is not a valid file name
    /// on Windows, so the name is built from a counter instead.
    fn temp_path(name: &str, extension: &str) -> std::path::PathBuf {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

        std::env::temp_dir().join(format!(
            "freshen-test-{name}-{}-{}.{extension}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ))
    }

    fn ca_bundle() -> std::path::PathBuf {
        let path = temp_path("ca", "pem");
        std::fs::write(&path, CA_CERTIFICATE)
            .unwrap_or_else(|error| panic!("failed to write test CA bundle: {error}"));
        path
    }

    /// Writes `content` to a new temporary file and returns its path.
    fn temp_file(name: &str, extension: &str, content: &[u8]) -> std::path::PathBuf {
        let path = temp_path(name, extension);
        std::fs::write(&path, content)
            .unwrap_or_else(|error| panic!("failed to write test file: {error}"));
        path
    }

    /// Decodes the body of a single-section PEM document into DER bytes.
    fn pem_body(pem: &str) -> Vec<u8> {
        let body: String = pem
            .lines()
            .filter(|line| !line.starts_with("-----"))
            .collect();
        base64::engine::general_purpose::STANDARD
            .decode(body.trim())
            .unwrap_or_else(|error| panic!("failed to decode test PEM body: {error}"))
    }

    #[test]
    fn https_proxy_is_preferred_over_http_proxy() {
        let environment = environment(&[
            ("HTTPS_PROXY", "http://secure-proxy:8443"),
            ("HTTP_PROXY", "http://plain-proxy:8080"),
        ]);
        let endpoint = endpoint("example.com", SocketAddr::from(([127, 0, 0, 1], 443)));

        let proxy = environment
            .proxy_for(&endpoint)
            .unwrap_or_else(|error| panic!("proxy should parse: {error}"))
            .unwrap_or_else(|| panic!("a proxy should be selected"));
        assert_eq!(proxy.host, "secure-proxy");
        assert_eq!(proxy.port, 8443);
    }

    #[test]
    fn http_proxy_is_used_when_no_https_proxy_is_set() {
        let environment = environment(&[("http_proxy", "plain-proxy:3128")]);
        let endpoint = endpoint("example.com", SocketAddr::from(([127, 0, 0, 1], 443)));

        let proxy = environment
            .proxy_for(&endpoint)
            .unwrap_or_else(|error| panic!("proxy should parse: {error}"))
            .unwrap_or_else(|| panic!("a proxy should be selected"));
        assert_eq!(proxy.host, "plain-proxy");
        assert_eq!(proxy.port, 3128);
        assert!(!proxy.secure);
    }

    #[test]
    fn proxy_credentials_are_encoded_for_the_connect_request() {
        let proxy = parse_proxy("http://user:secret@proxy.example:8080")
            .unwrap_or_else(|error| panic!("proxy should parse: {error}"));

        assert_eq!(proxy.authorization.as_deref(), Some("dXNlcjpzZWNyZXQ="));
    }

    #[test]
    fn an_https_proxy_is_marked_as_secure() {
        let proxy = parse_proxy("https://proxy.example:8443/")
            .unwrap_or_else(|error| panic!("proxy should parse: {error}"));

        assert!(proxy.secure);
        assert_eq!(proxy.host, "proxy.example");
        assert_eq!(proxy.port, 8443);
    }

    #[test]
    fn unsupported_proxy_schemes_are_rejected() {
        assert!(parse_proxy("socks5://proxy.example:1080").is_err());
    }

    #[test]
    fn no_proxy_entries_match_hosts_subdomains_wildcards_and_ports() {
        assert!(no_proxy_matches("example.com", "example.com", 443));
        assert!(no_proxy_matches("example.com", "api.example.com", 443));
        assert!(no_proxy_matches(".example.com", "example.com", 443));
        assert!(no_proxy_matches("*.example.com", "api.example.com", 443));
        assert!(!no_proxy_matches("*.example.com", "example.com", 443));
        assert!(no_proxy_matches("example.com:8443", "example.com", 8443));
        assert!(!no_proxy_matches("example.com:8443", "example.com", 443));
        assert!(no_proxy_matches("*", "anything.example", 443));
        assert!(!no_proxy_matches("example.com", "unrelated.example", 443));
        assert!(no_proxy_matches("10.0.0.1", "10.0.0.1", 443));
    }

    #[test]
    fn no_proxy_cidr_ranges_match_addresses_inside_the_network() {
        assert!(no_proxy_matches("10.0.0.0/8", "10.1.2.3", 443));
        assert!(no_proxy_matches("10.0.0.0/8", "10.255.255.255", 443));
        assert!(!no_proxy_matches("10.0.0.0/8", "11.0.0.1", 443));
        assert!(!no_proxy_matches("10.0.0.0/8", "9.255.255.255", 443));

        assert!(no_proxy_matches("192.168.0.0/16", "192.168.1.1", 443));
        assert!(!no_proxy_matches("192.168.0.0/16", "192.169.0.1", 443));

        assert!(no_proxy_matches("172.16.0.0/12", "172.31.255.255", 443));
        assert!(!no_proxy_matches("172.16.0.0/12", "172.32.0.1", 443));

        assert!(no_proxy_matches("0.0.0.0/0", "8.8.8.8", 443));
        assert!(!no_proxy_matches("10.0.0.0/33", "10.0.0.1", 443));
        assert!(!no_proxy_matches("10.0.0.0/8", "example.com", 443));

        assert!(no_proxy_matches("10.0.0.0/8:443", "10.0.0.1", 443));
        assert!(!no_proxy_matches("10.0.0.0/8:443", "10.0.0.1", 8443));

        assert!(no_proxy_matches("fd00::/8", "fd12:3456::1", 443));
        assert!(!no_proxy_matches("fd00::/8", "fe80::1", 443));
        assert!(no_proxy_matches("[fd00::/8]:443", "fd12:3456::1", 443));
        assert!(!no_proxy_matches("[fd00::/8]:443", "fd12:3456::1", 8443));
        assert!(!no_proxy_matches("fd00::/129", "fd12:3456::1", 443));
    }

    #[test]
    fn no_proxy_cidr_bypasses_the_configured_proxy() {
        let environment = environment(&[
            ("HTTPS_PROXY", "http://proxy.example:8080"),
            ("NO_PROXY", "10.0.0.0/8,192.168.0.0/16,fd00::/8"),
        ]);

        let private = endpoint(
            "unrelated.example",
            SocketAddr::from(([10, 20, 30, 40], 443)),
        );
        let local = endpoint(
            "unrelated.example",
            SocketAddr::from(([192, 168, 1, 1], 443)),
        );
        let public = endpoint("unrelated.example", SocketAddr::from(([8, 8, 8, 8], 443)));
        let v6 = endpoint(
            "unrelated.example",
            "[fd12:3456::1]:443"
                .parse()
                .unwrap_or_else(|error| panic!("test address should parse: {error}")),
        );

        assert!(environment.bypasses(&private));
        assert!(environment.bypasses(&local));
        assert!(environment.bypasses(&v6));
        assert!(!environment.bypasses(&public));
    }

    #[test]
    fn no_proxy_bypasses_the_configured_proxy() {
        let environment = environment(&[
            ("HTTPS_PROXY", "http://proxy.example:8080"),
            ("NO_PROXY", "example.com,10.0.0.1"),
        ]);

        let host = endpoint("example.com", SocketAddr::from(([127, 0, 0, 1], 443)));
        let address = endpoint("unrelated.example", SocketAddr::from(([10, 0, 0, 1], 443)));

        assert!(environment.bypasses(&host));
        assert!(environment.bypasses(&address));
    }

    #[test]
    fn a_custom_ca_bundle_is_trusted_by_the_network_check() {
        let (address, server) = tls_server();
        let bundle = ca_bundle();
        let environment = environment(&[(
            "SSL_CERT_FILE",
            bundle
                .to_str()
                .unwrap_or_else(|| panic!("test CA path is not UTF-8")),
        )]);

        assert!(check_with(
            &environment,
            &[endpoint("localhost", address)],
            Duration::from_secs(2)
        )
        .is_ok());
        server
            .join()
            .unwrap_or_else(|_| panic!("TLS test server panicked"));
        let _ = std::fs::remove_file(bundle);
    }

    #[test]
    fn a_custom_ca_bundle_is_added_to_the_trust_store() {
        let bundle = ca_bundle();
        let environment = environment(&[(
            "SSL_CERT_FILE",
            bundle
                .to_str()
                .unwrap_or_else(|| panic!("test CA path is not UTF-8")),
        )]);

        let roots = root_store(&environment)
            .unwrap_or_else(|error| panic!("the trust store should be built: {error}"));

        assert!(
            roots.subjects().iter().any(|subject| subject
                .as_ref()
                .windows(CA_SUBJECT.len())
                .any(|part| part == CA_SUBJECT)),
            "the custom CA certificate should be part of the trust store"
        );
        let _ = std::fs::remove_file(bundle);
    }

    #[test]
    fn a_pem_ca_bundle_is_read_as_certificates() {
        let path = temp_file("pem-ca", "pem", CA_CERTIFICATE.as_bytes());

        let certificates = certificates_from_file(&path)
            .unwrap_or_else(|error| panic!("the PEM bundle should be read: {error}"));

        assert_eq!(certificates.len(), 1);
        assert_eq!(
            certificates[0].as_ref(),
            pem_body(CA_CERTIFICATE).as_slice()
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_pem_ca_bundle_with_several_certificates_is_read_completely() {
        let bundle = format!("{CA_CERTIFICATE}{SERVER_CERTIFICATE}");
        let path = temp_file("multi-pem-ca", "pem", bundle.as_bytes());

        let certificates = certificates_from_file(&path)
            .unwrap_or_else(|error| panic!("the PEM bundle should be read: {error}"));

        assert_eq!(certificates.len(), 2);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_pem_ca_bundle_ignores_sections_that_are_not_certificates() {
        let bundle = format!("{SERVER_KEY}{CA_CERTIFICATE}");
        let path = temp_file("key-pem-ca", "pem", bundle.as_bytes());

        let certificates = certificates_from_file(&path)
            .unwrap_or_else(|error| panic!("the PEM bundle should be read: {error}"));

        assert_eq!(certificates.len(), 1);
        assert_eq!(
            certificates[0].as_ref(),
            pem_body(CA_CERTIFICATE).as_slice()
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_pem_ca_bundle_with_windows_line_endings_is_read_as_certificates() {
        let bundle = CA_CERTIFICATE.replace('\n', "\r\n");
        let path = temp_file("crlf-pem-ca", "pem", bundle.as_bytes());

        let certificates = certificates_from_file(&path)
            .unwrap_or_else(|error| panic!("the PEM bundle should be read: {error}"));

        assert_eq!(certificates.len(), 1);
        assert_eq!(
            certificates[0].as_ref(),
            pem_body(CA_CERTIFICATE).as_slice()
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_der_ca_bundle_is_read_as_a_certificate() {
        let path = temp_file("der-ca", "der", &pem_body(CA_CERTIFICATE));

        let certificates = certificates_from_file(&path)
            .unwrap_or_else(|error| panic!("the DER bundle should be read: {error}"));

        assert_eq!(certificates.len(), 1);
        assert_eq!(
            certificates[0].as_ref(),
            pem_body(CA_CERTIFICATE).as_slice()
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_broken_pem_ca_bundle_is_reported() {
        let bundle = "-----BEGIN CERTIFICATE-----\nnot base64!\n-----END CERTIFICATE-----\n";
        let path = temp_file("broken-pem-ca", "pem", bundle.as_bytes());

        let error = certificates_from_file(&path)
            .err()
            .unwrap_or_else(|| panic!("the broken bundle should be rejected"));

        assert!(error.contains("cannot parse CA bundle"), "{error}");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_missing_ca_bundle_is_reported() {
        let path = temp_path("missing-bundle", "pem");
        let _ = std::fs::remove_file(&path);

        let error = certificates_from_file(&path)
            .err()
            .unwrap_or_else(|| panic!("the missing bundle should be rejected"));

        assert!(error.contains("cannot read CA bundle"), "{error}");
    }

    #[test]
    fn a_tls_handshake_failure_is_reported() {
        let (address, server) = tls_server();

        let error = check_failure(
            &environment(&[]),
            &[endpoint("localhost", address)],
            Duration::from_secs(2),
        );

        assert!(matches!(error, NetworkCheckError::TlsHandshake { .. }));
        assert!(
            error
                .to_string()
                .contains("TLS handshake with localhost failed"),
            "{error}"
        );
        server
            .join()
            .unwrap_or_else(|_| panic!("TLS test server panicked"));
    }

    #[test]
    fn an_unreachable_endpoint_is_reported() {
        let endpoint = endpoint("localhost", SocketAddr::from(([127, 0, 0, 1], 1)));

        let error = check_failure(&environment(&[]), &[endpoint], Duration::from_millis(500));

        assert_eq!(error, NetworkCheckError::Unreachable);
        assert_eq!(error.to_string(), "network is unreachable");
    }

    #[test]
    fn an_invalid_proxy_url_is_reported() {
        let environment = environment(&[("HTTPS_PROXY", "socks5://proxy.example:1080")]);
        let endpoint = endpoint("example.com", SocketAddr::from(([127, 0, 0, 1], 443)));

        let error = check_failure(&environment, &[endpoint], Duration::from_millis(500));

        assert!(matches!(error, NetworkCheckError::InvalidProxy(_)));
        assert!(
            error
                .to_string()
                .contains("invalid proxy URL: unsupported proxy scheme 'socks5'"),
            "{error}"
        );
    }

    #[test]
    fn a_malformed_ca_bundle_fails_the_network_check() {
        let (address, server) = tls_server();
        let path = temp_path("bad-ca", "pem");
        std::fs::write(&path, "this is not a certificate")
            .unwrap_or_else(|error| panic!("failed to write test CA bundle: {error}"));
        let environment = environment(&[(
            "SSL_CERT_FILE",
            path.to_str()
                .unwrap_or_else(|| panic!("test CA path is not UTF-8")),
        )]);

        let error = check_failure(
            &environment,
            &[endpoint("localhost", address)],
            Duration::from_secs(2),
        );

        assert!(matches!(error, NetworkCheckError::TrustStore(_)));
        assert!(
            error
                .to_string()
                .contains("cannot parse certificate in CA bundle"),
            "{error}"
        );
        server
            .join()
            .unwrap_or_else(|_| panic!("TLS test server panicked"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn an_unreadable_ca_bundle_fails_the_network_check() {
        let (address, server) = tls_server();
        // A path that does not exist, so the bundle cannot be read on any
        // platform.
        let path = temp_path("missing-ca", "pem");
        let _ = std::fs::remove_file(&path);
        let environment = environment(&[(
            "SSL_CERT_FILE",
            path.to_str()
                .unwrap_or_else(|| panic!("test CA path is not UTF-8")),
        )]);

        let error = check_failure(
            &environment,
            &[endpoint("localhost", address)],
            Duration::from_secs(2),
        );

        assert!(matches!(error, NetworkCheckError::TrustStore(_)));
        assert!(
            error.to_string().contains("cannot read CA bundle"),
            "{error}"
        );
        server
            .join()
            .unwrap_or_else(|_| panic!("TLS test server panicked"));
    }

    #[test]
    fn the_proxy_is_used_and_no_proxy_bypasses_it() {
        let (address, server) = tls_server();
        let (proxy_address, proxy) = tunneling_proxy();
        let bundle = ca_bundle();
        let ca = bundle
            .to_str()
            .unwrap_or_else(|| panic!("test CA path is not UTF-8"));

        let proxied = environment(&[
            ("HTTPS_PROXY", &format!("http://{proxy_address}")),
            ("SSL_CERT_FILE", ca),
        ]);
        let direct = endpoint("localhost", address);
        let unreachable = Endpoint {
            address: SocketAddr::from(([127, 0, 0, 1], 1)),
            ..direct
        };

        assert!(check_with(&proxied, &[unreachable], Duration::from_secs(2)).is_ok());
        server
            .join()
            .unwrap_or_else(|_| panic!("TLS test server panicked"));
        proxy
            .join()
            .unwrap_or_else(|_| panic!("proxy test server panicked"));

        let (address, server) = tls_server();
        let bypassed = environment(&[
            ("HTTPS_PROXY", "http://127.0.0.1:1"),
            ("NO_PROXY", "localhost"),
            ("SSL_CERT_FILE", ca),
        ]);

        assert!(check_with(
            &bypassed,
            &[endpoint("localhost", address)],
            Duration::from_secs(2)
        )
        .is_ok());
        server
            .join()
            .unwrap_or_else(|_| panic!("TLS test server panicked"));
        let _ = std::fs::remove_file(bundle);
    }
}
