# DNSChecker Architecture

DNSChecker has a small frontend and a native Tauri backend. The frontend owns
editing, progress display, and clipboard behavior. The backend owns parsing DNS
server definitions, executing the requested DNS protocol, and returning complete
result objects to the UI.

## Protocol Contract

Each server line selects exactly one transport. The backend must not silently
fallback to another transport.

- `udp://host[:port]` uses DNS over UDP. The default port is `53`.
- `tcp://host[:port]` uses DNS over TCP. The default port is `53`.
- `dot://host[:port]` and `tls://host[:port]` use DNS over TLS. The default port is `853`.
- `https://host/path` and `doh://host/path` use DNS over HTTPS. The default path is `/dns-query`.
- A bare IP or host is treated as UDP on port `53`.

The optional second column is a bootstrap address list. It pins where a hostname
connects without changing the hostname used for TLS certificate verification.
For example, `dot://dot.pub:853 1.12.12.12` connects to `1.12.12.12:853` while
still verifying the certificate for `dot.pub`.

## TLS Verification

DoT is implemented directly in `src-tauri/src/lib.rs` because the upstream
Hickory DoT connector disables SNI. Some public resolvers require SNI and normal
server identity during the TLS handshake.

TLS is performed by `rustls`. Certificate path and hostname verification use
`rustls-platform-verifier`, so verification is delegated to the operating
system trust store. This is the same intent as system SSL validation: certificates
trusted by the OS should validate, and hostname mismatches should still fail.

DoH uses Hickory Resolver with the `rustls-platform-verifier` feature enabled.
Its HTTPS connections therefore also use Rust TLS with OS trust store
verification.

## Error Reporting

The backend returns full error strings. When a hostname resolves or bootstraps to
multiple addresses, each address is tried as a separate upstream for the same
requested protocol. If every address fails, the returned error includes every
address and its failure reason.

The UI may display a compact label such as `timeout` or `error`, but clicking
the cell copies the complete backend error string. This keeps the table readable
without losing diagnostic detail.

## Test Data

Protocol tests use domestic public resolvers as first-class coverage targets:

- AliDNS: `223.5.5.5`, `2400:3200:baba::1`, `dns.alidns.com`
- DNSPod: `119.29.29.29`, `dot.pub`

These tests verify that parser behavior and protocol execution match the server
list used by the UI. A timeout against these entries is treated as a failure to
investigate, not as an expected outcome.
