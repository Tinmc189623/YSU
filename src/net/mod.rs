//! 网络：HTTP 与 HTTPS 客户端。

pub mod client;
pub mod encoding;
pub mod http;

pub use client::{Client, NetError, Response};
pub use encoding::{decode, sniff};
pub use http::{Method, Request, ResponseHead};
