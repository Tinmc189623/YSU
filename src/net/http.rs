//! HTTP/1.1 的报文构造与解析。
//!
//! 只管字节层面的事：把请求拼成报文，把响应拆成状态行、头与正文。
//! 建连接、走 TLS、跟重定向都在 [`super::client`] 里。

use std::fmt;

/// 请求方法，目前只用到 GET。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    /// 取回资源。
    Get,
    /// 提交数据。
    Post,
    /// 只取头部。
    Head,
}

impl Method {
    /// 方法名。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Head => "HEAD",
        }
    }
}

/// 一个请求。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// 方法。
    pub method: Method,
    /// 目标地址，已经解析过，保证有主机名。
    pub url: url::Url,
    /// 附加的请求头。
    pub headers: Vec<(String, String)>,
    /// 请求体。
    pub body: Vec<u8>,
}

impl Request {
    /// 建一个 GET 请求。
    pub fn get(url: url::Url) -> Self {
        Self {
            method: Method::Get,
            url,
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    /// 追加一个请求头。
    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }

    /// 目标的端口，没写时按协议取默认值。
    pub fn port(&self) -> u16 {
        self.url
            .port_or_known_default()
            .unwrap_or(if self.url.scheme() == "https" {
                443
            } else {
                80
            })
    }

    /// 是否是 HTTPS。
    pub fn is_secure(&self) -> bool {
        self.url.scheme() == "https"
    }

    /// 主机名。
    pub fn host(&self) -> &str {
        self.url.host_str().unwrap_or("")
    }

    /// 请求目标：路径加查询串。
    pub fn target(&self) -> String {
        let path = self.url.path();
        match self.url.query() {
            Some(query) => format!("{path}?{query}"),
            None => {
                if path.is_empty() {
                    "/".to_string()
                } else {
                    path.to_string()
                }
            }
        }
    }

    /// 拼成待发送的报文。
    ///
    /// 主机名里带端口时 `Host` 要一并写上，虚拟主机才认得出来。
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = String::new();
        out.push_str(self.method.as_str());
        out.push(' ');
        out.push_str(&self.target());
        out.push_str(" HTTP/1.1\r\n");

        let host = self.host();
        let default_port = if self.is_secure() { 443 } else { 80 };
        if self.port() == default_port {
            out.push_str(&format!("Host: {host}\r\n"));
        } else {
            out.push_str(&format!("Host: {host}:{}\r\n", self.port()));
        }

        out.push_str("Connection: close\r\n");
        if self.method == Method::Post && !self.body.is_empty() {
            out.push_str(&format!("Content-Length: {}\r\n", self.body.len()));
        }
        for (name, value) in &self.headers {
            out.push_str(name);
            out.push_str(": ");
            out.push_str(value);
            out.push_str("\r\n");
        }
        out.push_str("\r\n");

        let mut bytes = out.into_bytes();
        bytes.extend_from_slice(&self.body);
        bytes
    }
}

/// 响应头部，正文还没读。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResponseHead {
    /// 状态码。
    pub status: u16,
    /// 状态说明，可能为空。
    pub reason: String,
    /// 头字段，名字保留原样，取值时按大小写不敏感处理。
    pub headers: Vec<(String, String)>,
}

impl ResponseHead {
    /// 取某个头的值，名字大小写不敏感。
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    /// 是否是重定向状态码。
    pub fn is_redirect(&self) -> bool {
        matches!(self.status, 301 | 302 | 303 | 307 | 308)
    }

    /// 正文的字节数，`Content-Length` 缺失或非法时返回 `None`。
    pub fn content_length(&self) -> Option<usize> {
        self.header("content-length")?.trim().parse().ok()
    }

    /// 是否使用分块传输。
    pub fn is_chunked(&self) -> bool {
        self.header("transfer-encoding")
            .is_some_and(|value| value.to_ascii_lowercase().contains("chunked"))
    }
}

/// 解析响应头，返回头部与正文起始位置。
///
/// 数据还不完整时返回 `None`，调用方应当继续读。
pub fn parse_head(data: &[u8]) -> Result<Option<(ResponseHead, usize)>, ProtocolError> {
    let Some(end) = find_header_end(data) else {
        return Ok(None);
    };
    let text = std::str::from_utf8(&data[..end])
        .map_err(|_| ProtocolError::new("响应头不是合法的 UTF-8"))?;
    let mut lines = text.split("\r\n");

    let status_line = lines
        .next()
        .ok_or_else(|| ProtocolError::new("响应缺少状态行"))?;
    let (status, reason) = parse_status_line(status_line)?;

    let mut headers = Vec::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let Some((name, value)) = line.split_once(':') else {
            return Err(ProtocolError::new(format!("响应头格式不对：{line}")));
        };
        headers.push((name.trim().to_string(), value.trim().to_string()));
    }

    Ok(Some((
        ResponseHead {
            status,
            reason,
            headers,
        },
        end + 4,
    )))
}

/// 找响应头结束的位置，也就是空行之前。
fn find_header_end(data: &[u8]) -> Option<usize> {
    data.windows(4).position(|window| window == b"\r\n\r\n")
}

/// 解析状态行。
fn parse_status_line(line: &str) -> Result<(u16, String), ProtocolError> {
    let mut parts = line.splitn(3, ' ');
    let version = parts.next().unwrap_or_default();
    if !version.starts_with("HTTP/1.") {
        return Err(ProtocolError::new(format!("不认识的状态行：{line}")));
    }
    let code = parts
        .next()
        .ok_or_else(|| ProtocolError::new("状态行里没有状态码"))?;
    let status = code
        .parse::<u16>()
        .map_err(|_| ProtocolError::new(format!("状态码不是数字：{code}")))?;
    let reason = parts.next().unwrap_or("").to_string();
    Ok((status, reason))
}

/// 按 `Content-Length` 判断正文是否读齐。
pub fn body_is_complete(head: &ResponseHead, body_bytes: usize) -> bool {
    match head.content_length() {
        Some(expected) => body_bytes >= expected,
        None => false,
    }
}

/// 解码分块传输的正文。
///
/// 输入是分块体本身（不含响应头）。数据不完整或格式有误时返回错误。
pub fn decode_chunked(data: &[u8]) -> Result<Vec<u8>, ProtocolError> {
    let mut out = Vec::new();
    let mut cursor = 0usize;

    loop {
        // 先读块长度那一行。
        let Some(line_end) = find_crlf(&data[cursor..]) else {
            return Err(ProtocolError::new("分块长度行不完整"));
        };
        let line = std::str::from_utf8(&data[cursor..cursor + line_end])
            .map_err(|_| ProtocolError::new("分块长度不是合法文本"))?;
        // 长度后面可能跟着 `;扩展`，只取分号之前的部分。
        let size_text = line.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_text, 16)
            .map_err(|_| ProtocolError::new(format!("分块长度不是十六进制：{size_text}")))?;
        cursor += line_end + 2;

        if size == 0 {
            // 结尾块之后是若干个尾随头，读到空行结束。
            return Ok(out);
        }
        if cursor + size > data.len() {
            return Err(ProtocolError::new("分块正文不完整"));
        }
        out.extend_from_slice(&data[cursor..cursor + size]);
        cursor += size;
        // 块数据后面必须跟一个换行。
        if data.len() < cursor + 2 {
            return Err(ProtocolError::new("分块结尾不完整"));
        }
        cursor += 2;
    }
}

/// 分块正文是否已经读到结尾。
///
/// 逐个块往前走，遇到零长度块就算读完。这样不必依赖服务器主动关闭连接，
/// 万一对方忽略了 `Connection: close` 也不会一直等下去。
pub fn chunked_is_complete(data: &[u8]) -> bool {
    let mut cursor = 0usize;
    loop {
        let Some(line_end) = find_crlf(&data[cursor..]) else {
            return false;
        };
        let Ok(line) = std::str::from_utf8(&data[cursor..cursor + line_end]) else {
            return false;
        };
        let size_text = line.split(';').next().unwrap_or("").trim();
        let Ok(size) = usize::from_str_radix(size_text, 16) else {
            return false;
        };
        cursor += line_end + 2;
        if size == 0 {
            return true;
        }
        cursor += size + 2;
        if cursor > data.len() {
            return false;
        }
    }
}

/// 找第一个回车换行的位置。
fn find_crlf(data: &[u8]) -> Option<usize> {
    data.windows(2).position(|window| window == b"\r\n")
}

/// 协议层面的错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtocolError {
    /// 说明。
    pub message: String,
}

impl ProtocolError {
    /// 构造错误。
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for ProtocolError {
    /// 直接输出说明。
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ProtocolError {}

#[cfg(test)]
mod tests {
    use super::*;

    /// 解析一个地址。
    fn url(text: &str) -> url::Url {
        url::Url::parse(text).expect("测试地址应当合法")
    }

    #[test]
    fn request_line_and_host_header() {
        let request = Request::get(url("http://example.com/a?b=1"));
        let text = String::from_utf8(request.to_bytes()).expect("是文本");
        assert!(text.starts_with("GET /a?b=1 HTTP/1.1\r\n"));
        assert!(text.contains("Host: example.com\r\n"));
        assert!(text.contains("Connection: close\r\n"));
        assert!(text.ends_with("\r\n\r\n"));
    }

    #[test]
    fn empty_path_becomes_root() {
        let request = Request::get(url("http://example.com"));
        assert_eq!(request.target(), "/");
    }

    #[test]
    fn non_default_port_is_written_into_host() {
        let request = Request::get(url("http://example.com:8080/x"));
        let text = String::from_utf8(request.to_bytes()).expect("是文本");
        assert!(text.contains("Host: example.com:8080\r\n"), "{text}");
    }

    #[test]
    fn https_default_port_is_omitted() {
        let request = Request::get(url("https://example.com/"));
        let text = String::from_utf8(request.to_bytes()).expect("是文本");
        assert!(text.contains("Host: example.com\r\n"), "{text}");
        assert!(request.is_secure());
        assert_eq!(request.port(), 443);
    }

    #[test]
    fn post_carries_content_length() {
        let mut request = Request::get(url("http://example.com/"));
        request.method = Method::Post;
        request.body = b"hello".to_vec();
        let text = String::from_utf8(request.to_bytes()).expect("是文本");
        assert!(text.contains("Content-Length: 5\r\n"));
        assert!(text.ends_with("hello"));
    }

    #[test]
    fn extra_headers_are_written() {
        let request = Request::get(url("http://example.com/"))
            .header("Accept", "text/html")
            .header("User-Agent", "YSU");
        let text = String::from_utf8(request.to_bytes()).expect("是文本");
        assert!(text.contains("Accept: text/html\r\n"));
        assert!(text.contains("User-Agent: YSU\r\n"));
    }

    #[test]
    fn parse_simple_response() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 5\r\n\r\nhello";
        let (head, start) = parse_head(raw).expect("应当解析成功").expect("数据完整");
        assert_eq!(head.status, 200);
        assert_eq!(head.reason, "OK");
        assert_eq!(head.header("content-type"), Some("text/html"));
        assert_eq!(head.content_length(), Some(5));
        assert_eq!(&raw[start..], b"hello");
    }

    #[test]
    fn header_lookup_is_case_insensitive() {
        let raw = b"HTTP/1.1 200 OK\r\nCONTENT-TYPE: text/plain\r\n\r\n";
        let (head, _) = parse_head(raw).expect("解析成功").expect("数据完整");
        assert_eq!(head.header("Content-Type"), Some("text/plain"));
    }

    #[test]
    fn incomplete_head_returns_none() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n";
        assert!(parse_head(raw).expect("不算错误").is_none());
    }

    #[test]
    fn malformed_status_line_is_an_error() {
        let raw = b"NOT-HTTP\r\n\r\n";
        assert!(parse_head(raw).is_err());
    }

    #[test]
    fn malformed_header_is_an_error() {
        let raw = "HTTP/1.1 200 OK\r\n没有冒号\r\n\r\n".as_bytes();
        assert!(parse_head(raw).is_err());
    }

    #[test]
    fn redirect_is_recognised() {
        let raw = b"HTTP/1.1 301 Moved Permanently\r\nLocation: /new\r\n\r\n";
        let (head, _) = parse_head(raw).expect("解析成功").expect("数据完整");
        assert!(head.is_redirect());
        assert_eq!(head.header("location"), Some("/new"));
    }

    #[test]
    fn chunked_body_is_decoded() {
        let raw = b"5\r\nhello\r\n1\r\n \r\n5\r\nworld\r\n0\r\n\r\n";
        let body = decode_chunked(raw).expect("应当解码成功");
        assert_eq!(body, b"hello world");
    }

    #[test]
    fn chunked_with_extension_is_decoded() {
        let raw = b"5;name=value\r\nhello\r\n0\r\n\r\n";
        assert_eq!(decode_chunked(raw).expect("解码成功"), b"hello");
    }

    #[test]
    fn chunked_with_incomplete_data_is_an_error() {
        let raw = b"5\r\nhel";
        assert!(decode_chunked(raw).is_err());
    }

    #[test]
    fn chunked_with_bad_length_is_an_error() {
        let raw = b"zz\r\nhello\r\n0\r\n\r\n";
        assert!(decode_chunked(raw).is_err());
    }

    #[test]
    fn chunked_completeness_detection() {
        assert!(chunked_is_complete(b"5\r\nhello\r\n0\r\n\r\n"));
        assert!(!chunked_is_complete(b"5\r\nhel"));
        assert!(!chunked_is_complete(b"5\r\nhello\r\n"));
        assert!(!chunked_is_complete(b""));
    }

    #[test]
    fn chunked_detection() {
        let raw = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n";
        let (head, _) = parse_head(raw).expect("解析成功").expect("数据完整");
        assert!(head.is_chunked());
    }

    #[test]
    fn body_completeness_by_content_length() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\n";
        let (head, _) = parse_head(raw).expect("解析成功").expect("数据完整");
        assert!(!body_is_complete(&head, 4));
        assert!(body_is_complete(&head, 5));
    }

    #[test]
    fn status_without_reason_is_accepted() {
        let (head, _) = parse_head(b"HTTP/1.1 204 \r\n\r\n")
            .expect("解析成功")
            .expect("数据完整");
        assert_eq!(head.status, 204);
        assert!(head.reason.is_empty());
    }
}
