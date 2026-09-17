//! 用 html5lib-tests 的树构建用例核对 YSU 的 HTML 解析器。
//!
//! 用例放在 `tests/html5lib/tree-construction` 下，来源与许可见那里的 README。
//! 这套用例是各浏览器与 Servo 共用的那一份，不是为本项目定制的——拿它当尺子，
//! 「通过」这个词才有外人能核对的含义。
//!
//! 用例格式见同目录的 `README.md`：每段以 `#data` 开头，`#errors` 后面是期望的
//! 错误条数，`#document` 后面是按缩进写的期望树。这里只比对树，不比对错误条数
//! ——解析错误的计数是另一件事，混在一起会让失败原因看不清。
//!
//! 片段解析的用例也一起跑，不单独排除。把它们挡在外面会让通过率虚高，
//! 那种数字没有意义。

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use ysu::dom::node::{Document, Namespace, NodeData, NodeId};
use ysu::html::{Attribute, parse_document, parse_fragment};

/// 一条用例。
#[derive(Debug)]
struct Case {
    /// 出错时用来定位：文件名加段号。
    label: String,
    /// 要解析的 HTML。
    data: String,
    /// 期望的树，按行拆好。
    expected: Vec<String>,
    /// 片段解析的上下文元素，`None` 表示整篇文档。
    context: Option<String>,
}

/// 把一份 `.dat` 拆成用例。
///
/// 按行扫而不是按空行切：用例的数据里本来就可能有空行，用分隔符切会把它们
/// 切碎，切错的代价是整段用例静默消失。
fn parse_cases(label: &str, text: &str) -> Vec<Case> {
    let lines: Vec<&str> = text.lines().collect();
    let mut cases = Vec::new();
    let mut index = 0;

    while index < lines.len() {
        if lines[index] != "#data" {
            index += 1;
            continue;
        }
        index += 1;

        let mut data = Vec::new();
        while index < lines.len() && lines[index] != "#errors" {
            data.push(lines[index]);
            index += 1;
        }
        // 数据末尾那个换行不算内容。
        let data = data.join("\n");

        // `#errors` 与可选的 `#new-errors` 都跳过，这一段只管树。
        let mut context = None;
        while index < lines.len() && lines[index] != "#document" {
            if lines[index] == "#document-fragment" {
                index += 1;
                context = lines.get(index).map(|line| (*line).to_string());
            }
            index += 1;
        }
        index += 1; // 越过 `#document`

        let mut expected = Vec::new();
        while index < lines.len() && !lines[index].is_empty() {
            // 用例之间靠空行隔开，但数据里也可能有空行；只有遇到下一个
            // `#data` 才算这条结束。
            if lines[index] == "#data" {
                break;
            }
            expected.push(lines[index].to_string());
            index += 1;
        }

        cases.push(Case {
            label: format!("{label}#{}", cases.len() + 1),
            data,
            expected,
            context,
        });
    }

    cases
}

/// 按 html5lib 的格式打印一棵文档树。
fn dump(document: &Document) -> Vec<String> {
    let mut lines = Vec::new();
    for &child in document.children(document.root()) {
        dump_node(document, child, 0, &mut lines);
    }
    lines
}

/// 递归打印一个节点。
///
/// 属性的缩进比它所属的元素深一层，跟子节点一样——这是用例格式的规定，
/// 不是随便定的：属性按名字排好序之后，看起来就是元素的头一批“子节点”。
fn dump_node(document: &Document, id: NodeId, depth: usize, out: &mut Vec<String>) {
    let node = document.node(id);
    let indent = format!("| {}", "  ".repeat(depth));

    match &node.data {
        NodeData::Document => {}
        NodeData::Text(text) => out.push(format!("{indent}\"{text}\"")),
        NodeData::Comment(text) => out.push(format!("{indent}<!-- {text} -->")),
        NodeData::Doctype(doctype) => {
            let public = doctype.public_id.as_deref().unwrap_or("");
            let system = doctype.system_id.as_deref().unwrap_or("");
            let mut line = format!("{indent}<!DOCTYPE {}", doctype.name);
            if !public.is_empty() || !system.is_empty() {
                let _ = write!(line, " \"{public}\" \"{system}\"");
            }
            line.push('>');
            out.push(line);
        }
        NodeData::Element(element) => {
            let prefix = match element.namespace {
                Namespace::Html => "",
                Namespace::Svg => "svg ",
                Namespace::MathMl => "math ",
            };
            out.push(format!("{indent}<{prefix}{}>", element.name));

            let inner = format!("| {}", "  ".repeat(depth + 1));
            let mut attributes: Vec<&Attribute> = element.attributes.iter().collect();
            attributes.sort_by(|left, right| left.name.cmp(&right.name));
            for attribute in attributes {
                out.push(format!("{inner}{}=\"{}\"", attribute.name, attribute.value));
            }

            for &child in &node.children {
                dump_node(document, child, depth + 1, out);
            }
        }
    }
}

/// 列出目录里全部 `.dat` 文件，按名字排序。
fn dat_files(root: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(root)
        .expect("读得到用例目录")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|extension| extension == "dat"))
        .collect();
    files.sort();
    files
}

/// 两份树之间的第一处不同，用来写失败原因。
fn first_difference(expected: &[String], actual: &[String]) -> String {
    let limit = expected.len().max(actual.len());
    for index in 0..limit {
        let want = expected.get(index).map(String::as_str);
        let got = actual.get(index).map(String::as_str);
        if want != got {
            return format!(
                "第 {} 行：期望 {:?}，实际 {:?}",
                index + 1,
                want.unwrap_or("<没有这一行>"),
                got.unwrap_or("<没有这一行>")
            );
        }
    }
    "两份树一样".to_string()
}

/// 拆开片段解析的上下文描述。
///
/// 格式见用例目录的 README：`svg ` 前缀表示 SVG 命名空间，`math ` 表示
/// MathML，其余是 HTML 命名空间下的元素名。
fn split_context(context: &str) -> (Namespace, &str) {
    if let Some(name) = context.strip_prefix("svg ") {
        (Namespace::Svg, name)
    } else if let Some(name) = context.strip_prefix("math ") {
        (Namespace::MathMl, name)
    } else {
        (Namespace::Html, context)
    }
}

/// 把 YSU 解析出来的结果拍成用例期望的那种文本。
///
/// 片段解析的产物是容器 `html` 元素的子节点，所以从它的子节点开始编号；
/// 整篇解析从文档根的子节点开始。
fn actual_tree(case: &Case) -> Vec<String> {
    match &case.context {
        Some(context) => {
            let (namespace, name) = split_context(context);
            let document = parse_fragment(&case.data, name, namespace);
            let mut lines = Vec::new();
            // 产物是容器 `html` 元素的子节点，容器自己不出现在结果里。
            let container = document
                .children(document.root())
                .first()
                .copied()
                .expect("片段解析总会建出容器");
            for &child in document.children(container) {
                dump_node(&document, child, 0, &mut lines);
            }
            lines
        }
        None => dump(&parse_document(&case.data)),
    }
}

/// 跑完整个用例集，统计结果。
///
/// 不通过就把详情写进 `CARGO_TARGET_TMPDIR/html5lib-failures.txt`，并把总数报出来。
/// 失败列表要落在文件里，终端上的截断会让人以为只有几条。
///
/// 片段解析的用例也一起跑，不单独排除——把它们挡在外面会让通过率虚高，
/// 那种数字没有意义。
#[test]
fn html5lib_tree_construction() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/html5lib/tree-construction");
    let mut passed = 0usize;
    let mut failures: Vec<String> = Vec::new();
    let mut total = 0usize;
    let mut fragment_total = 0usize;
    let mut fragment_failed = 0usize;

    for path in dat_files(&root) {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("?")
            .to_string();
        let text = std::fs::read_to_string(&path).expect("读得到用例文件");

        for case in parse_cases(&name, &text) {
            total += 1;
            if case.context.is_some() {
                fragment_total += 1;
            }

            let actual = actual_tree(&case);
            if actual == case.expected {
                passed += 1;
                continue;
            }
            if case.context.is_some() {
                fragment_failed += 1;
            }
            failures.push(format!(
                "=== {} ===\n输入：{:?}\n上下文：{:?}\n{}\n期望：\n{}\n实际：\n{}\n",
                case.label,
                case.data,
                case.context,
                first_difference(&case.expected, &actual),
                case.expected.join("\n"),
                actual.join("\n"),
            ));
        }
    }

    if !failures.is_empty() {
        let target = Path::new(env!("CARGO_TARGET_TMPDIR"));
        let _ = std::fs::create_dir_all(target);
        let _ = std::fs::write(target.join("html5lib-failures.txt"), failures.join("\n"));
    }

    let failed = failures.len();
    let fragment_passed = fragment_total - fragment_failed;
    println!(
        "树构建用例：{total} 条，通过 {passed}，失败 {failed}。\
         其中片段解析 {fragment_total} 条，通过 {fragment_passed}，失败 {fragment_failed}。"
    );

    assert_eq!(
        failed, 0,
        "{total} 条用例里有 {failed} 条不通过，前几条是：\n\n{}",
        failures
            .iter()
            .take(3)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}
