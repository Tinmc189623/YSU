//! 从 `version.toml` 取内核版本号，编译期嵌进程序。
//!
//! 版本号是构建期就定死的东西，所以在这里读一次、经 `rustc-env` 传进去，
//! 源码里就只剩一个 `env!`——运行期不必再去碰文件。
//!
//! 清单可能在两处：工作区里各 crate 共用仓库根那一份，crate 被单独拿出去
//! 发布时则在自己根目录。两处都找，谁先找到用谁。
//!
//! 两处都没有就退回 Cargo 里的版本号，让程序还能起来，而不是因为少一个
//! 文本文件编译不过。

use std::path::Path;

/// 清单里给内核的那一项。
const KEY: &str = "kernel_version";

/// 传进去的环境变量名，源码那边用 `env!` 取。
const ENV_NAME: &str = "YSU_KERNEL_VERSION";

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    // 先看 crate 自己根目录，再看仓库根。两处都盯着，改动任一都要重编。
    let candidates = [root.join("version.toml"), root.join("../../version.toml")];
    for candidate in &candidates {
        println!("cargo:rerun-if-changed={}", candidate.display());
    }

    let version = candidates
        .iter()
        .find_map(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| value_of(&text, KEY))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| std::env::var("CARGO_PKG_VERSION").unwrap_or_default());

    println!("cargo:rustc-env={ENV_NAME}={version}");
}

/// 从清单的 `[version]` 段里取某个键的值，值两边的引号去掉。
///
/// 清单是人手写的，结构就一个 `[version]` 段加几个键，所以不引 TOML 解析库。
/// 键要整段相等，不能只比前缀——`kernel_version` 与 `browser_version` 都以
/// `version` 结尾，按前缀找会认错。
fn value_of(text: &str, key: &str) -> Option<String> {
    let mut in_version_section = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        // 段头，形如 `[version]`。
        if let Some(name) = trimmed
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
        {
            in_version_section = name.trim() == "version";
            continue;
        }
        if !in_version_section {
            continue;
        }
        let Some((name, value)) = trimmed.split_once('=') else {
            continue;
        };
        if name.trim() == key {
            return Some(value.trim().trim_matches('"').to_string());
        }
    }
    None
}
