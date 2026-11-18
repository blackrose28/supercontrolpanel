//! Tiny URL helpers for the OIDC flow's host/scheme policy (no URL crate).

/// A parsed absolute `http(s)` URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Url {
    pub scheme: String,
    /// Lower-cased `host[:port]` with the scheme's default port removed.
    pub authority: String,
    pub host: String,
    /// Path plus query, always starting with `/`; fragments are dropped.
    pub path_query: String,
}

impl Url {
    /// `https` everywhere; plain `http` only for loopback hosts (local test fakes).
    pub fn scheme_allowed(&self) -> bool {
        self.scheme == "https" || (self.scheme == "http" && is_loopback_host(&self.host))
    }

    pub fn path(&self) -> &str {
        self.path_query.split('?').next().unwrap_or("/")
    }

    pub fn as_string(&self) -> String {
        format!("{}://{}{}", self.scheme, self.authority, self.path_query)
    }
}

/// Parses an absolute `http`/`https` URL; rejects userinfo, empty hosts and other schemes.
pub fn parse(url: &str) -> Option<Url> {
    let url = url.trim();
    let (scheme, rest) = url.split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return None;
    }
    let rest = rest.split('#').next().unwrap_or("");
    let auth_end = rest.find(['/', '?']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(auth_end);
    if authority.is_empty() || authority.contains('@') || authority.contains(char::is_whitespace) {
        return None;
    }
    let authority = authority.to_ascii_lowercase();
    let (host, port) = split_host_port(&authority)?;
    if host.is_empty() {
        return None;
    }
    let default_port = if scheme == "https" { "443" } else { "80" };
    let authority = match port {
        Some(p) if p != default_port => format!("{host}:{p}"),
        _ => host.to_string(),
    };
    let path_query = match tail {
        "" => "/".to_string(),
        t if t.starts_with('?') => format!("/{t}"),
        t => t.to_string(),
    };
    Some(Url {
        scheme,
        authority,
        host: host.to_string(),
        path_query,
    })
}

fn split_host_port(authority: &str) -> Option<(&str, Option<&str>)> {
    if let Some(stripped) = authority.strip_prefix('[') {
        let close = stripped.find(']')?;
        let host = &authority[..close + 2];
        let after = &authority[close + 2..];
        return match after {
            "" => Some((host, None)),
            a => Some((host, Some(valid_port(a.strip_prefix(':')?)?))),
        };
    }
    match authority.rsplit_once(':') {
        Some((host, port)) => Some((host, Some(valid_port(port)?))),
        None => Some((authority, None)),
    }
}

fn valid_port(port: &str) -> Option<&str> {
    (!port.is_empty() && port.len() <= 5 && port.bytes().all(|b| b.is_ascii_digit()))
        .then_some(port)
}

pub fn is_loopback_host(host: &str) -> bool {
    if host == "localhost" || host == "[::1]" {
        return true;
    }
    let octets: Vec<&str> = host.split('.').collect();
    octets.len() == 4
        && octets[0] == "127"
        && octets
            .iter()
            .all(|o| !o.is_empty() && o.len() <= 3 && o.bytes().all(|b| b.is_ascii_digit()))
}

/// Resolves `reference` (absolute, `//host`, `/path`, `?query` or relative) against `base`.
pub fn resolve(base: &Url, reference: &str) -> Option<Url> {
    let reference = reference.trim();
    if reference.contains("://") {
        return parse(reference);
    }
    if let Some(rest) = reference.strip_prefix("//") {
        return parse(&format!("{}://{rest}", base.scheme));
    }
    if reference.contains(':') && !reference.starts_with(['/', '?', '.']) {
        // `javascript:`, `mailto:` and friends.
        let first = reference.split(['/', '?']).next().unwrap_or("");
        if first.contains(':') {
            return None;
        }
    }
    let reference = reference.split('#').next().unwrap_or("");
    let path_query = if reference.is_empty() {
        base.path_query.clone()
    } else if reference.starts_with('/') {
        normalise_path(reference)
    } else if let Some(q) = reference.strip_prefix('?') {
        format!("{}?{q}", base.path())
    } else {
        let dir = base.path().rsplit_once('/').map_or("", |(d, _)| d);
        normalise_path(&format!("{dir}/{reference}"))
    };
    Some(Url {
        scheme: base.scheme.clone(),
        authority: base.authority.clone(),
        host: base.host.clone(),
        path_query,
    })
}

fn normalise_path(path_and_query: &str) -> String {
    let (path, query) = match path_and_query.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (path_and_query, None),
    };
    let mut out: Vec<&str> = Vec::new();
    for seg in path.split('/').skip(1) {
        match seg {
            "." => {}
            ".." => {
                out.pop();
            }
            s => out.push(s),
        }
    }
    let mut joined = format!("/{}", out.join("/"));
    if let Some(q) = query {
        joined.push('?');
        joined.push_str(q);
    }
    joined
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_normalises_authority() {
        let u = parse("HTTPS://IdP.Example.test:443/realms/r?a=1&b=2#frag").unwrap();
        assert_eq!(u.scheme, "https");
        assert_eq!(u.authority, "idp.example.test");
        assert_eq!(u.host, "idp.example.test");
        assert_eq!(u.path_query, "/realms/r?a=1&b=2");
        assert_eq!(u.path(), "/realms/r");
        let v = parse("http://127.0.0.1:8080").unwrap();
        assert_eq!(v.authority, "127.0.0.1:8080");
        assert_eq!(v.path_query, "/");
        assert_eq!(parse("https://[::1]:9/x").unwrap().host, "[::1]");
    }

    #[test]
    fn rejects_userinfo_empty_host_and_foreign_schemes() {
        for bad in [
            "https://user@evil.test/",
            "https://good.test@evil.test/",
            "https:///path",
            "ftp://x.test/",
            "javascript:alert(1)",
            "/relative",
            "https://",
            "https://host:notaport/",
        ] {
            assert!(parse(bad).is_none(), "{bad} must be rejected");
        }
    }

    #[test]
    fn scheme_policy_allows_http_only_on_loopback() {
        assert!(parse("https://a.test/").unwrap().scheme_allowed());
        assert!(!parse("http://a.test/").unwrap().scheme_allowed());
        for host in ["127.0.0.1", "localhost", "[::1]", "127.9.9.9"] {
            assert!(
                parse(&format!("http://{host}:1/"))
                    .unwrap()
                    .scheme_allowed(),
                "{host}"
            );
        }
        assert!(!parse("http://127.0.0.1.evil.test/")
            .unwrap()
            .scheme_allowed());
    }

    #[test]
    fn resolves_references() {
        let base = parse("https://idp.test/realms/r/login?x=1").unwrap();
        let r = |s: &str| resolve(&base, s).unwrap().as_string();
        assert_eq!(r("https://other.test/a"), "https://other.test/a");
        assert_eq!(r("//other.test/a"), "https://other.test/a");
        assert_eq!(r("/abs?q=1"), "https://idp.test/abs?q=1");
        assert_eq!(r("next"), "https://idp.test/realms/r/next");
        assert_eq!(r("?y=2"), "https://idp.test/realms/r/login?y=2");
        assert_eq!(r("../up#f"), "https://idp.test/realms/up");
        assert_eq!(r("a/./b/../c"), "https://idp.test/realms/r/a/c");
        assert!(resolve(&base, "javascript:alert(1)").is_none());
    }
}
