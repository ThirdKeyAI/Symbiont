//! Host / IP egress policy (ported from the ToolClad reference runtime) shared by every argument type that can carry a
//! literal address: `scope_target`, `url`, `ip_address` and `cidr`.
//!
//! The range table in this module is the single source of truth for the Rust
//! runtime and is mirrored verbatim by the Go, JavaScript and Python runtimes,
//! so the four can be compared side by side. There are two tiers:
//!
//! * **Always blocked**, with or without `block_internal`: the link-local
//!   ranges. IPv4 link-local contains the cloud-metadata endpoint
//!   169.254.169.254, and the blast radius of reaching it (instance-credential
//!   theft) is maximal. No agent tool has a legitimate reason to target these.
//! * **Blocked when `block_internal = true`**: loopback, private, shared
//!   (carrier-grade NAT), unspecified, broadcast, documentation, benchmarking
//!   and reserved space, plus the IPv6 equivalents.
//!
//! IPv4-mapped IPv6 addresses (`::ffff:a.b.c.d`) are unwrapped before the
//! table is consulted, so the IPv6 spelling of any IPv4 range is caught.
//!
//! The parser helpers at the bottom (`parse_canonical_ip`,
//! `looks_like_ip_literal`, `has_punycode_label`) sit in front of the policy:
//! a value the resolver would expand (`2130706433`, `0x7f000001`,
//! `0177.0.0.1`, `127.1`) must be refused outright rather than passed through
//! as a "hostname" the table never sees.
//!
//! The hostname and range rules build on the table:
//!
//! * [`is_internal_name`] — the internal-name denylist (`localhost`,
//!   `*.local`, `*.internal`, ...), applied with or without `block_internal`.
//! * [`embedded_ip_literal`] — a dotted or dashed quad at the front of a
//!   hostname (`127.0.0.1.nip.io`, `10-0-0-1.nip.io`); wildcard-DNS services
//!   resolve such names to the address they spell, so the host is treated as
//!   that address.
//! * [`enforce_cidr_policy`] — a CIDR value is tested for *overlap* with every
//!   blocked range, not just its base address, so a supernet such as
//!   `8.0.0.0/5` cannot smuggle `10.0.0.0/8` past the table.
//! * [`enforce_resolved_host_policy`] — under `block_internal` a name is
//!   resolved and every A/AAAA answer is tested; a name that cannot be
//!   resolved cannot be shown to be public and is refused.
//!
//! [`enforce_hostname_policy`] runs those in order for a structurally valid
//! hostname and is the single entry point the validators call.

#[cfg(not(test))]
use std::net::ToSocketAddrs;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// One blocked IPv4 prefix and the reason reported when a target falls in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockedRange4 {
    pub net: Ipv4Addr,
    pub prefix: u8,
    pub reason: &'static str,
}

/// One blocked IPv6 prefix and the reason reported when a target falls in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockedRange6 {
    pub net: Ipv6Addr,
    pub prefix: u8,
    pub reason: &'static str,
}

const fn r4(a: u8, b: u8, c: u8, d: u8, prefix: u8, reason: &'static str) -> BlockedRange4 {
    BlockedRange4 {
        net: Ipv4Addr::new(a, b, c, d),
        prefix,
        reason,
    }
}

#[allow(clippy::too_many_arguments)]
const fn r6(
    s0: u16,
    s1: u16,
    s2: u16,
    s3: u16,
    s4: u16,
    s5: u16,
    s6: u16,
    s7: u16,
    prefix: u8,
    reason: &'static str,
) -> BlockedRange6 {
    BlockedRange6 {
        net: Ipv6Addr::new(s0, s1, s2, s3, s4, s5, s6, s7),
        prefix,
        reason,
    }
}

/// IPv4 ranges blocked regardless of `block_internal`.
pub const ALWAYS_BLOCKED_V4: &[BlockedRange4] = &[r4(
    169,
    254,
    0,
    0,
    16,
    "is a link-local / cloud-metadata address (169.254.0.0/16) — always blocked",
)];

/// IPv6 ranges blocked regardless of `block_internal`.
pub const ALWAYS_BLOCKED_V6: &[BlockedRange6] = &[r6(
    0xfe80,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    10,
    "is an IPv6 link-local address (fe80::/10) — always blocked",
)];

/// IPv4 ranges blocked when `block_internal = true`. Entries are tested in
/// order, so the broadcast address (which also lies inside 240.0.0.0/4) is
/// reported as broadcast, not as reserved.
pub const BLOCK_INTERNAL_V4: &[BlockedRange4] = &[
    r4(127, 0, 0, 0, 8, "resolves to a loopback address"),
    r4(10, 0, 0, 0, 8, "resolves to a private address"),
    r4(172, 16, 0, 0, 12, "resolves to a private address"),
    r4(192, 168, 0, 0, 16, "resolves to a private address"),
    r4(
        100,
        64,
        0,
        0,
        10,
        "resolves to a shared / carrier-grade NAT address (100.64.0.0/10)",
    ),
    r4(0, 0, 0, 0, 8, "is in the unspecified range (0.0.0.0/8)"),
    r4(255, 255, 255, 255, 32, "is the broadcast address"),
    r4(192, 0, 2, 0, 24, "is a documentation-range address"),
    r4(198, 51, 100, 0, 24, "is a documentation-range address"),
    r4(203, 0, 113, 0, 24, "is a documentation-range address"),
    r4(
        198,
        18,
        0,
        0,
        15,
        "is a benchmarking-range address (198.18.0.0/15)",
    ),
    r4(240, 0, 0, 0, 4, "is a reserved address (240.0.0.0/4)"),
];

/// IPv6 ranges blocked when `block_internal = true`. Link-local appears here
/// as well as in the always-blocked tier so the table matches the policy as
/// written; in practice the always-blocked check fires first.
pub const BLOCK_INTERNAL_V6: &[BlockedRange6] = &[
    r6(
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        1,
        128,
        "resolves to the IPv6 loopback ::1",
    ),
    r6(
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        128,
        "is the IPv6 unspecified address ::",
    ),
    r6(
        0xfc00,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        7,
        "is an IPv6 unique-local address (fc00::/7)",
    ),
    r6(
        0xfe80,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        10,
        "is an IPv6 link-local address (fe80::/10)",
    ),
    r6(
        0x2001,
        0x0db8,
        0,
        0,
        0,
        0,
        0,
        0,
        32,
        "is an IPv6 documentation-range address (2001:db8::/32)",
    ),
];

/// Network mask for an IPv4 prefix length (`/0` is all zeros).
fn v4_mask(prefix: u8) -> u32 {
    if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - u32::from(prefix.min(32)))
    }
}

/// Network mask for an IPv6 prefix length (`/0` is all zeros).
fn v6_mask(prefix: u8) -> u128 {
    if prefix == 0 {
        0
    } else {
        u128::MAX << (128 - u32::from(prefix.min(128)))
    }
}

/// True if the prefixes `a/ap` and `b/bp` share any address. Two prefixes
/// overlap exactly when one contains the other's network address, which is
/// the same as agreeing on the leading bits up to the *shorter* of the two
/// prefix lengths.
pub fn v4_overlaps(a: Ipv4Addr, ap: u8, b: Ipv4Addr, bp: u8) -> bool {
    let mask = v4_mask(ap.min(bp));
    (u32::from(a) & mask) == (u32::from(b) & mask)
}

/// IPv6 counterpart of [`v4_overlaps`].
pub fn v6_overlaps(a: Ipv6Addr, ap: u8, b: Ipv6Addr, bp: u8) -> bool {
    let mask = v6_mask(ap.min(bp));
    (u128::from(a) & mask) == (u128::from(b) & mask)
}

/// True if `ip` lies inside the range (prefix match on the leading bits).
pub fn v4_in_range(ip: Ipv4Addr, range: &BlockedRange4) -> bool {
    v4_overlaps(ip, 32, range.net, range.prefix)
}

/// True if `ip` lies inside the range (prefix match on the leading bits).
pub fn v6_in_range(ip: Ipv6Addr, range: &BlockedRange6) -> bool {
    v6_overlaps(ip, 128, range.net, range.prefix)
}

/// A parsed CIDR prefix (`addr/prefix`). `std` has no prefix type, so this
/// is the minimal one the policy needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IpNet {
    pub addr: IpAddr,
    pub prefix: u8,
}

impl std::fmt::Display for IpNet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.addr, self.prefix)
    }
}

/// Parse strict CIDR notation: a canonical IP literal (dotted-quad IPv4 or
/// standard IPv6, as accepted by [`parse_canonical_ip`]), one `/`, and a
/// prefix length written in plain decimal with no sign and no leading zero,
/// at most 32 for IPv4 and 128 for IPv6. Anything else — a shorthand or hex
/// base, `/+24`, `/024`, a bracketed IPv6 — is not a CIDR.
pub fn parse_cidr(val: &str) -> Option<IpNet> {
    let (base, prefix) = val.split_once('/')?;
    let addr = parse_canonical_ip(base)?;
    let plain_decimal = !prefix.is_empty()
        && prefix.len() <= 3
        && prefix.bytes().all(|b| b.is_ascii_digit())
        && (prefix == "0" || !prefix.starts_with('0'));
    if !plain_decimal {
        return None;
    }
    let prefix: u8 = prefix.parse().ok()?;
    let max = match addr {
        IpAddr::V4(_) => 32,
        IpAddr::V6(_) => 128,
    };
    (prefix <= max).then_some(IpNet { addr, prefix })
}

/// Overlap of `net` with a blocked IPv4 range. An IPv6 `net` is tested
/// against the range's IPv4-mapped image (`::ffff:a.b.c.d/(96+p)`), so
/// `::ffff:10.0.0.0/104` is caught like `10.0.0.0/8`.
fn net_overlaps_v4_range(net: &IpNet, range: &BlockedRange4) -> bool {
    match net.addr {
        IpAddr::V4(a) => v4_overlaps(a, net.prefix, range.net, range.prefix),
        IpAddr::V6(a) => v6_overlaps(a, net.prefix, range.net.to_ipv6_mapped(), 96 + range.prefix),
    }
}

/// Overlap of `net` with a blocked IPv6 range. No IPv6 range in the table
/// lies inside the IPv4-mapped block, so an IPv4 `net` never overlaps one.
fn net_overlaps_v6_range(net: &IpNet, range: &BlockedRange6) -> bool {
    match net.addr {
        IpAddr::V4(_) => false,
        IpAddr::V6(a) => v6_overlaps(a, net.prefix, range.net, range.prefix),
    }
}

fn v4_range_text(range: &BlockedRange4) -> String {
    format!("{}/{}", range.net, range.prefix)
}

fn v6_range_text(range: &BlockedRange6) -> String {
    format!("{}/{}", range.net, range.prefix)
}

/// The always-blocked range `net` overlaps, if any, as `(range, reason)`.
pub fn always_blocked_net_reason(net: &IpNet) -> Option<(String, &'static str)> {
    ALWAYS_BLOCKED_V4
        .iter()
        .find(|r| net_overlaps_v4_range(net, r))
        .map(|r| (v4_range_text(r), r.reason))
        .or_else(|| {
            ALWAYS_BLOCKED_V6
                .iter()
                .find(|r| net_overlaps_v6_range(net, r))
                .map(|r| (v6_range_text(r), r.reason))
        })
}

/// The `block_internal` range `net` overlaps, if any, as `(range, reason)`.
pub fn non_public_net_reason(net: &IpNet) -> Option<(String, &'static str)> {
    BLOCK_INTERNAL_V4
        .iter()
        .find(|r| net_overlaps_v4_range(net, r))
        .map(|r| (v4_range_text(r), r.reason))
        .or_else(|| {
            BLOCK_INTERNAL_V6
                .iter()
                .find(|r| net_overlaps_v6_range(net, r))
                .map(|r| (v6_range_text(r), r.reason))
        })
}

/// Enforce the egress policy for a CIDR target on behalf of argument `name`.
/// The whole range is tested for overlap with each blocked prefix: a range
/// that contains, is contained in, or is any blocked range is refused, so a
/// supernet (`8.0.0.0/5` ⊃ `10.0.0.0/8`) or `::/0` cannot pass on the
/// strength of a public base address.
pub fn enforce_cidr_policy(name: &str, net: &IpNet, block_internal: bool) -> Result<(), String> {
    if let Some((range, reason)) = always_blocked_net_reason(net) {
        return Err(format!(
            "argument '{name}' target range overlaps {range}, which {reason}"
        ));
    }
    if block_internal {
        if let Some((range, reason)) = non_public_net_reason(net) {
            return Err(format!(
                "argument '{name}' target range overlaps {range}, which {reason} (blocked by block_internal policy)"
            ));
        }
    }
    Ok(())
}

/// The IPv4 view of `ip`: the address itself, or the embedded address of an
/// IPv4-mapped IPv6 literal (`::ffff:a.b.c.d`). `None` for every other IPv6.
fn unwrap_v4(ip: &IpAddr) -> Option<Ipv4Addr> {
    match ip {
        IpAddr::V4(a) => Some(*a),
        IpAddr::V6(a) => a.to_ipv4_mapped(),
    }
}

/// Reason if `ip` must be blocked regardless of policy: link-local
/// (169.254.0.0/16, fe80::/10), which includes the cloud-metadata endpoint
/// 169.254.169.254. IPv4-mapped IPv6 is unwrapped first.
pub fn always_blocked_ip_reason(ip: &IpAddr) -> Option<&'static str> {
    if let Some(a) = unwrap_v4(ip) {
        return ALWAYS_BLOCKED_V4
            .iter()
            .find(|r| v4_in_range(a, r))
            .map(|r| r.reason);
    }
    if let IpAddr::V6(a) = ip {
        return ALWAYS_BLOCKED_V6
            .iter()
            .find(|r| v6_in_range(*a, r))
            .map(|r| r.reason);
    }
    None
}

/// Reason if `ip` is a non-public address that the opt-in `block_internal`
/// policy blocks: loopback, private, shared, unspecified, broadcast,
/// documentation, benchmarking, reserved, and the IPv6 equivalents.
/// IPv4-mapped IPv6 is unwrapped so `::ffff:127.0.0.1` is caught.
pub fn non_public_ip_reason(ip: &IpAddr) -> Option<&'static str> {
    if let Some(a) = unwrap_v4(ip) {
        return BLOCK_INTERNAL_V4
            .iter()
            .find(|r| v4_in_range(a, r))
            .map(|r| r.reason);
    }
    if let IpAddr::V6(a) = ip {
        return BLOCK_INTERNAL_V6
            .iter()
            .find(|r| v6_in_range(*a, r))
            .map(|r| r.reason);
    }
    None
}

/// Enforce the egress policy for a literal (or resolved) target IP on behalf
/// of argument `name`: link-local / metadata is always blocked; the remaining
/// non-public ranges are blocked only under the opt-in `block_internal`
/// policy.
pub fn enforce_ip_policy(name: &str, ip: &IpAddr, block_internal: bool) -> Result<(), String> {
    if let Some(reason) = always_blocked_ip_reason(ip) {
        return Err(format!("argument '{name}' target {reason}"));
    }
    if block_internal {
        if let Some(reason) = non_public_ip_reason(ip) {
            return Err(format!(
                "argument '{name}' target {reason} (blocked by block_internal policy)"
            ));
        }
        // Symbiont's HTTP egress guard is the union baseline: anything it
        // treats as non-public (multicast, IPv4-compatible IPv6, ...) is
        // refused here too, so the command path is never looser.
        if crate::net_guard::is_non_public_ip(*ip) {
            return Err(format!(
                "argument '{name}' target is a non-public address (blocked by block_internal policy)"
            ));
        }
    }
    Ok(())
}

/// Parse only canonical IP text (dotted-quad IPv4 / standard IPv6). Rejects
/// integer, hex, octal, and shorthand encodings by construction: the `std`
/// parsers accept nothing else.
pub fn parse_canonical_ip(val: &str) -> Option<IpAddr> {
    val.parse::<Ipv4Addr>()
        .map(IpAddr::V4)
        .or_else(|_| val.parse::<Ipv6Addr>().map(IpAddr::V6))
        .ok()
}

/// True if `val` is an attempt at an IP literal in a non-canonical encoding:
/// a `0x…` hex form, or every dot-separated label is all ASCII digits
/// (integer / dotted-decimal / octal / shorthand). A libc resolver would
/// expand any of these to an address, so they must never pass as a hostname.
/// Canonical IPs are handled by [`parse_canonical_ip`] first and never reach
/// here.
pub fn looks_like_ip_literal(val: &str) -> bool {
    let lower = val.to_ascii_lowercase();
    if lower.starts_with("0x") {
        return true;
    }
    val.split('.')
        .all(|label| !label.is_empty() && label.bytes().all(|b| b.is_ascii_digit()))
}

/// True if any DNS label in `host` is an A-label (`xn--…`). Comparison is
/// case-insensitive per IDNA: `XN--`, `Xn--`, `xN--` all count. Punycode is
/// rejected everywhere because a homoglyph domain supplied in its ASCII form
/// would otherwise satisfy an ASCII-only hostname check.
pub fn has_punycode_label(host: &str) -> bool {
    host.split('.')
        .any(|label| label.len() >= 4 && label.as_bytes()[..4].eq_ignore_ascii_case(b"xn--"))
}

/// Names refused outright, with or without `block_internal`: a host equal to
/// one of these.
pub const INTERNAL_NAME_EXACT: &[&str] = &["localhost", "metadata.google.internal"];

/// Names refused outright, with or without `block_internal`: a host ending in
/// one of these suffixes. `.local` is mDNS, `.internal` / `.home.arpa` /
/// `.lan` / `.intranet` / `.corp` / `.private` are the conventional private
/// zones, and `.localhost` is reserved for loopback (RFC 6761).
pub const INTERNAL_NAME_SUFFIXES: &[&str] = &[
    ".localhost",
    ".local",
    ".internal",
    ".home.arpa",
    ".lan",
    ".intranet",
    ".corp",
    ".private",
];

/// True if `host` is on the internal-name denylist. The comparison is
/// case-insensitive (DNS is) and ignores one trailing dot (an absolute name
/// is the same name); the validators' hostname grammar refuses a trailing dot
/// anyway, so this only matters for callers that use the helper directly.
pub fn is_internal_name(host: &str) -> bool {
    let lower = host.to_ascii_lowercase();
    let host = lower.strip_suffix('.').unwrap_or(&lower);
    INTERNAL_NAME_EXACT.contains(&host) || INTERNAL_NAME_SUFFIXES.iter().any(|s| host.ends_with(s))
}

/// The IPv4 address a hostname spells at its front, if any:
///
/// * the **first four** dot-separated labels form a canonical dotted quad
///   (`127.0.0.1.nip.io`, `169.254.169.254.xip.io`), or
/// * the **first label** is a dashed quad of four 1–3 digit groups each in
///   0–255 (`10-0-0-1.nip.io`).
///
/// Wildcard-DNS services resolve such names to exactly that address, which
/// made them a one-step bypass of a string-only host check. A host with an
/// embedded literal is treated as that address by the policy; the shape alone
/// is not a refusal (`93-184-216-34.example.com` is a public address).
pub fn embedded_ip_literal(host: &str) -> Option<Ipv4Addr> {
    let labels: Vec<&str> = host.split('.').collect();
    if labels.len() >= 4 {
        if let Ok(ip) = labels[..4].join(".").parse::<Ipv4Addr>() {
            return Some(ip);
        }
    }
    let first = labels.first()?;
    let groups: Vec<&str> = first.split('-').collect();
    if groups.len() != 4 {
        return None;
    }
    let mut octets = [0u8; 4];
    for (slot, group) in octets.iter_mut().zip(&groups) {
        let is_digits =
            !group.is_empty() && group.len() <= 3 && group.bytes().all(|b| b.is_ascii_digit());
        if !is_digits {
            return None;
        }
        *slot = group.parse().ok()?;
    }
    Some(Ipv4Addr::from(octets))
}

/// A hostname resolver: the system one, or a stand-in under test. Returns
/// every address the name maps to, or an error when it maps to none.
pub type HostResolver = fn(&str) -> std::io::Result<Vec<IpAddr>>;

/// Resolve `host` with the system resolver via `(host, 0).to_socket_addrs()`,
/// which returns every A and AAAA answer. `std` offers no per-call deadline;
/// the lookup is bounded by the platform resolver's own timeout and retry
/// settings (`resolv.conf` `options timeout:` / `attempts:` on Linux).
pub fn system_resolve(host: &str) -> std::io::Result<Vec<IpAddr>> {
    #[cfg(test)]
    {
        // Unit tests never touch live DNS: a test installs a resolver with
        // `with_test_resolver`, and without one every lookup fails closed.
        if let Some(resolve) = TEST_RESOLVER.with(std::cell::Cell::get) {
            return resolve(host);
        }
        let _ = host;
        Err(std::io::Error::other("DNS is disabled in unit tests"))
    }
    #[cfg(not(test))]
    {
        Ok((host, 0u16).to_socket_addrs()?.map(|sa| sa.ip()).collect())
    }
}

#[cfg(test)]
thread_local! {
    static TEST_RESOLVER: std::cell::Cell<Option<HostResolver>> = const { std::cell::Cell::new(None) };
}

/// Run `f` with `resolve` standing in for the system resolver on this thread.
#[cfg(test)]
pub(crate) fn with_test_resolver<T>(resolve: HostResolver, f: impl FnOnce() -> T) -> T {
    let previous = TEST_RESOLVER.with(|r| r.replace(Some(resolve)));
    let out = f();
    TEST_RESOLVER.with(|r| r.set(previous));
    out
}

/// Under `block_internal`, resolve `host` and apply the full range policy to
/// **every** answer: one internal address among the answers is a refusal.
/// This is the control that closes DNS rebinding and attacker-controlled
/// domains, so a name that cannot be resolved is refused rather than passed
/// (it cannot be shown to be public).
///
/// Resolve-then-reconnect window: the target binary performs its own lookup
/// after this check and may receive a different answer. A runtime closing
/// that window should pin the address verified here into the invocation
/// (for example `curl --resolve host:443:ip`); that pinning is out of scope
/// for the validator and is the follow-up this seam exists for.
pub fn enforce_resolved_host_policy(name: &str, host: &str) -> Result<(), String> {
    enforce_resolved_host_policy_with(name, host, system_resolve)
}

/// [`enforce_resolved_host_policy`] with an explicit resolver, so the
/// every-answer rule can be tested without live DNS.
pub fn enforce_resolved_host_policy_with(
    name: &str,
    host: &str,
    resolve: HostResolver,
) -> Result<(), String> {
    let answers = match resolve(host) {
        Ok(answers) if !answers.is_empty() => answers,
        _ => {
            return Err(format!(
                "argument '{name}' could not resolve host '{host}' to verify it is public (block_internal)"
            ));
        }
    };
    for ip in &answers {
        enforce_ip_policy(name, ip, true)?;
    }
    Ok(())
}

/// The hostname rules, in order, for a structurally valid hostname (not a
/// literal — the validators handle literals and CIDRs first):
///
/// 1. internal-name denylist — always refused;
/// 2. embedded IP literal — the host is treated as that address under the
///    range policy, and if the policy allows it the host is accepted as is;
/// 3. under `block_internal`, DNS resolution with the range policy applied
///    to every answer, failing closed when the name does not resolve.
pub fn enforce_hostname_policy(name: &str, host: &str, block_internal: bool) -> Result<(), String> {
    if is_internal_name(host) {
        return Err(format!(
            "argument '{name}' target resolves to an internal name — always blocked"
        ));
    }
    if let Some(ip) = embedded_ip_literal(host) {
        return enforce_ip_policy(name, &IpAddr::V4(ip), block_internal);
    }
    if block_internal {
        return enforce_resolved_host_policy(name, host);
    }
    Ok(())
}

/// The always-on tier for a free-form destination (a session target): an IP
/// literal or CIDR may not touch link-local / metadata space, and a name may
/// not be an internal name or embed such an address.
pub fn enforce_target_policy(name: &str, value: &str) -> Result<(), String> {
    if let Some(ip) = parse_canonical_ip(value) {
        return enforce_ip_policy(name, &ip, false);
    }
    if let Some(net) = parse_cidr(value) {
        return enforce_cidr_policy(name, &net, false);
    }
    if looks_like_ip_literal(value) {
        return Err(format!(
            "argument '{name}' target looks like a non-canonical IP literal"
        ));
    }
    enforce_hostname_policy(name, value, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap_or_else(|_| panic!("bad test literal {s}"))
    }

    #[test]
    fn always_blocked_tier_fires_in_both_modes() {
        for v in [
            "169.254.169.254",
            "169.254.0.1",
            "169.254.255.255",
            "::ffff:169.254.169.254",
            "fe80::1",
            "febf::1", // top of fe80::/10
        ] {
            let a = ip(v);
            assert!(always_blocked_ip_reason(&a).is_some(), "{v}");
            for block_internal in [false, true] {
                let err = enforce_ip_policy("t", &a, block_internal)
                    .unwrap_err()
                    .to_string();
                assert!(err.contains("always blocked"), "{v} -> {err}");
            }
        }
        // Just outside the ranges.
        for v in ["169.253.255.255", "169.255.0.0", "fe7f::1", "fec0::1"] {
            assert!(always_blocked_ip_reason(&ip(v)).is_none(), "{v}");
        }
    }

    #[test]
    fn block_internal_tier_covers_every_range() {
        // One representative per table row, plus the IPv4-mapped spelling.
        let cases: &[(&str, &str)] = &[
            ("127.0.0.1", "loopback"),
            ("127.255.255.254", "loopback"),
            ("10.0.0.5", "private"),
            ("172.16.0.1", "private"),
            ("172.31.255.255", "private"),
            ("192.168.1.1", "private"),
            ("100.64.0.1", "carrier-grade NAT"),
            ("100.127.255.255", "carrier-grade NAT"),
            ("0.0.0.0", "unspecified"),
            ("0.1.2.3", "unspecified"),
            ("255.255.255.255", "broadcast"),
            ("192.0.2.1", "documentation"),
            ("198.51.100.1", "documentation"),
            ("203.0.113.1", "documentation"),
            ("198.18.0.1", "benchmarking"),
            ("198.19.255.255", "benchmarking"),
            ("240.0.0.1", "reserved"),
            ("255.255.255.254", "reserved"),
            ("::1", "loopback"),
            ("::", "unspecified"),
            ("fc00::1", "unique-local"),
            ("fdff::1", "unique-local"),
            ("fe80::1", "link-local"),
            ("2001:db8::1", "documentation"),
            ("2001:db8:ffff:ffff:ffff:ffff:ffff:ffff", "documentation"),
            ("::ffff:127.0.0.1", "loopback"),
            ("::ffff:10.1.2.3", "private"),
            ("::ffff:100.64.0.1", "carrier-grade NAT"),
            ("::ffff:0.0.0.0", "unspecified"),
            ("::ffff:255.255.255.255", "broadcast"),
            ("::ffff:198.18.0.1", "benchmarking"),
            ("::ffff:240.0.0.1", "reserved"),
        ];
        for (v, word) in cases {
            let a = ip(v);
            let reason = non_public_ip_reason(&a).unwrap_or_else(|| panic!("{v} not blocked"));
            assert!(reason.contains(word), "{v}: {reason}");
            // Enforced when the policy is on. A row that also sits in the
            // always-blocked tier (fe80::/10) is reported by that tier first
            // and is refused with the policy off as well; every other row is
            // open with the policy off.
            let err = enforce_ip_policy("t", &a, true).unwrap_err().to_string();
            if always_blocked_ip_reason(&a).is_some() {
                assert!(err.contains("always blocked"), "{v} -> {err}");
                assert!(enforce_ip_policy("t", &a, false).is_err(), "{v} off");
            } else {
                assert!(
                    err.contains("blocked by block_internal policy"),
                    "{v} -> {err}"
                );
                assert!(enforce_ip_policy("t", &a, false).is_ok(), "{v} off");
            }
        }
    }

    #[test]
    fn block_internal_tier_range_boundaries() {
        // Immediately outside each range is public.
        for v in [
            "126.255.255.255",
            "128.0.0.0",
            "9.255.255.255",
            "11.0.0.0",
            "172.15.255.255",
            "172.32.0.0",
            "192.167.255.255",
            "192.169.0.0",
            "100.63.255.255",
            "100.128.0.0",
            "1.0.0.0",
            "192.0.1.255",
            "192.0.3.0",
            "198.51.99.255",
            "198.51.101.0",
            "203.0.112.255",
            "203.0.114.0",
            "198.17.255.255",
            "198.20.0.0",
            "93.184.216.34",
            "8.8.8.8",
            "fbff::1",
            "fe00::1",
            "2001:db7:ffff::1",
            "2001:db9::1",
            "2606:2800:220:1:248:1893:25c8:1946",
            "::ffff:93.184.216.34",
        ] {
            let a = ip(v);
            assert!(non_public_ip_reason(&a).is_none(), "{v} should be public");
            assert!(enforce_ip_policy("t", &a, true).is_ok(), "{v}");
        }
    }

    #[test]
    fn broadcast_is_reported_before_reserved() {
        assert_eq!(
            non_public_ip_reason(&ip("255.255.255.255")),
            Some("is the broadcast address")
        );
    }

    #[test]
    fn prefix_math_handles_zero_and_full_prefixes() {
        let all4 = r4(0, 0, 0, 0, 0, "everything");
        assert!(v4_in_range(Ipv4Addr::new(93, 184, 216, 34), &all4));
        let exact4 = r4(93, 184, 216, 34, 32, "exact");
        assert!(v4_in_range(Ipv4Addr::new(93, 184, 216, 34), &exact4));
        assert!(!v4_in_range(Ipv4Addr::new(93, 184, 216, 35), &exact4));

        let all6 = r6(0, 0, 0, 0, 0, 0, 0, 0, 0, "everything");
        assert!(v6_in_range(
            Ipv6Addr::new(0x2606, 0x2800, 0, 0, 0, 0, 0, 1),
            &all6
        ));
        let exact6 = r6(0, 0, 0, 0, 0, 0, 0, 1, 128, "exact");
        assert!(v6_in_range(Ipv6Addr::LOCALHOST, &exact6));
        assert!(!v6_in_range(Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 2), &exact6));
    }

    #[test]
    fn range_table_matches_policy_document() {
        // Pin the table shape so a drift in any runtime is a visible diff.
        let v4: Vec<String> = BLOCK_INTERNAL_V4
            .iter()
            .map(|r| format!("{}/{}", r.net, r.prefix))
            .collect();
        assert_eq!(
            v4,
            [
                "127.0.0.0/8",
                "10.0.0.0/8",
                "172.16.0.0/12",
                "192.168.0.0/16",
                "100.64.0.0/10",
                "0.0.0.0/8",
                "255.255.255.255/32",
                "192.0.2.0/24",
                "198.51.100.0/24",
                "203.0.113.0/24",
                "198.18.0.0/15",
                "240.0.0.0/4",
            ]
        );
        let v6: Vec<String> = BLOCK_INTERNAL_V6
            .iter()
            .map(|r| format!("{}/{}", r.net, r.prefix))
            .collect();
        assert_eq!(
            v6,
            [
                "::1/128",
                "::/128",
                "fc00::/7",
                "fe80::/10",
                "2001:db8::/32"
            ]
        );
        let always4: Vec<String> = ALWAYS_BLOCKED_V4
            .iter()
            .map(|r| format!("{}/{}", r.net, r.prefix))
            .collect();
        assert_eq!(always4, ["169.254.0.0/16"]);
        let always6: Vec<String> = ALWAYS_BLOCKED_V6
            .iter()
            .map(|r| format!("{}/{}", r.net, r.prefix))
            .collect();
        assert_eq!(always6, ["fe80::/10"]);
    }

    #[test]
    fn canonical_ip_parser_rejects_resolver_expansions() {
        assert!(parse_canonical_ip("127.0.0.1").is_some());
        assert!(parse_canonical_ip("::1").is_some());
        assert!(parse_canonical_ip("::ffff:127.0.0.1").is_some());
        for v in [
            "2130706433",
            "0x7f000001",
            "0177.0.0.1",
            "127.1",
            "010.0.0.1",
            "127.0.0.1.",
            "[::1]",
        ] {
            assert!(parse_canonical_ip(v).is_none(), "{v}");
        }
    }

    #[test]
    fn non_canonical_literal_detection() {
        for v in [
            "2130706433",
            "0x7f000001",
            "0X7F000001",
            "0177.0.0.1",
            "127.1",
            "1.2.3",
        ] {
            assert!(looks_like_ip_literal(v), "{v}");
        }
        for v in ["example.com", "localhost", "1.example", "a.1", "", "."] {
            assert!(!looks_like_ip_literal(v), "{v}");
        }
    }

    #[test]
    fn punycode_label_detection_is_case_insensitive() {
        for v in [
            "xn--example-9c.com",
            "sub.XN--example.com",
            "Xn--a.b",
            "a.xN--b",
        ] {
            assert!(has_punycode_label(v), "{v}");
        }
        for v in ["example.com", "xn-.com", "axn--b.com", "xn.com", ""] {
            assert!(!has_punycode_label(v), "{v}");
        }
    }

    #[test]
    fn internal_name_denylist() {
        for v in [
            "localhost",
            "LOCALHOST",
            "localhost.",
            "api.localhost",
            "a.b.c.localhost",
            "printer.local",
            "Printer.LOCAL",
            "metadata.google.internal",
            "METADATA.GOOGLE.INTERNAL.",
            "db.internal",
            "x.home.arpa",
            "nas.lan",
            "wiki.intranet",
            "fs.corp",
            "vpn.private",
        ] {
            assert!(is_internal_name(v), "{v}");
        }
        for v in [
            "localhost.com",
            "notlocalhost",
            "internal.example.com",
            "example.local.com",
            "local",
            "internal",
            "home.arpa",
            "corp",
            "lan",
            "example.com",
            "metadata.google.internal.example.com",
            "localhost..", // only one trailing dot is ignored
            "",
        ] {
            assert!(!is_internal_name(v), "{v}");
        }
    }

    #[test]
    fn embedded_ip_literal_shapes() {
        let found = |s: &str| embedded_ip_literal(s).map(|ip| ip.to_string());
        for (host, addr) in [
            ("127.0.0.1.nip.io", "127.0.0.1"),
            ("169.254.169.254.xip.io", "169.254.169.254"),
            ("10-0-0-1.nip.io", "10.0.0.1"),
            ("93-184-216-34.example.com", "93.184.216.34"),
            ("010-0-0-1.nip.io", "10.0.0.1"), // dashed groups are 1-3 digits, 0-255
            ("10-0-0-1", "10.0.0.1"),
            ("1.2.3.4.5.6", "1.2.3.4"),
            ("255-255-255-255.x", "255.255.255.255"),
        ] {
            assert_eq!(found(host).as_deref(), Some(addr), "{host}");
        }
        for v in [
            "example.com",
            "nip.io",
            "127.0.0.nip.io",
            "010.0.0.1.nip.io", // the dotted form must be canonical
            "256-0-0-1.nip.io",
            "1-2-3.nip.io",
            "1-2-3-4-5.nip.io",
            "a-b-c-d.nip.io",
            "1234-0-0-1.nip.io",
            "-1-2-3.nip.io",
            "1-2-3-.nip.io",
            "1.2.3.x.y",
            "",
        ] {
            assert_eq!(found(v), None, "{v}");
        }
    }

    #[test]
    fn cidr_parser_is_strict() {
        let net = |s: &str| parse_cidr(s).map(|n| n.to_string());
        for v in [
            "10.0.0.0/8",
            "10.0.0.5/24",
            "0.0.0.0/0",
            "8.8.8.8/32",
            "2001:db8::/32",
            "::/0",
            "::1/128",
            "::ffff:10.0.0.0/104",
        ] {
            assert_eq!(net(v).as_deref(), Some(v), "{v}");
        }
        for v in [
            "10.0.0.0/33",
            "2001:db8::/129",
            "2001:db8::/1000",
            "10.0.0.0/024",
            "10.0.0.0/+24",
            "10.0.0.0/-1",
            "10.0.0.0/0x8",
            "10.0.0.0/ 8",
            "10.0.0.0/",
            "/8",
            "10.0.0.0",
            "10.0.0/8",
            "0x0a000000/8",
            "0177.0.0.0/8",
            "127.1/8",
            "10.0.0.0/8/",
            "10.0.0.0/8/24",
            "[::1]/128",
            "",
        ] {
            assert_eq!(net(v), None, "{v}");
        }
    }

    #[test]
    fn overlap_boundaries_v4() {
        let o = |a: &str, ap: u8, b: &str, bp: u8| {
            v4_overlaps(a.parse().unwrap(), ap, b.parse().unwrap(), bp)
        };
        // Supernet, subnet, equal, everything.
        assert!(o("8.0.0.0", 5, "10.0.0.0", 8));
        assert!(o("10.0.0.0", 8, "8.0.0.0", 5));
        assert!(o("10.0.0.0", 8, "10.0.0.0", 8));
        assert!(o("10.1.2.0", 24, "10.0.0.0", 8));
        assert!(o("0.0.0.0", 0, "10.0.0.0", 8));
        assert!(o("0.0.0.0", 0, "255.255.255.255", 32));
        // 8.0.0.0/6 is 8-11 and holds 10/8; 8.0.0.0/7 is 8-9 and does not.
        assert!(o("8.0.0.0", 6, "10.0.0.0", 8));
        assert!(!o("8.0.0.0", 7, "10.0.0.0", 8));
        // One address either side of 10/8, and its first and last address.
        assert!(!o("9.255.255.255", 32, "10.0.0.0", 8));
        assert!(o("10.0.0.0", 32, "10.0.0.0", 8));
        assert!(o("10.255.255.255", 32, "10.0.0.0", 8));
        assert!(!o("11.0.0.0", 32, "10.0.0.0", 8));
        assert!(!o("9.0.0.0", 8, "10.0.0.0", 8));
        assert!(!o("11.0.0.0", 8, "10.0.0.0", 8));
        // 172.16/12 edges.
        assert!(!o("172.15.255.255", 32, "172.16.0.0", 12));
        assert!(o("172.16.0.0", 32, "172.16.0.0", 12));
        assert!(o("172.31.255.255", 32, "172.16.0.0", 12));
        assert!(!o("172.32.0.0", 32, "172.16.0.0", 12));
        assert!(o("172.0.0.0", 8, "172.16.0.0", 12));
        assert!(!o("172.32.0.0", 11, "172.16.0.0", 12));
        // Host routes.
        assert!(o("1.2.3.4", 32, "1.2.3.4", 32));
        assert!(!o("1.2.3.4", 32, "1.2.3.5", 32));
        assert!(o("1.2.3.4", 31, "1.2.3.5", 32));
    }

    #[test]
    fn overlap_boundaries_v6() {
        let o = |a: &str, ap: u8, b: &str, bp: u8| {
            v6_overlaps(a.parse().unwrap(), ap, b.parse().unwrap(), bp)
        };
        assert!(o("2001:db8::", 32, "2001:db8:ffff::", 48));
        assert!(o("2001:db8:ffff::", 48, "2001:db8::", 32));
        assert!(o("2000::", 3, "2001:db8::", 32));
        assert!(o("::", 0, "fe80::", 10));
        assert!(!o("2001:db9::", 32, "2001:db8::", 32));
        assert!(!o("2001:db7:ffff::", 48, "2001:db8::", 32));
        assert!(o(
            "2001:db8:ffff:ffff:ffff:ffff:ffff:ffff",
            128,
            "2001:db8::",
            32
        ));
        assert!(!o("2001:db9::", 128, "2001:db8::", 32));
        // fe80::/10 is fe80-febf.
        assert!(!o("fe7f:ffff::", 32, "fe80::", 10));
        assert!(o("fe80::", 128, "fe80::", 10));
        assert!(o("febf:ffff::", 32, "fe80::", 10));
        assert!(!o("fec0::", 32, "fe80::", 10));
        assert!(o("fe00::", 7, "fe80::", 10)); // fe00-feff
        assert!(!o("fe00::", 9, "fe80::", 10)); // fe00-fe7f
                                                // fc00::/7 is fc00-fdff.
        assert!(o("fdff::", 16, "fc00::", 7));
        assert!(!o("fe00::", 16, "fc00::", 7));
        assert!(!o("fbff::", 16, "fc00::", 7));
        // Host routes.
        assert!(o("::1", 128, "::1", 128));
        assert!(!o("::2", 128, "::1", 128));
        assert!(o("::", 127, "::1", 128));
    }

    #[test]
    fn cidr_policy_tests_overlap_not_base() {
        let net = |s: &str| parse_cidr(s).unwrap_or_else(|| panic!("bad test cidr {s}"));
        // The supernet case: a public base address that contains 10.0.0.0/8.
        let err = enforce_cidr_policy("t", &net("8.0.0.0/5"), true)
            .unwrap_err()
            .to_string();
        assert!(err.contains("overlaps 10.0.0.0/8"), "{err}");
        assert!(err.contains("blocked by block_internal policy"), "{err}");
        // It holds no link-local space, so it is open without the policy.
        assert!(enforce_cidr_policy("t", &net("8.0.0.0/5"), false).is_ok());

        // Anything touching the always-blocked tier is refused in both modes,
        // including the whole address space and the IPv4-mapped block.
        for v in [
            "0.0.0.0/0",
            "::/0",
            "169.254.0.0/16",
            "169.254.169.254/32",
            "169.0.0.0/8",
            "::ffff:169.254.0.0/112",
            "::ffff:0:0/96",
            "::ffff:0:0/95",
            "fe80::/10",
            "fe00::/7",
        ] {
            for block_internal in [false, true] {
                let err = enforce_cidr_policy("t", &net(v), block_internal)
                    .unwrap_err()
                    .to_string();
                assert!(err.contains("always blocked"), "{v}: {err}");
            }
        }

        // Inside, equal to, or containing a `block_internal` range — v4, v6
        // and v4-mapped — refused with the policy and open without it.
        for (v, range) in [
            ("10.0.0.0/24", "10.0.0.0/8"),
            ("10.0.0.0/8", "10.0.0.0/8"),
            ("127.0.0.1/32", "127.0.0.0/8"),
            ("172.16.0.0/12", "172.16.0.0/12"),
            ("172.0.0.0/8", "172.16.0.0/12"),
            ("192.168.1.0/24", "192.168.0.0/16"),
            ("192.0.0.0/8", "192.168.0.0/16"),
            ("100.64.0.0/10", "100.64.0.0/10"),
            ("198.18.0.0/15", "198.18.0.0/15"),
            ("2001:db8::/32", "2001:db8::/32"),
            ("2001:db8:1::/48", "2001:db8::/32"),
            ("2001::/16", "2001:db8::/32"),
            ("fc00::/7", "fc00::/7"),
            ("fd00::/8", "fc00::/7"),
            ("::1/128", "::1/128"),
            ("::/127", "::1/128"),
            ("::ffff:10.0.0.0/104", "10.0.0.0/8"),
            ("::ffff:127.0.0.1/128", "127.0.0.0/8"),
        ] {
            let err = enforce_cidr_policy("t", &net(v), true)
                .unwrap_err()
                .to_string();
            assert!(err.contains(&format!("overlaps {range}")), "{v}: {err}");
            assert!(
                err.contains("blocked by block_internal policy"),
                "{v}: {err}"
            );
            assert!(enforce_cidr_policy("t", &net(v), false).is_ok(), "{v} open");
        }

        // Public ranges pass in both modes.
        for v in [
            "93.184.216.0/24",
            "8.8.8.0/24",
            "1.0.0.0/8",
            "11.0.0.0/8",
            "172.32.0.0/11",
            "192.169.0.0/16",
            "2606:4700::/32",
            "2a00::/12",
            "2001:db9::/32",
            "2001:db7::/32",
            "::ffff:93.184.216.0/120",
            "::2/127",
        ] {
            assert!(enforce_cidr_policy("t", &net(v), true).is_ok(), "{v}");
            assert!(enforce_cidr_policy("t", &net(v), false).is_ok(), "{v}");
        }
    }

    #[test]
    fn resolved_host_policy_checks_every_answer() {
        fn all_public(_: &str) -> std::io::Result<Vec<IpAddr>> {
            Ok(vec![
                ip("93.184.216.34"),
                ip("2606:2800:220:1:248:1893:25c8:1946"),
            ])
        }
        fn one_internal(_: &str) -> std::io::Result<Vec<IpAddr>> {
            Ok(vec![ip("93.184.216.34"), ip("10.0.0.1")])
        }
        fn metadata(_: &str) -> std::io::Result<Vec<IpAddr>> {
            Ok(vec![ip("169.254.169.254")])
        }
        fn mapped_loopback(_: &str) -> std::io::Result<Vec<IpAddr>> {
            Ok(vec![ip("::ffff:127.0.0.1")])
        }
        fn empty(_: &str) -> std::io::Result<Vec<IpAddr>> {
            Ok(vec![])
        }
        fn failing(_: &str) -> std::io::Result<Vec<IpAddr>> {
            Err(std::io::Error::other("no such host"))
        }

        assert!(enforce_resolved_host_policy_with("t", "h.example", all_public).is_ok());
        let err = enforce_resolved_host_policy_with("t", "h.example", one_internal)
            .unwrap_err()
            .to_string();
        assert!(err.contains("blocked by block_internal policy"), "{err}");
        let err = enforce_resolved_host_policy_with("t", "h.example", mapped_loopback)
            .unwrap_err()
            .to_string();
        assert!(err.contains("loopback"), "{err}");
        let err = enforce_resolved_host_policy_with("t", "h.example", metadata)
            .unwrap_err()
            .to_string();
        assert!(err.contains("always blocked"), "{err}");
        for resolver in [empty as HostResolver, failing] {
            let err = enforce_resolved_host_policy_with("t", "h.example", resolver)
                .unwrap_err()
                .to_string();
            assert_eq!(
                err,
                "argument 't' could not resolve host 'h.example' to verify it is public (block_internal)"
            );
        }
    }

    #[test]
    fn hostname_policy_order() {
        // The denylist wins in both modes.
        for block_internal in [false, true] {
            let err = enforce_hostname_policy("t", "api.localhost", block_internal)
                .unwrap_err()
                .to_string();
            assert_eq!(
                err,
                "argument 't' target resolves to an internal name — always blocked"
            );
        }
        // An embedded literal is treated as that address: a public one needs
        // no lookup, an internal one is refused under the policy, link-local
        // is refused regardless.
        assert!(enforce_hostname_policy("t", "93-184-216-34.example.com", true).is_ok());
        assert!(enforce_hostname_policy("t", "93.184.216.34.nip.io", true).is_ok());
        let err = enforce_hostname_policy("t", "127.0.0.1.nip.io", true)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("loopback") && err.contains("block_internal"),
            "{err}"
        );
        assert!(enforce_hostname_policy("t", "127.0.0.1.nip.io", false).is_ok());
        let err = enforce_hostname_policy("t", "169.254.169.254.nip.io", false)
            .unwrap_err()
            .to_string();
        assert!(err.contains("always blocked"), "{err}");
        // Resolution happens only under the policy. `.invalid` is reserved by
        // RFC 2606 and never resolves: open without the policy, refused with it.
        assert!(enforce_hostname_policy("t", "no-such-host.invalid", false).is_ok());
        let err = enforce_hostname_policy("t", "no-such-host.invalid", true)
            .unwrap_err()
            .to_string();
        assert_eq!(
            err,
            "argument 't' could not resolve host 'no-such-host.invalid' to verify it is public (block_internal)"
        );
    }
}
