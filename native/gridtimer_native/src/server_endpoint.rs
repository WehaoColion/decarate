// v2.22.25 - Make every desktop sync route use one strict URL policy.

use std::fmt;
use url::{Host, Url};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatedServerUrl {
    normalized: String,
    loopback: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServerUrlError {
    Empty,
    Invalid,
    UnsupportedScheme,
    MissingHost,
    UserInfoForbidden,
    QueryForbidden,
    FragmentForbidden,
    InsecureRemoteHttp,
    InvalidPort,
}

impl fmt::Display for ServerUrlError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Empty => "同步地址为空",
            Self::Invalid => "同步地址格式无效",
            Self::UnsupportedScheme => "同步地址只允许 HTTP 或 HTTPS",
            Self::MissingHost => "同步地址缺少主机名",
            Self::UserInfoForbidden => "同步地址不能包含用户名或密码",
            Self::QueryForbidden => "同步地址不能包含查询参数",
            Self::FragmentForbidden => "同步地址不能包含片段标识",
            Self::InsecureRemoteHttp => "非本机同步地址必须使用 HTTPS",
            Self::InvalidPort => "同步地址端口无效",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for ServerUrlError {}

impl ValidatedServerUrl {
    pub fn parse(value: &str) -> Result<Self, ServerUrlError> {
        let raw = value.trim();
        if raw.is_empty() {
            return Err(ServerUrlError::Empty);
        }
        if raw
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
            || raw.contains('\\')
        {
            return Err(ServerUrlError::Invalid);
        }

        let url = Url::parse(raw).map_err(|_| ServerUrlError::Invalid)?;
        if url.cannot_be_a_base() {
            return Err(ServerUrlError::Invalid);
        }
        match url.scheme() {
            "http" | "https" => {}
            _ => return Err(ServerUrlError::UnsupportedScheme),
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err(ServerUrlError::UserInfoForbidden);
        }
        if url.query().is_some() {
            return Err(ServerUrlError::QueryForbidden);
        }
        if url.fragment().is_some() {
            return Err(ServerUrlError::FragmentForbidden);
        }
        if url.port() == Some(0) {
            return Err(ServerUrlError::InvalidPort);
        }

        let loopback = match url.host().ok_or(ServerUrlError::MissingHost)? {
            Host::Domain(domain) => domain.eq_ignore_ascii_case("localhost"),
            Host::Ipv4(address) => address.is_loopback(),
            Host::Ipv6(address) => address.is_loopback(),
        };
        if url.scheme() == "http" && !loopback {
            return Err(ServerUrlError::InsecureRemoteHttp);
        }

        let normalized = url.as_str().trim_end_matches('/').to_string();
        if normalized.is_empty() {
            return Err(ServerUrlError::Invalid);
        }
        Ok(Self {
            normalized,
            loopback,
        })
    }

    pub fn as_str(&self) -> &str {
        &self.normalized
    }

    pub fn is_loopback(&self) -> bool {
        self.loopback
    }

    pub fn endpoint(&self, path: &str) -> String {
        format!("{}/{}", self.normalized, path.trim_start_matches('/'))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_https_and_exact_loopback_http() {
        let remote = ValidatedServerUrl::parse("https://sync.example.com/base/").unwrap();
        assert_eq!("https://sync.example.com/base", remote.as_str());
        assert!(!remote.is_loopback());
        assert_eq!(
            "https://sync.example.com/base/health",
            remote.endpoint("/health")
        );

        for local in [
            "http://127.0.0.1:8917",
            "http://localhost:8917/",
            "http://[::1]:8917",
        ] {
            assert!(ValidatedServerUrl::parse(local).unwrap().is_loopback());
        }
    }

    #[test]
    fn rejects_ambiguous_or_credential_bearing_routes() {
        for invalid in [
            "",
            "http://example.com",
            "https://user:secret@example.com",
            "https://example.com/path?next=other",
            "https://example.com/#fragment",
            "https://example.com\\other",
            "ftp://example.com",
            "https://example.com:0",
            "https://exa mple.com",
        ] {
            assert!(
                ValidatedServerUrl::parse(invalid).is_err(),
                "unexpectedly accepted {invalid:?}"
            );
        }
    }
}
