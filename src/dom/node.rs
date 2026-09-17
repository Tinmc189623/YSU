//! 文档对象模型。
//!
//! 节点存在一张表里，彼此之间用 [`NodeId`] 互相引用而不是指针。这样树里
//! 不会出现引用计数成环，样式表与布局表也都能直接拿 `NodeId` 当键。

use crate::html::Attribute;

/// 节点的表内编号。
///
/// 编号只在所属的 [`Document`] 内有效，跨文档使用没有意义。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(pub usize);

impl NodeId {
    /// 取出编号对应的下标。
    pub const fn index(self) -> usize {
        self.0
    }
}

/// 元素所在的命名空间。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Namespace {
    /// HTML 命名空间。
    #[default]
    Html,
    /// SVG 命名空间。
    Svg,
    /// MathML 命名空间。
    MathMl,
}

/// 元素节点。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Element {
    /// 标签名，HTML 元素一律小写。
    pub name: String,
    /// 所属命名空间。
    pub namespace: Namespace,
    /// 属性列表，保持源码里的顺序，重名属性在建树时已经去掉了。
    pub attributes: Vec<Attribute>,
}

impl Element {
    /// 建一个 HTML 元素。
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            namespace: Namespace::Html,
            attributes: Vec::new(),
        }
    }

    /// 取属性值。
    pub fn get_attribute(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|attribute| attribute.name == name)
            .map(|attribute| attribute.value.as_str())
    }

    /// 设置属性，已经存在就改值，不存在就追加。
    pub fn set_attribute(&mut self, name: &str, value: &str) {
        match self
            .attributes
            .iter_mut()
            .find(|attribute| attribute.name == name)
        {
            Some(attribute) => attribute.value = value.to_string(),
            None => self.attributes.push(Attribute {
                name: name.to_string(),
                value: value.to_string(),
            }),
        }
    }

    /// 删除属性。
    pub fn remove_attribute(&mut self, name: &str) {
        self.attributes.retain(|attribute| attribute.name != name);
    }

    /// 是否含有某个属性。
    pub fn has_attribute(&self, name: &str) -> bool {
        self.get_attribute(name).is_some()
    }

    /// 判断某个属性值是否落在以空白分隔的词列表里。
    ///
    /// `class` 与 `rel` 这类属性都是这种形式。
    pub fn has_token(&self, name: &str, token: &str) -> bool {
        match self.get_attribute(name) {
            Some(value) => value.split_whitespace().any(|item| item == token),
            None => false,
        }
    }

    /// 取 `id` 属性。
    pub fn id(&self) -> Option<&str> {
        self.get_attribute("id")
    }

    /// 按空白切分某个属性，得到词列表。
    pub fn tokens(&self, name: &str) -> Vec<&str> {
        match self.get_attribute(name) {
            Some(value) => value.split_whitespace().collect(),
            None => Vec::new(),
        }
    }

    /// 是否是 HTML 命名空间下的指定标签。
    pub fn is_html(&self, name: &str) -> bool {
        self.namespace == Namespace::Html && self.name == name
    }
}

/// 节点承载的内容。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeData {
    /// 文档根。
    Document,
    /// DOCTYPE 声明。
    Doctype(crate::html::Doctype),
    /// 元素。
    Element(Element),
    /// 文本。
    Text(String),
    /// 注释。
    Comment(String),
}

/// 一个节点。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    /// 节点内容。
    pub data: NodeData,
    /// 父节点。
    pub parent: Option<NodeId>,
    /// 子节点，按文档顺序排列。
    pub children: Vec<NodeId>,
}

impl Node {
    /// 该节点是否是元素。
    pub fn as_element(&self) -> Option<&Element> {
        match &self.data {
            NodeData::Element(element) => Some(element),
            _ => None,
        }
    }

    /// 该节点是否是可变的元素。
    pub fn as_element_mut(&mut self) -> Option<&mut Element> {
        match &mut self.data {
            NodeData::Element(element) => Some(element),
            _ => None,
        }
    }

    /// 取文本内容，非文本节点返回 `None`。
    pub fn as_text(&self) -> Option<&str> {
        match &self.data {
            NodeData::Text(text) => Some(text),
            _ => None,
        }
    }

    /// 取注释内容，非注释节点返回 `None`。
    pub fn as_comment(&self) -> Option<&str> {
        match &self.data {
            NodeData::Comment(text) => Some(text),
            _ => None,
        }
    }

    /// 该节点的标签名，非元素返回 `None`。
    pub fn tag_name(&self) -> Option<&str> {
        self.as_element().map(|element| element.name.as_str())
    }

    /// 是否是 HTML 命名空间下的指定标签。
    pub fn is_element(&self, name: &str) -> bool {
        self.as_element()
            .is_some_and(|element| element.is_html(name))
    }

    /// 该节点是否是元素且带有指定属性。
    pub fn has_attribute(&self, name: &str) -> bool {
        self.as_element()
            .is_some_and(|element| element.has_attribute(name))
    }
}

/// 一棵文档树。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    /// 全部节点。
    nodes: Vec<Node>,
    /// 文档根节点。
    root: NodeId,
}

impl Default for Document {
    /// 建一棵只有文档根的树。
    fn default() -> Self {
        Self::new()
    }
}

impl Document {
    /// 建一棵只有文档根的树。
    pub fn new() -> Self {
        Self {
            nodes: vec![Node {
                data: NodeData::Document,
                parent: None,
                children: Vec::new(),
            }],
            root: NodeId(0),
        }
    }

    /// 文档根节点。
    pub fn root(&self) -> NodeId {
        self.root
    }

    /// 节点总数，含已经不在树上的节点。
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// 是否只有一个文档根。
    pub fn is_empty(&self) -> bool {
        self.nodes.len() == 1
    }

    /// 取节点。
    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id.0]
    }

    /// 取可变节点。
    pub fn node_mut(&mut self, id: NodeId) -> &mut Node {
        &mut self.nodes[id.0]
    }

    /// 取元素，节点不是元素时返回 `None`。
    pub fn element(&self, id: NodeId) -> Option<&Element> {
        self.nodes[id.0].as_element()
    }

    /// 取可变元素。
    pub fn element_mut(&mut self, id: NodeId) -> Option<&mut Element> {
        self.nodes[id.0].as_element_mut()
    }

    /// 新建一个节点并返回编号。
    ///
    /// 新节点还没有接到树上，需要调用 [`Document::append_child`] 之类的方法挂上去。
    pub fn create(&mut self, data: NodeData) -> NodeId {
        self.nodes.push(Node {
            data,
            parent: None,
            children: Vec::new(),
        });
        NodeId(self.nodes.len() - 1)
    }

    /// 建一个元素节点。
    pub fn create_element(&mut self, element: Element) -> NodeId {
        self.create(NodeData::Element(element))
    }

    /// 建一个文本节点。
    pub fn create_text(&mut self, text: impl Into<String>) -> NodeId {
        self.create(NodeData::Text(text.into()))
    }

    /// 建一个注释节点。
    pub fn create_comment(&mut self, text: impl Into<String>) -> NodeId {
        self.create(NodeData::Comment(text.into()))
    }

    /// 把节点挂到父节点末尾。
    ///
    /// 节点原本挂在别处时会先从原位置摘下来。
    pub fn append_child(&mut self, parent: NodeId, child: NodeId) {
        self.detach(child);
        self.nodes[child.0].parent = Some(parent);
        self.nodes[parent.0].children.push(child);
    }

    /// 把节点插到父节点的指定位置之前。
    pub fn insert_before(&mut self, parent: NodeId, child: NodeId, reference: NodeId) {
        self.detach(child);
        self.nodes[child.0].parent = Some(parent);
        let position = self.nodes[parent.0]
            .children
            .iter()
            .position(|existing| *existing == reference)
            .unwrap_or(self.nodes[parent.0].children.len());
        self.nodes[parent.0].children.insert(position, child);
    }

    /// 把节点从树上摘下来，保留节点本身。
    pub fn detach(&mut self, child: NodeId) {
        if let Some(parent) = self.nodes[child.0].parent.take() {
            self.nodes[parent.0]
                .children
                .retain(|existing| *existing != child);
        }
    }

    /// 取父节点。
    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.nodes[id.0].parent
    }

    /// 取子节点列表。
    pub fn children(&self, id: NodeId) -> &[NodeId] {
        &self.nodes[id.0].children
    }

    /// 取第 `index` 个子节点。
    pub fn child(&self, id: NodeId, index: usize) -> Option<NodeId> {
        self.nodes[id.0].children.get(index).copied()
    }

    /// 取最后一个子节点。
    pub fn last_child(&self, id: NodeId) -> Option<NodeId> {
        self.nodes[id.0].children.last().copied()
    }

    /// 取同级的下一个节点。
    pub fn next_sibling(&self, id: NodeId) -> Option<NodeId> {
        let parent = self.nodes[id.0].parent?;
        let siblings = &self.nodes[parent.0].children;
        let index = siblings.iter().position(|sibling| *sibling == id)?;
        siblings.get(index + 1).copied()
    }

    /// 取同级的上一个节点。
    pub fn previous_sibling(&self, id: NodeId) -> Option<NodeId> {
        let parent = self.nodes[id.0].parent?;
        let siblings = &self.nodes[parent.0].children;
        let index = siblings.iter().position(|sibling| *sibling == id)?;
        index.checked_sub(1).and_then(|i| siblings.get(i).copied())
    }

    /// 按文档顺序深度优先遍历以 `root` 为根的子树，含 `root` 本身。
    pub fn descendants(&self, root: NodeId) -> Vec<NodeId> {
        let mut out = Vec::new();
        self.collect_descendants(root, &mut out);
        out
    }

    /// 递归收集子树里的节点。
    fn collect_descendants(&self, node: NodeId, out: &mut Vec<NodeId>) {
        out.push(node);
        for child in &self.nodes[node.0].children {
            self.collect_descendants(*child, out);
        }
    }

    /// 按文档顺序遍历子树里的全部元素。
    pub fn descendants_of_kind(&self, root: NodeId, name: &str) -> Vec<NodeId> {
        self.descendants(root)
            .into_iter()
            .filter(|id| self.nodes[id.0].is_element(name))
            .collect()
    }

    /// 取节点及其后代的全部文本，拼接成一个字符串。
    ///
    /// 只在块级元素之间补换行，方便给 `textContent` 之外的地方做摘要。
    pub fn text_content(&self, node: NodeId) -> String {
        let mut out = String::new();
        self.collect_text(node, &mut out);
        out
    }

    /// 递归收集文本。
    fn collect_text(&self, node: NodeId, out: &mut String) {
        match &self.nodes[node.0].data {
            NodeData::Text(text) => out.push_str(text),
            NodeData::Element(_) | NodeData::Document => {
                for child in &self.nodes[node.0].children {
                    self.collect_text(*child, out);
                }
            }
            NodeData::Comment(_) | NodeData::Doctype(_) => {}
        }
    }

    /// 按顺序查找后代里第一个满足条件的元素。
    pub fn find_element(
        &self,
        root: NodeId,
        predicate: impl Fn(&Element) -> bool,
    ) -> Option<NodeId> {
        self.descendants(root)
            .into_iter()
            .find(|id| self.nodes[id.0].as_element().is_some_and(&predicate))
    }

    /// 找出全部满足条件的元素。
    pub fn find_elements(&self, root: NodeId, predicate: impl Fn(&Element) -> bool) -> Vec<NodeId> {
        self.descendants(root)
            .into_iter()
            .filter(|id| self.nodes[id.0].as_element().is_some_and(&predicate))
            .collect()
    }

    /// 取文档标题，也就是 `title` 元素的文本内容。
    ///
    /// 没有 `title` 元素或内容为空时返回 `None`。
    pub fn title(&self) -> Option<String> {
        let node = self.find_element(self.root, |element| element.is_html("title"))?;
        let text = self.text_content(node);
        let trimmed = text.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    }

    /// 取最后一个元素子节点。
    pub fn last_element_child(&self, id: NodeId) -> Option<NodeId> {
        self.nodes[id.0]
            .children
            .iter()
            .rev()
            .copied()
            .find(|child| self.nodes[child.0].as_element().is_some())
    }

    /// 从节点往下取第一个元素子节点。
    pub fn first_element_child(&self, id: NodeId) -> Option<NodeId> {
        self.nodes[id.0]
            .children
            .iter()
            .copied()
            .find(|child| self.nodes[child.0].as_element().is_some())
    }

    /// 把某个元素的子节点全部交给另一个元素，用于建树过程中的搬移。
    pub fn reparent_children(&mut self, from: NodeId, to: NodeId) {
        let children = std::mem::take(&mut self.nodes[from.0].children);
        for child in children {
            self.nodes[child.0].parent = Some(to);
            self.nodes[to.0].children.push(child);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 建一份用于测试的文档，结构是 `div > (span, "text")`。
    fn sample() -> (Document, NodeId, NodeId, NodeId) {
        let mut document = Document::new();
        let div = document.create_element(Element::new("div"));
        let span = document.create_element(Element::new("span"));
        let text = document.create_text("hi");
        document.append_child(document.root(), div);
        document.append_child(div, span);
        document.append_child(div, text);
        (document, div, span, text)
    }

    #[test]
    fn structure_is_linked_both_ways() {
        let (document, div, span, text) = sample();
        assert_eq!(document.children(div), &[span, text]);
        assert_eq!(document.parent(span), Some(div));
        assert_eq!(document.parent(div), Some(document.root()));
    }

    #[test]
    fn sibling_navigation() {
        let (document, _div, span, text) = sample();
        assert_eq!(document.next_sibling(span), Some(text));
        assert_eq!(document.previous_sibling(text), Some(span));
        assert_eq!(document.next_sibling(text), None);
        assert_eq!(document.previous_sibling(span), None);
    }

    #[test]
    fn detach_removes_from_parent() {
        let (mut document, div, span, _text) = sample();
        document.detach(span);
        assert_eq!(document.parent(span), None);
        assert_eq!(document.children(div).len(), 1);
    }

    #[test]
    fn append_moves_existing_node() {
        let (mut document, div, span, _text) = sample();
        let section = document.create_element(Element::new("section"));
        document.append_child(document.root(), section);
        document.append_child(section, span);
        assert_eq!(document.children(div).len(), 1);
        assert_eq!(document.children(section), &[span]);
    }

    #[test]
    fn insert_before_positions_correctly() {
        let (mut document, div, _span, text) = sample();
        let em = document.create_element(Element::new("em"));
        document.insert_before(div, em, text);
        assert_eq!(document.children(div)[1], em);
    }

    #[test]
    fn insert_before_missing_reference_appends() {
        let (mut document, div, _span, _text) = sample();
        let em = document.create_element(Element::new("em"));
        let detached = document.create_element(Element::new("b"));
        document.insert_before(div, em, detached);
        assert_eq!(document.last_child(div), Some(em));
    }

    #[test]
    fn descendants_visit_in_document_order() {
        let (document, div, span, text) = sample();
        assert_eq!(document.descendants(div), vec![div, span, text]);
    }

    #[test]
    fn text_content_concatenates() {
        let (document, div, span, _text) = sample();
        assert_eq!(document.text_content(div), "hi");
        assert_eq!(document.text_content(span), "");
    }

    #[test]
    fn text_content_skips_comments() {
        let mut document = Document::new();
        let body = document.create_element(Element::new("body"));
        let text = document.create_text("a");
        let comment = document.create_comment("ignored");
        let text2 = document.create_text("b");
        document.append_child(document.root(), body);
        document.append_child(body, text);
        document.append_child(body, comment);
        document.append_child(body, text2);
        assert_eq!(document.text_content(body), "ab");
    }

    #[test]
    fn find_elements_by_predicate() {
        let (document, div, span, _text) = sample();
        let found = document.find_elements(div, |element| element.name == "span");
        assert_eq!(found, vec![span]);
        assert_eq!(
            document.find_element(div, |element| element.name == "div"),
            Some(div)
        );
    }

    #[test]
    fn element_attributes() {
        let mut element = Element::new("a");
        element.set_attribute("href", "x");
        element.set_attribute("class", "one two");
        assert_eq!(element.get_attribute("href"), Some("x"));
        assert!(element.has_token("class", "two"));
        assert!(!element.has_token("class", "three"));
        assert_eq!(element.tokens("class"), vec!["one", "two"]);

        element.set_attribute("href", "y");
        assert_eq!(element.get_attribute("href"), Some("y"));
        assert_eq!(element.attributes.len(), 2);

        element.remove_attribute("href");
        assert!(!element.has_attribute("href"));
    }

    #[test]
    fn element_identity_helpers() {
        let mut element = Element::new("div");
        element.set_attribute("id", "main");
        assert!(element.is_html("div"));
        assert!(!element.is_html("span"));
        assert_eq!(element.id(), Some("main"));
    }

    #[test]
    fn reparent_children_moves_all() {
        let (mut document, div, span, text) = sample();
        let section = document.create_element(Element::new("section"));
        document.append_child(document.root(), section);
        document.reparent_children(div, section);
        assert!(document.children(div).is_empty());
        assert_eq!(document.children(section), &[span, text]);
    }

    #[test]
    fn last_element_child_skips_text() {
        let (document, div, span, _text) = sample();
        assert_eq!(document.last_element_child(div), Some(span));
        assert_eq!(document.first_element_child(div), Some(span));
    }
}
