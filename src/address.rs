//! 地址的处理：地址栏里的一行输入该变成什么，以及页面里的相对地址怎么补全。
//!
//! 这两件事都属于网页平台的行为，不属于界面行为。放在外壳里的话，换一套外壳
//! 就得重写一遍，而且两套写法迟早会有出入——「什么是文件路径」这种判断尤其
//! 容易各写各的。
//!
//! 内核不认识「自己提供的页面」这种说法：它只解析它拿到的文档，不生产文档。
//! 起始页、空白页、出错提示这些都是外壳的事，外壳把内容交给内核渲染。

use url::Url;

/// 地址栏里的一行输入最终要打开什么。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// 本地文件，值是 `file://` 地址。
    File(String),
    /// 网络地址。
    Remote(String),
}

impl Target {
    /// 对应的规范地址。
    pub fn url(&self) -> &str {
        match self {
            Self::File(url) | Self::Remote(url) => url,
        }
    }

    /// 是不是本地文件。
    pub fn is_file(&self) -> bool {
        matches!(self, Self::File(_))
    }
}

/// 把地址栏里的一行输入解析成要打开的目标。
///
/// 认不出来时返回原因，调用方把它显示给用户——比默默拼一个打不开的地址强。
pub fn resolve_input(input: &str) -> Result<Target, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("没有输入地址".to_string());
    }

    // 文件路径只认带前缀的写法。裸文件名不当路径：`example.com` 与
    // `index.html` 在这个位置上长得一样，按主机名处理是浏览器的惯例，
    // 猜错了更让人困惑。
    if looks_like_path(trimmed) {
        let url = file_url_from_input(trimmed)?;
        return Ok(Target::File(url));
    }

    Ok(Target::Remote(normalize(trimmed)))
}

/// 判断一段输入是不是文件路径。
pub fn looks_like_path(input: &str) -> bool {
    let trimmed = input.trim();
    trimmed == "~"
        || trimmed.starts_with("file://")
        || trimmed.starts_with('/')
        || trimmed.starts_with("./")
        || trimmed.starts_with("../")
        || trimmed.starts_with("~/")
}

/// 把一种路径写法补成 `file://` 地址。
///
/// 识别 `file://` 地址、绝对路径、`./` 与 `../` 开头的相对路径，以及 `~`
/// 开头的家目录路径。
pub fn file_url_from_input(input: &str) -> Result<String, String> {
    let trimmed = input.trim();

    if trimmed.starts_with("file://") {
        // 已经是文件地址了，只做一次规范化，把 `..` 之类抹平。
        let parsed = Url::parse(trimmed).map_err(|error| format!("文件地址写得不合法：{error}"))?;
        return Ok(parsed.to_string());
    }

    let path = if trimmed == "~" || trimmed.starts_with("~/") {
        let home =
            std::env::var_os("HOME").ok_or_else(|| "读不到家目录，用绝对路径试试".to_string())?;
        let rest = trimmed.strip_prefix("~/").unwrap_or("");
        std::path::PathBuf::from(home).join(rest)
    } else {
        std::path::PathBuf::from(trimmed)
    };

    // `absolute` 只按当前目录补全，不碰文件系统，所以文件不存在也能算出路径，
    // 读盘时再报「找不到」，两种错误分得开。
    let absolute =
        std::path::absolute(&path).map_err(|error| format!("补不出绝对路径：{error}"))?;

    Url::from_file_path(&absolute)
        .map(|url| url.to_string())
        .map_err(|()| "这个路径转不成文件地址，可能是写法不对".to_string())
}

/// 把 `file://` 地址还原成磁盘路径。
pub fn path_from_file_url(url: &str) -> Option<std::path::PathBuf> {
    Url::parse(url.trim()).ok()?.to_file_path().ok()
}

/// 给没写协议的地址补上 `http://`。
///
/// 搜索功能还没接，所以没有「当成搜索词」这一档；一律按主机名处理，让错误页
/// 去说明取不回来。
pub fn normalize(input: &str) -> String {
    let trimmed = input.trim();
    if trimmed.contains("://") {
        return trimmed.to_string();
    }
    format!("http://{trimmed}")
}

/// 按页面地址补全一个相对地址。
///
/// 页面里的链接写的是相对地址时，得知道自己现在在哪儿才补得出来。内核在加载
/// 文档时已经把基地址记下来了，所以这件事该在这里做，不该让外壳自己拿当前
/// 地址去拼。
pub fn resolve_link(base: &str, href: &str) -> String {
    let href = href.trim();
    // 纯片段（`#anchor`）与 `javascript:` 这类不指向新文档的，原样交回去，
    // 由调用方决定怎么处理。
    if href.is_empty() || href.starts_with('#') {
        return href.to_string();
    }
    match Url::parse(base) {
        Ok(base) => base
            .join(href)
            .map_or_else(|_| href.to_string(), |url| url.to_string()),
        Err(_) => href.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_is_reported_not_guessed() {
        // 空地址没什么可解析的。给个起始页是外壳的选择，不是内核的——
        // 内核不生产文档，只解析递给它的那一份。
        assert!(resolve_input("").is_err());
        assert!(resolve_input("   ").is_err());
    }

    #[test]
    fn path_shapes_are_recognised() {
        for input in [
            "file:///tmp/a.html",
            "/tmp/a.html",
            "./a.html",
            "../a.html",
            "~/a.html",
            "~",
        ] {
            assert!(looks_like_path(input), "{input} 应当被当成路径");
        }
        for input in ["example.com", "index.html", "about:home", "https://x/", ""] {
            assert!(!looks_like_path(input), "{input} 不该被当成路径");
        }
    }

    #[test]
    fn absolute_path_becomes_a_file_url() {
        let target = resolve_input("/tmp/a.html").expect("应当认得");
        assert_eq!(target, Target::File("file:///tmp/a.html".to_string()));
        assert!(target.is_file());
    }

    #[test]
    fn relative_path_is_made_absolute() {
        let target = resolve_input("./page.html").expect("应当认得");
        assert!(
            target.url().starts_with("file:///"),
            "实际：{}",
            target.url()
        );
        assert!(target.url().ends_with("/page.html"));
    }

    #[test]
    fn home_directory_is_expanded() {
        let target = resolve_input("~/a.html").expect("应当认得");
        let home = std::env::var("HOME").expect("测试环境有家目录");
        assert!(target.url().contains(&home), "实际：{}", target.url());
    }

    #[test]
    fn non_ascii_paths_are_encoded() {
        // 中文路径在地址里要按百分号编码，还原回来还是原来的路径。
        let target = resolve_input("/tmp/中文/页面.html").expect("应当认得");
        assert!(!target.url().contains('页'), "地址里不该有未编码的中文");
        let path = path_from_file_url(target.url()).expect("应当还原成路径");
        assert!(path.ends_with("页面.html"), "实际：{}", path.display());
    }

    #[test]
    fn bare_host_gets_a_scheme() {
        assert_eq!(
            resolve_input("example.com").expect("应当认得"),
            Target::Remote("http://example.com".to_string())
        );
    }

    #[test]
    fn explicit_scheme_is_kept() {
        assert_eq!(
            resolve_input("https://example.com/a").expect("应当认得"),
            Target::Remote("https://example.com/a".to_string())
        );
    }

    #[test]
    fn relative_links_resolve_against_the_page() {
        assert_eq!(
            resolve_link("https://example.com/dir/page.html", "next.html"),
            "https://example.com/dir/next.html"
        );
        assert_eq!(
            resolve_link("https://example.com/dir/page.html", "/root.html"),
            "https://example.com/root.html"
        );
        assert_eq!(
            resolve_link("https://example.com/dir/page.html", "https://other/x"),
            "https://other/x"
        );
    }

    #[test]
    fn local_pages_resolve_their_links_too() {
        // 本地文件之间的相对链接要按文件地址补全，这条路上文件与目录的关系
        // 和网址一样。
        assert_eq!(
            resolve_link("file:///tmp/demo/page.html", "style.css"),
            "file:///tmp/demo/style.css"
        );
    }

    #[test]
    fn pure_fragments_are_left_alone() {
        assert_eq!(resolve_link("https://example.com/", "#top"), "#top");
        assert_eq!(resolve_link("https://example.com/", ""), "");
    }

    #[test]
    fn unusable_base_falls_back_to_the_raw_link() {
        assert_eq!(resolve_link("", "next.html"), "next.html");
    }
}
