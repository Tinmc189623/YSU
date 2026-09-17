//! 对真实站点发一次请求，验证 TLS 与 HTTP 客户端。
//!
//! 运行：`cargo run -p ysu --example fetch_probe -- https://example.com/`

use ysu::net::Client;

fn main() {
    let url = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "https://example.com/".to_string());
    let client = Client::new();
    match client.fetch(&url) {
        Ok(response) => {
            println!("地址 {}", response.url);
            println!("状态 {} {}", response.status, response.reason);
            println!("重定向 {} 次", response.redirects);
            println!("正文 {} 字节", response.body.len());
            if let Some(kind) = response.header("content-type") {
                println!("类型 {kind}");
            }
            let head: String = String::from_utf8_lossy(&response.body)
                .chars()
                .take(240)
                .collect();
            println!("开头：{}", head.replace('\n', " "));
        }
        Err(error) => {
            eprintln!("请求失败：{error}");
            std::process::exit(1);
        }
    }
}
