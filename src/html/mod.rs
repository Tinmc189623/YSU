//! HTML 解析：词法分析、字符引用解码与树构建。

pub mod entities;
pub mod foreign;
pub mod references;
pub mod tokenizer;
pub mod tree_builder;

pub use references::{NamedReference, NumericReference, lookup_named, lookup_numeric};
pub use tokenizer::{
    Attribute, Doctype, RawTextMode, Tag, Token, Tokenizer, decode_character_references, tokenize,
};
pub use tree_builder::TreeBuilder;

use crate::dom::node::{Document, Namespace as NodeNamespace};

/// 解析一段 HTML，返回文档树。
pub fn parse_document(input: &str) -> Document {
    TreeBuilder::new(input).parse()
}

/// 按片段解析一段 HTML，上下文是给定的元素。
///
/// 返回的文档里只有一个 `html` 元素，它的子节点就是片段的内容——这是规范
/// 定的形状。片段解析用在 `innerHTML` 那类场景上：上下文元素决定内容该怎么
/// 解析，比如上下文是 `table` 时，一段 `<tr>` 才不会被当成无主的行丢掉。
pub fn parse_fragment(input: &str, context_name: &str, namespace: NodeNamespace) -> Document {
    TreeBuilder::new_fragment(input, context_name, namespace).parse()
}
