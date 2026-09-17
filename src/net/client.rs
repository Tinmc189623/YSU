//! HTTP 客户端。
//!
//! 每次请求新建一条连接，用 `Connection: close` 告诉服务器发完就关。
//! 连接池留到有性能需求时再做，现在这样语义最简单。
//!
//! TLS 交给 rustls，根证书用 webpki-roots；这两件事不适合自己写。

use super::http::{
    ProtocolError, Request, ResponseHead, chunked_is_complete, decode_chunked, parse_head,
};
use std::fmt;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Duration;
use url::Url;

/// 读取响应时单次读取的缓冲区大小。
const READ_CHUNK: usize = 16 * 1024;
/// 响应头的大小上限，防止对端一直发头把内存撑爆。
const MAX_HEAD_SIZE: usize = 64 * 1024;

/// 网络层错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetError {
    /// 地址不合法或协议不支持。
    InvalidUrl(String),
    /// 建立连接失败。
    Connect(String),
    /// TLS 握手或加密层出错。
    Tls(String),
    /// 读写超时。
    Timeout,
    /// 对端返回的报文不符合协议。
    Protocol(String),
    /// 重定向次数超出上限。
    TooManyRedirects,
    /// 响应体超过大小上限。
    TooLarge,
    /// 其他输入输出错误。
    Io(String),
}

impl fmt::Display for NetError {
    /// 输出人类可读的说明。
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidUrl(text) => write!(f, "地址不合法：{text}"),
            Self::Connect(text) => write!(f, "连接失败：{text}"),
            Self::Tls(text) => write!(f, "安全连接失败：{text}"),
            Self::Timeout => f.write_str("请求超时"),
            Self::Protocol(text) => write!(f, "响应不符合协议：{text}"),
            Self::TooManyRedirects => f.write_str("重定向次数过多"),
            Self::TooLarge => f.write_str("响应内容过大"),
            Self::Io(text) => write!(f, "网络读写失败：{text}"),
        }
    }
}

impl std::error::Error for NetError {}

impl From<ProtocolError> for NetError {
    /// 协议错误直接转换过来。
    fn from(error: ProtocolError) -> Self {
        Self::Protocol(error.message)
    }
}

/// 一次请求的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    /// 状态码。
    pub status: u16,
    /// 状态说明。
    pub reason: String,
    /// 响应头。
    pub headers: Vec<(String, String)>,
    /// 响应体原始字节，字符集解码交给上层。
    pub body: Vec<u8>,
    /// 最终地址，跟过重定向之后的值。
    pub url: Url,
    /// 经过的重定向次数。
    pub redirects: usize,
}

impl Response {
    /// 取某个响应头。
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    /// 响应是否成功。
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// 按 `Content-Type` 判断是否是 HTML。
    ///
    /// 没写 `Content-Type` 时按 HTML 处理，这是浏览器的惯例。
    pub fn is_html(&self) -> bool {
        match self.header("content-type") {
            Some(value) => {
                let lower = value.to_ascii_lowercase();
                lower.contains("text/html")
                    || lower.contains("application/xhtml")
                    || lower.contains("text/plain")
            }
            None => true,
        }
    }
}

/// 一条可以读写的连接。
enum Stream {
    /// 明文连接。
    Plain(TcpStream),
    /// 加密连接。
    Tls(Box<rustls::StreamOwned<rustls::ClientConnection, TcpStream>>),
}

impl Read for Stream {
    /// 从连接里读数据。
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::Plain(stream) => stream.read(buffer),
            Self::Tls(stream) => stream.read(buffer),
        }
    }
}

impl Write for Stream {
    /// 往连接里写数据。
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        match self {
            Self::Plain(stream) => stream.write(buffer),
            Self::Tls(stream) => stream.write(buffer),
        }
    }

    /// 冲刷缓冲区。
    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Self::Plain(stream) => stream.flush(),
            Self::Tls(stream) => stream.flush(),
        }
    }
}

/// HTTP 客户端。
pub struct Client {
    /// TLS 配置，建一次就够，握手时复用。
    tls: Arc<rustls::ClientConfig>,
    /// 发出去的 User-Agent。
    user_agent: String,
    /// 单次读写的超时。
    timeout: Duration,
    /// 最多跟几次重定向。
    max_redirects: usize,
    /// 响应体的大小上限。
    max_body: usize,
}

impl Default for Client {
    /// 默认配置。
    fn default() -> Self {
        Self::new()
    }
}

impl Client {
    /// 建一个客户端。
    pub fn new() -> Self {
        let mut roots = rustls::RootCertStore::empty();
        roots
            .roots
            .extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let config = rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();

        Self {
            tls: Arc::new(config),
            user_agent: format!(
                "Vexo/{} YSU/{}",
                env!("CARGO_PKG_VERSION"),
                env!("CARGO_PKG_VERSION")
            ),
            timeout: Duration::from_secs(20),
            max_redirects: 10,
            max_body: 16 * 1024 * 1024,
        }
    }

    /// 设置 User-Agent。
    pub fn set_user_agent(&mut self, user_agent: impl Into<String>) {
        self.user_agent = user_agent.into();
    }

    /// 设置超时。
    pub fn set_timeout(&mut self, timeout: Duration) {
        self.timeout = timeout;
    }

    /// 取回一个地址。
    pub fn fetch(&self, url: &str) -> Result<Response, NetError> {
        let mut current =
            Url::parse(url).map_err(|error| NetError::InvalidUrl(error.to_string()))?;

        for hop in 0..=self.max_redirects {
            let (head, body) = self.fetch_once(&current)?;
            if head.is_redirect()
                && let Some(location) = head.header("location")
            {
                let next = current
                    .join(location)
                    .map_err(|error| NetError::InvalidUrl(error.to_string()))?;
                current = next;
                continue;
            }
            return Ok(Response {
                status: head.status,
                reason: head.reason,
                headers: head.headers,
                body,
                url: current,
                redirects: hop,
            });
        }
        Err(NetError::TooManyRedirects)
    }

    /// 请求一次，不做重定向处理。
    fn fetch_once(&self, url: &Url) -> Result<(ResponseHead, Vec<u8>), NetError> {
        let scheme = url.scheme();
        if scheme != "http" && scheme != "https" {
            return Err(NetError::InvalidUrl(format!("不支持的协议：{scheme}")));
        }
        if url.host_str().is_none() {
            return Err(NetError::InvalidUrl("地址里没有主机名".to_string()));
        }

        let request = Request::get(url.clone())
            .header("User-Agent", &self.user_agent)
            .header(
                "Accept",
                "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
            )
            .header("Accept-Language", "zh-CN,zh;q=0.9,en;q=0.8");

        let mut stream = self.connect(url)?;

        let bytes = request.to_bytes();
        stream
            .write_all(&bytes)
            .map_err(|error| classify_io(&error))?;
        stream.flush().map_err(|error| classify_io(&error))?;

        read_response(&mut stream, self.max_body)
    }

    /// 建立到目标主机的连接，HTTPS 时套上 TLS。
    fn connect(&self, url: &Url) -> Result<Stream, NetError> {
        let host = url
            .host_str()
            .ok_or_else(|| NetError::InvalidUrl("地址里没有主机名".to_string()))?;
        let port = url
            .port_or_known_default()
            .ok_or_else(|| NetError::InvalidUrl("地址里没有端口".to_string()))?;

        let socket = TcpStream::connect((host, port))
            .map_err(|error| NetError::Connect(format!("{host}:{port} —— {error}")))?;
        socket
            .set_read_timeout(Some(self.timeout))
            .map_err(|error| classify_io(&error))?;
        socket
            .set_write_timeout(Some(self.timeout))
            .map_err(|error| classify_io(&error))?;
        // 关掉 Nagle，浏览器发的小请求不需要攒包。
        let _ = socket.set_nodelay(true);

        if url.scheme() != "https" {
            return Ok(Stream::Plain(socket));
        }

        let server_name = rustls::pki_types::ServerName::try_from(host.to_string())
            .map_err(|error| NetError::Tls(format!("主机名不能用于 TLS：{error}")))?;
        let connection = rustls::ClientConnection::new(Arc::clone(&self.tls), server_name)
            .map_err(|error| NetError::Tls(error.to_string()))?;
        Ok(Stream::Tls(Box::new(rustls::StreamOwned::new(
            connection, socket,
        ))))
    }
}

/// 从连接里把响应头和正文读完。
fn read_response(
    stream: &mut Stream,
    max_body: usize,
) -> Result<(ResponseHead, Vec<u8>), NetError> {
    let mut buffer: Vec<u8> = Vec::new();
    let mut chunk = vec![0u8; READ_CHUNK];
    let mut head: Option<(ResponseHead, usize)> = None;

    loop {
        if let Some((head, start)) = &head {
            let available = buffer.len() - start;
            let done = if head.is_chunked() {
                chunked_is_complete(&buffer[*start..])
            } else {
                head.content_length().is_some_and(|len| available >= len)
            };
            if done {
                break;
            }
        }

        match stream.read(&mut chunk) {
            // 对端关闭说明正文到头了，没有长度信息时就是靠它界定结尾。
            Ok(0) => break,
            Ok(count) => buffer.extend_from_slice(&chunk[..count]),
            Err(error) => return Err(classify_io(&error)),
        }

        if buffer.len() > max_body {
            return Err(NetError::TooLarge);
        }
        if head.is_none() {
            head = parse_head(&buffer)?;
            // 响应头本身就不该有这么大，多半是异常流量。
            if head
                .as_ref()
                .is_some_and(|(_, start)| *start > MAX_HEAD_SIZE)
            {
                return Err(NetError::TooLarge);
            }
        }
    }

    let Some((head, start)) = head else {
        return Err(NetError::Protocol("响应头不完整".to_string()));
    };
    let raw = &buffer[start.min(buffer.len())..];

    // 这几个状态码按规范没有正文。
    let body = if head.status == 204 || head.status == 304 || (100..200).contains(&head.status) {
        Vec::new()
    } else if head.is_chunked() {
        decode_chunked(raw)?
    } else if let Some(length) = head.content_length() {
        raw[..length.min(raw.len())].to_vec()
    } else {
        // 既没有长度也没分块，靠连接关闭来界定正文，读到多少算多少。
        raw.to_vec()
    };

    Ok((head, body))
}

/// 把输入输出错误归类。
fn classify_io(error: &std::io::Error) -> NetError {
    match error.kind() {
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => NetError::Timeout,
        _ => {
            let text = error.to_string();
            // rustls 的错误会以 io 错误的形式冒上来，里面带 TLS 字样。
            if text.contains("TLS") || text.contains("certificate") {
                NetError::Tls(text)
            } else {
                NetError::Io(text)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    /// 起一个只回一次响应的假服务器，返回它的端口。
    ///
    /// 这是真开一个监听端口的，客户端走的是真实的 socket 路径。
    fn fake_server(response: &'static [u8], close_early: bool) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").expect("应当能监听本地端口");
        let port = listener.local_addr().expect("有本地地址").port();
        std::thread::spawn(move || {
            if let Ok((mut socket, _)) = listener.accept() {
                // 先把请求读完再回，免得对端还没写完就被关掉。
                let mut request = [0u8; 4096];
                let _ = socket.read(&mut request);
                if !close_early {
                    let _ = socket.write_all(response);
                }
                let _ = socket.flush();
            }
        });
        port
    }

    #[test]
    fn fetches_a_simple_response() {
        let port = fake_server(
            b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 11\r\n\r\nhello world",
            false,
        );
        let client = Client::new();
        let response = client
            .fetch(&format!("http://127.0.0.1:{port}/"))
            .expect("应当取回响应");
        assert_eq!(response.status, 200);
        assert_eq!(response.body, b"hello world");
        assert!(response.is_success());
        assert!(response.is_html());
        assert_eq!(response.redirects, 0);
    }

    #[test]
    fn decodes_chunked_response() {
        let port = fake_server(
            b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nTransfer-Encoding: chunked\r\n\r\n\
              5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n",
            false,
        );
        let client = Client::new();
        let response = client
            .fetch(&format!("http://127.0.0.1:{port}/"))
            .expect("应当取回响应");
        assert_eq!(response.body, b"hello world");
    }

    #[test]
    fn follows_redirects() {
        // 两个端口：一个回重定向，一个回正文。
        let target = fake_server(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\ndone", false);
        let redirect: &'static [u8] = Box::leak(
            format!(
                "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{target}/final\r\nContent-Length: 0\r\n\r\n"
            )
            .into_bytes()
            .into_boxed_slice(),
        );
        let entry = fake_server(redirect, false);

        let client = Client::new();
        let response = client
            .fetch(&format!("http://127.0.0.1:{entry}/start"))
            .expect("应当跟到最终地址");
        assert_eq!(response.body, b"done");
        assert_eq!(response.redirects, 1);
        assert!(response.url.path().ends_with("/final"));
    }

    #[test]
    fn reports_http_error_status() {
        let port = fake_server(
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 9\r\n\r\nnot found",
            false,
        );
        let client = Client::new();
        let response = client
            .fetch(&format!("http://127.0.0.1:{port}/missing"))
            .expect("404 也是一次成功的请求");
        assert_eq!(response.status, 404);
        assert!(!response.is_success());
    }

    #[test]
    fn connection_closed_early_is_an_error() {
        let port = fake_server(b"", true);
        let mut client = Client::new();
        client.set_timeout(Duration::from_millis(500));
        assert!(client.fetch(&format!("http://127.0.0.1:{port}/")).is_err());
    }

    #[test]
    fn rejects_unsupported_scheme() {
        let client = Client::new();
        let error = client.fetch("file:///etc/hostname").expect_err("应当拒绝");
        assert!(matches!(error, NetError::InvalidUrl(_)), "实际是 {error:?}");
    }

    #[test]
    fn rejects_malformed_url() {
        let client = Client::new();
        assert!(client.fetch("这不是地址").is_err());
    }

    #[test]
    fn connection_refused_is_reported() {
        // 本地一个几乎不可能被占用的端口。
        let mut client = Client::new();
        client.set_timeout(Duration::from_millis(300));
        let error = client.fetch("http://127.0.0.1:9/").expect_err("应当连不上");
        assert!(
            matches!(error, NetError::Connect(_) | NetError::Io(_)),
            "实际是 {error:?}"
        );
    }

    #[test]
    fn response_without_content_length_reads_until_close() {
        let port = fake_server(
            b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\n\r\nno length here",
            false,
        );
        let client = Client::new();
        let response = client
            .fetch(&format!("http://127.0.0.1:{port}/"))
            .expect("应当取回响应");
        assert_eq!(response.body, b"no length here");
    }

    #[test]
    fn head_size_limit_is_enforced() {
        let mut huge = Vec::from(&b"HTTP/1.1 200 OK\r\n"[..]);
        for index in 0..20000 {
            huge.extend_from_slice(format!("X-Pad-{index}: value\r\n").as_bytes());
        }
        huge.extend_from_slice(b"\r\n");
        let leaked: &'static [u8] = Box::leak(huge.into_boxed_slice());
        let port = fake_server(leaked, false);

        let client = Client::new();
        let result = client.fetch(&format!("http://127.0.0.1:{port}/"));
        assert!(result.is_err(), "超大的响应头应当被拒绝");
    }
}
