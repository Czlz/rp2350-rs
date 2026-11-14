//! Bearer-token authentication for the HTTP transport
//! HTTP 传输的 Bearer Token 鉴权
//!
//! Only a static bearer token is supported: no users, roles, rate limiting or
//! TLS. A token is compared byte by byte with [`constant_time_eq`], so the
//! comparison time does not depend on how many leading bytes match.
//! 仅支持静态 Bearer Token：没有用户、角色、限流或 TLS。Token 通过
//! [`constant_time_eq`] 逐字节比较，比较耗时与匹配的前缀长度无关。
//!
//! The token is never rendered: [`Auth`] implements [`std::fmt::Debug`] with a
//! redacted value so startup banners and logs cannot leak it.
//! Token 永不回显：[`Auth`] 的 [`std::fmt::Debug`] 实现会脱敏，启动横幅与日志
//! 不会泄露它。

use std::fmt;

/// Authorization scheme required by the transport / 传输要求的鉴权方案
pub const AUTH_SCHEME: &str = "Bearer";

/// Value of the `WWW-Authenticate` header sent with `401` / `401` 响应的
/// `WWW-Authenticate` 头值
pub const WWW_AUTHENTICATE: &str = AUTH_SCHEME;

/// Environment variable read by the CLI when `--auth-token` is absent
/// `--auth-token` 缺省时 CLI 读取的环境变量
pub const TOKEN_ENV: &str = "RP2350_MCP_TOKEN";

/// Static bearer-token configuration / 静态 Bearer Token 配置
///
/// `token: None` means authentication is disabled and every request is
/// accepted. `token: Some(...)` requires `Authorization: Bearer <token>` on
/// `POST /mcp`; `GET /health` stays open either way.
/// `token: None` 表示关闭鉴权、所有请求放行；`token: Some(...)` 则要求
/// `POST /mcp` 携带 `Authorization: Bearer <token>`，而 `GET /health` 始终开放。
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Auth {
    /// Expected token, or `None` when authentication is disabled
    /// 期望的 Token；`None` 表示关闭鉴权
    token: Option<String>,
}

impl Auth {
    /// Authentication disabled (accept everything) / 关闭鉴权（全部接受）
    #[must_use]
    pub fn disabled() -> Self {
        Self { token: None }
    }

    /// Require exactly `token` on `POST /mcp` / `POST /mcp` 要求精确匹配 `token`
    ///
    /// An empty token is treated as "disabled", matching the CLI rule that an
    /// empty flag or environment variable turns authentication off.
    /// 空 Token 视为「关闭」，与 CLI 中空标志/空环境变量关闭鉴权的规则一致。
    #[must_use]
    pub fn bearer(token: impl Into<String>) -> Self {
        Self::from_optional(Some(token.into()))
    }

    /// Build from an optional token; `None` and `""` disable authentication
    /// 由可选 Token 构建；`None` 与 `""` 均关闭鉴权
    #[must_use]
    pub fn from_optional(token: Option<String>) -> Self {
        Self {
            token: token.filter(|value| !value.is_empty()),
        }
    }

    /// Whether a token is configured / 是否配置了 Token
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.token.is_some()
    }

    /// Check one `Authorization` header value / 校验一条 `Authorization` 头值
    ///
    /// Returns `true` when authentication is disabled. Otherwise the scheme is
    /// matched case-insensitively (`Bearer`, `bearer`, `BEARER`, …) and the
    /// credentials must equal the configured token exactly.
    /// 关闭鉴权时恒为 `true`；否则方案名大小写不敏感（`Bearer`/`bearer`/
    /// `BEARER` 等），凭据必须与配置的 Token 完全一致。
    #[must_use]
    pub fn authorize(&self, header: Option<&str>) -> bool {
        match self.token.as_deref() {
            None => true,
            Some(token) => bearer_matches(header, token),
        }
    }
}

// 自定义 Debug 实现避免 Token 出现在日志、panic 消息或启动横幅里。
// A custom Debug implementation keeps the token out of logs, panic messages
// and startup banners.
impl fmt::Debug for Auth {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Auth")
            .field("enabled", &self.is_enabled())
            .field("token", &self.token.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

/// Options for [`crate::http::serve_with_options`] / [`crate::http::serve_with_options`]
/// 的选项
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HttpOptions {
    /// Bearer-token policy / Bearer Token 策略
    pub auth: Auth,
}

impl HttpOptions {
    /// Default options: authentication disabled / 默认选项：关闭鉴权
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Require `token` on `POST /mcp` / `POST /mcp` 要求 `token`
    #[must_use]
    pub fn with_token(token: impl Into<String>) -> Self {
        Self {
            auth: Auth::bearer(token),
        }
    }

    /// Use an optional token (`None`/`""` disable auth) / 使用可选 Token
    /// （`None`/`""` 关闭鉴权）
    #[must_use]
    pub fn with_optional_token(token: Option<String>) -> Self {
        Self {
            auth: Auth::from_optional(token),
        }
    }

    /// Whether authentication is enabled / 是否启用鉴权
    #[must_use]
    pub fn auth_enabled(&self) -> bool {
        self.auth.is_enabled()
    }
}

/// Parse a `Bearer <token>` header value and compare it in constant time
/// 解析 `Bearer <token>` 头值并以常量时间比较
fn bearer_matches(header: Option<&str>, expected: &str) -> bool {
    let Some(value) = header else {
        return false;
    };
    // 方案名与凭据之间允许一个空格；凭据两端空白忽略以兼容 `Bearer  x`。
    // One space separates scheme and credentials; surrounding whitespace is
    // ignored for compatibility with `Bearer  x`.
    let Some((scheme, credentials)) = value.split_once(' ') else {
        return false;
    };
    scheme.eq_ignore_ascii_case(AUTH_SCHEME) && constant_time_eq(credentials.trim(), expected)
}

/// Compare two strings without early exit on the first mismatch
/// 不在首个不同字节处提前返回的字符串比较
///
/// The loop always runs `max(left.len(), right.len())` iterations and folds
/// every difference into one accumulator, so timing leaks neither the matched
/// prefix nor (beyond the iteration count) the token length.
/// 循环固定执行 `max(left.len(), right.len())` 次并把所有差异累加到一个变量，
/// 因此耗时既不泄露匹配前缀，也不泄露 Token 长度（长度只影响循环次数）。
#[must_use]
pub fn constant_time_eq(left: &str, right: &str) -> bool {
    let left = left.as_bytes();
    let right = right.as_bytes();
    let mut difference = left.len() ^ right.len();
    let length = left.len().max(right.len());

    for index in 0..length {
        let a = left.get(index).copied().unwrap_or_default();
        let b = right.get(index).copied().unwrap_or_default();
        difference |= usize::from(a ^ b);
    }

    difference == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_time_eq_accepts_identical_strings() {
        assert!(constant_time_eq("secret", "secret"));
        assert!(constant_time_eq("", ""));
    }

    #[test]
    fn constant_time_eq_rejects_different_content() {
        assert!(!constant_time_eq("secret", "secreT"));
        assert!(!constant_time_eq("secret", "secreu"));
        assert!(!constant_time_eq("abc", "abd"));
    }

    #[test]
    fn constant_time_eq_rejects_different_lengths() {
        assert!(!constant_time_eq("secret", "secret "));
        assert!(!constant_time_eq("secret", "secre"));
        assert!(!constant_time_eq("", "secret"));
    }

    #[test]
    fn disabled_auth_accepts_every_request() {
        let auth = Auth::disabled();
        assert!(!auth.is_enabled());
        assert!(auth.authorize(None));
        assert!(auth.authorize(Some("Bearer anything")));
    }

    #[test]
    fn bearer_auth_requires_scheme_and_exact_token() {
        let auth = Auth::bearer("secret");
        assert!(auth.is_enabled());
        assert!(auth.authorize(Some("Bearer secret")));
        assert!(auth.authorize(Some("bearer secret")));
        assert!(auth.authorize(Some("BEARER secret")));
        assert!(auth.authorize(Some("Bearer  secret ")));
        assert!(!auth.authorize(None));
        assert!(!auth.authorize(Some("Bearer secre")));
        assert!(!auth.authorize(Some("Bearer secret2")));
        assert!(!auth.authorize(Some("Basic secret")));
        assert!(!auth.authorize(Some("secret")));
    }

    #[test]
    fn empty_token_disables_auth() {
        let auth = Auth::from_optional(Some(String::new()));
        assert!(!auth.is_enabled());
        assert!(auth.authorize(None));

        let options = HttpOptions::with_optional_token(Some(String::new()));
        assert!(!options.auth_enabled());
        assert!(!HttpOptions::new().auth_enabled());
        assert_eq!(HttpOptions::new(), HttpOptions::default());
        assert!(HttpOptions::with_token("token").auth_enabled());
    }

    #[test]
    fn debug_output_redacts_the_token() {
        let auth = Auth::bearer("super-secret");
        let rendered = format!("{auth:?}");
        assert!(rendered.contains("redacted"));
        assert!(!rendered.contains("super-secret"));
    }
}
