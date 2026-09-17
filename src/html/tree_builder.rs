//! HTML 树构建。
//!
//! 按 HTML 规范的插入模式状态机把记号流变成文档树。规范里最绕的三处都在
//! 这里实现了：格式化元素的收养机构算法（处理交错的 `<b>` 与 `<i>`）、
//! 活动格式化元素表的重建、以及表格内容不对时把节点寄养到表格之前的规则。

use super::foreign;
use super::tokenizer::{Attribute, Doctype, RawTextMode, Tag, Token, Tokenizer};
use crate::dom::node::{Document, Element, Namespace, NodeData, NodeId};

/// 插入模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InsertionMode {
    /// 处理文档最开始的 DOCTYPE 与注释。
    Initial,
    /// 建立 `html` 元素之前。
    BeforeHtml,
    /// 建立 `head` 元素之前。
    BeforeHead,
    /// `head` 内部。
    InHead,
    /// `head` 结束之后、`body` 开始之前。
    AfterHead,
    /// `body` 内部，绝大部分内容都在这里处理。
    InBody,
    /// 原始文本元素的内部，如 `script` 与 `title`。
    Text,
    /// 表格内部。
    InTable,
    /// 表格里出现的非表格文本，需要攒起来判断。
    InTableText,
    /// `caption` 内部。
    InCaption,
    /// `colgroup` 内部。
    InColumnGroup,
    /// `tbody`、`thead`、`tfoot` 内部。
    InTableBody,
    /// `tr` 内部。
    InRow,
    /// `td` 与 `th` 内部。
    InCell,
    /// `select` 内部。
    InSelect,
    /// `select` 内部出现了表格标签。
    InSelectInTable,
    /// `body` 结束之后。
    AfterBody,
    /// 整个文档结束之后。
    AfterAfterBody,
}

/// 活动格式化元素表里的一项。
#[derive(Debug, Clone)]
enum FormattingEntry {
    /// 作用域分界标记，阻止跨表格重建。
    Marker,
    /// 一个格式化元素。
    Element {
        /// 元素编号。
        id: NodeId,
        /// 标签名。
        name: String,
        /// 建立时的属性，重建时要照着再建一个。
        attributes: Vec<Attribute>,
    },
}

/// 表格里待判定的文本片段。
struct PendingTableText {
    /// 是否含非空白字符。
    has_non_whitespace: bool,
    /// 累计的文本。
    text: String,
}

/// 树构建器。
pub struct TreeBuilder<'a> {
    /// 记号来源。
    tokenizer: Tokenizer<'a>,
    /// 正在构建的文档。
    document: Document,
    /// 未闭合元素的栈，栈顶是当前节点。
    open_elements: Vec<NodeId>,
    /// 活动格式化元素表。
    active_formatting: Vec<FormattingEntry>,
    /// 当前插入模式。
    mode: InsertionMode,
    /// 进入 [`InsertionMode::Text`] 之前的模式，离开时恢复。
    original_mode: InsertionMode,
    /// `head` 元素。
    head: Option<NodeId>,
    /// 文档是否按怪异模式解析。
    quirks: bool,
    /// 表格文本的暂存区。
    pending_table_text: Option<PendingTableText>,
    /// `pre`、`listing` 与 `textarea` 之后紧邻的一个换行要丢掉。
    skip_next_newline: bool,
    /// 是否把内容寄养到表格之前，只在处理非表格内容时打开。
    foster_parenting: bool,
    /// 片段解析的上下文元素，整篇解析时是 `None`。
    fragment: Option<FragmentContext>,
    /// 片段解析时那个虚拟的上下文元素节点。
    ///
    /// 它不在树上，只用来回答「调整当前节点是谁」——上下文是 SVG 或 MathML
    /// 时，片段内容要按外来内容解析，靠的就是它。
    fragment_root: Option<NodeId>,
}

/// 片段解析的上下文。
///
/// 上下文元素本身不会出现在产物里，它只决定两件事：起始插入模式，以及
/// 记号该怎么切（上下文是 `title` 时内容按 RCDATA 读，等等）。
#[derive(Debug, Clone)]
struct FragmentContext {
    /// 上下文元素的标签名，小写。
    name: String,
    /// 上下文元素所在的命名空间。
    namespace: Namespace,
}

impl<'a> TreeBuilder<'a> {
    /// 基于一段 HTML 文本构造构建器。
    pub fn new(input: &'a str) -> Self {
        Self {
            tokenizer: Tokenizer::new(input),
            document: Document::new(),
            open_elements: Vec::new(),
            active_formatting: Vec::new(),
            mode: InsertionMode::Initial,
            original_mode: InsertionMode::Initial,
            head: None,
            quirks: false,
            pending_table_text: None,
            skip_next_newline: false,
            foster_parenting: false,
            fragment: None,
            fragment_root: None,
        }
    }

    /// 构造一个片段解析器，上下文是某个元素。
    ///
    /// 片段解析与整篇解析的区别只有开头：没有 `html`/`head`/`body` 那一套，
    /// 起始插入模式由上下文元素决定，内容全部挂在一个新建的 `html` 元素下面。
    /// 收尾之后，那个 `html` 元素的子节点就是片段的产物。
    pub fn new_fragment(input: &'a str, context_name: &str, namespace: Namespace) -> Self {
        let mut builder = Self::new(input);
        builder.fragment = Some(FragmentContext {
            name: context_name.to_ascii_lowercase(),
            namespace,
        });
        builder
    }

    /// 解析整段输入，返回文档树。
    pub fn parse(mut self) -> Document {
        if let Some(context) = self.fragment.clone() {
            self.start_fragment(&context);
        }
        loop {
            let token = self.tokenizer.next_token();
            let is_eof = matches!(token, Token::Eof);
            // 输入结束也要过一遍插入模式，否则空文档里连 html 都不会建出来。
            self.process_token(token);
            if is_eof {
                self.finish();
                break;
            }
        }
        // 输入结束时表格里攒着的文本还没落地，补一次。
        self.flush_pending_table_text();
        self.document
    }

    /// 片段解析的开场。
    ///
    /// 建一个 `html` 元素当容器，栈里只放它，插入模式与记号状态都按上下文
    /// 元素定。规范里片段解析到最后取的就是这个 `html` 元素的子节点，所以
    /// 产物是它的子节点，不是整篇文档。
    fn start_fragment(&mut self, context: &FragmentContext) {
        let id = self.document.create_element(Element::new("html"));
        self.document.append_child(self.document.root(), id);
        self.open_elements.push(id);

        // 上下文元素只建节点不上树：它不参与产物，只用来回答「调整当前
        // 节点是谁」。栈里只剩这一个 html 时，规范要求把上下文元素当成
        // 当前节点，外来命名空间的上下文才走得通。
        let mut element = Element::new(&context.name);
        element.namespace = context.namespace;
        self.fragment_root = Some(self.document.create_element(element));

        self.mode = context_insertion_mode(&context.name);

        // 上下文是原始文本类的元素时，内容整段按文本读，标签不成其为标签。
        match context.name.as_str() {
            "title" | "textarea" => {
                self.tokenizer
                    .set_raw_text_mode(RawTextMode::Rcdata, &context.name);
            }
            "style" | "xmp" | "iframe" | "noembed" | "noframes" | "listing" => {
                self.tokenizer
                    .set_raw_text_mode(RawTextMode::Rawtext, &context.name);
            }
            "script" => {
                self.tokenizer
                    .set_raw_text_mode(RawTextMode::ScriptData, &context.name);
            }
            "plaintext" => {
                self.tokenizer.set_raw_text_mode(RawTextMode::Plaintext, "");
            }
            _ => {}
        }
    }

    // ---- 外来内容 ----
    //
    // SVG 与 MathML 的子树里，元素名大小写敏感、属性要恢复成驼峰写法、
    // 出现的 HTML 标签要把整棵子树收掉再按 HTML 处理。这一套和 body 里那套
    // 完全不同，规范把它们分成两条路，这里也照分。

    /// 元素是不是外来元素，也就是不在 HTML 命名空间里。
    fn is_foreign_element(&self, id: NodeId) -> bool {
        self.document
            .element(id)
            .is_some_and(|element| element.namespace != Namespace::Html)
    }

    /// 当前的调整节点。
    ///
    /// 片段解析时，栈里只剩一个元素就取上下文元素——这是一条特例，
    /// 为了让上下文是 SVG 或 MathML 的片段也走外来内容那一条路。
    /// 其余情况都取栈顶。
    fn adjusted_current_node(&self) -> Option<NodeId> {
        if self.open_elements.len() == 1
            && let Some(context) = self.fragment_root
        {
            return Some(context);
        }
        self.current_node()
    }

    /// 是不是 MathML 的文本整合点。
    ///
    /// 这几个元素的内容按 HTML 规则解析，所以它们的子元素回到 HTML 命名空间。
    fn is_mathml_text_integration_point(&self, id: NodeId) -> bool {
        self.document.element(id).is_some_and(|element| {
            element.namespace == Namespace::MathMl
                && matches!(element.name.as_str(), "mi" | "mo" | "mn" | "ms" | "mtext")
        })
    }

    /// 是不是 HTML 整合点。
    ///
    /// SVG 里 `foreignObject`、`desc`、`title` 三个元素的内容是 HTML；
    /// MathML 里 `annotation-xml` 带了 HTML 的 encoding 时也一样。
    fn is_html_integration_point(&self, id: NodeId) -> bool {
        let Some(element) = self.document.element(id) else {
            return false;
        };
        match element.namespace {
            Namespace::Svg => matches!(element.name.as_str(), "foreignObject" | "desc" | "title"),
            Namespace::MathMl => {
                element.name == "annotation-xml"
                    && element.get_attribute("encoding").is_some_and(|value| {
                        let value = value.trim().to_ascii_lowercase();
                        value == "text/html" || value == "application/xhtml+xml"
                    })
            }
            Namespace::Html => false,
        }
    }

    /// 这个记号该不该走外来内容那一套规则。
    ///
    /// 基本条件是「当前节点是外来元素」，但有几处例外——整合点里的内容和
    /// MathML 的 `annotation-xml` 里的 `svg`，都要按 HTML 处理。空字符与
    /// 注释不例外，仍旧走外来那一套。
    fn in_foreign_content(&self, token: &Token) -> bool {
        // 输入结束永远走插入模式那一侧，不然收尾会漏掉。
        if matches!(token, Token::Eof) {
            return false;
        }
        let Some(current) = self.adjusted_current_node() else {
            return false;
        };
        let Some(element) = self.document.element(current) else {
            return false;
        };
        if element.namespace == Namespace::Html {
            return false;
        }

        // MathML 文本整合点里，字符与「不是 mglyph/malignmark 的起始标签」按 HTML 走。
        if self.is_mathml_text_integration_point(current) {
            let html_side = match token {
                Token::Character(_) => true,
                Token::StartTag(tag) => !matches!(tag.name.as_str(), "mglyph" | "malignmark"),
                _ => false,
            };
            if html_side {
                return false;
            }
        }
        // HTML 整合点里，字符与起始标签按 HTML 走。
        if self.is_html_integration_point(current)
            && matches!(token, Token::StartTag(_) | Token::Character(_))
        {
            return false;
        }
        // `annotation-xml` 里的 `svg` 也是 HTML 那一侧的。
        if element.namespace == Namespace::MathMl
            && element.name == "annotation-xml"
            && matches!(token, Token::StartTag(tag) if tag.name == "svg")
        {
            return false;
        }
        true
    }

    /// 外来内容的记号处理。
    fn foreign_content(&mut self, token: Token) {
        match token {
            Token::Character(text) => self.insert_foreign_text(&text),
            Token::Comment(text) => {
                let node = self.document.create_comment(text);
                self.insert_at_appropriate_place(node);
            }
            Token::StartTag(tag) => self.foreign_start_tag(tag),
            Token::EndTag(tag) => self.foreign_end_tag(tag),
            // CDATA 在进这里之前就分流了，DOCTYPE 在外来内容里被忽略。
            Token::Cdata(_) | Token::Doctype(_) | Token::Eof => {}
        }
    }

    /// 外来内容里插一段文本。
    ///
    /// 空字符换成替换字符。HTML 那边是直接丢掉，两边规矩不一样，不能共用。
    fn insert_foreign_text(&mut self, text: &str) {
        let text: String = text
            .chars()
            .map(|character| {
                if character == '\0' {
                    '\u{fffd}'
                } else {
                    character
                }
            })
            .collect();
        if !text.is_empty() {
            self.insert_text(&text);
        }
    }

    /// 外来内容里的起始标签。
    fn foreign_start_tag(&mut self, tag: Tag) {
        // 碰到 HTML 的标签就说明外来子树到此为止：一路弹回 HTML 元素或
        // 整合点，再把这个标签交给插入模式那一侧重新处理。
        //
        // 这里的「交给」是一次直接调用。走 process_token 的话，判定会再问
        // 一遍「当前在不在外来内容里」——片段解析的上下文是 SVG 时，栈里只
        // 剩 `html` 而调整当前节点仍指向上下文元素，判定照样说是外来内容，
        // 于是两边来回弹到栈溢出。
        if foreign::breaks_out(&tag.name, &tag.attributes) {
            self.pop_until_html_or_integration_point();
            self.process_token_html(Token::StartTag(tag));
            return;
        }

        // 新元素跟着当前元素所在的命名空间，不是固定 HTML。
        let namespace = self
            .adjusted_current_node()
            .and_then(|id| self.document.element(id))
            .map_or(Namespace::Html, |element| element.namespace);

        let name = match namespace {
            Namespace::Svg => foreign::adjust_svg_tag_name(&tag.name).to_string(),
            _ => tag.name.clone(),
        };
        let mut attributes = tag.attributes.clone();
        match namespace {
            Namespace::Svg => foreign::adjust_svg_attributes(&mut attributes),
            Namespace::MathMl => foreign::adjust_mathml_attributes(&mut attributes),
            Namespace::Html => {}
        }

        self.insert_element(&name, &attributes, namespace);
        if tag.self_closing {
            self.pop();
        }
    }

    /// 外来内容里的结束标签。
    ///
    /// 从栈顶往下找同名的元素，找到就把连同它在内的一串弹掉。中途碰到 HTML
    /// 元素或整合点还没找到，说明这个结束标签是给 HTML 那一侧的，转过去处理。
    fn foreign_end_tag(&mut self, tag: Tag) {
        if foreign::breaks_out_end_tag(&tag.name) {
            self.pop_until_html_or_integration_point();
            // 同起始标签：这一步是直接进 HTML 规则，不再过外来内容的判定。
            self.process_token_html(Token::EndTag(tag));
            return;
        }

        for index in (0..self.open_elements.len()).rev() {
            let node = self.open_elements[index];
            let matched = self
                .document
                .node(node)
                .tag_name()
                .is_some_and(|name| name.eq_ignore_ascii_case(&tag.name));
            if matched {
                self.open_elements.truncate(index);
                return;
            }
            if index == 0 {
                return;
            }
            if !self.is_foreign_element(node) {
                // 规范这一步是直接进插入模式，不再过外来内容的判定。
                self.process_token_html(Token::EndTag(tag));
                return;
            }
        }
    }

    /// 从栈顶往下弹，直到当前节点是 HTML 元素或者某个整合点。
    fn pop_until_html_or_integration_point(&mut self) {
        while let Some(current) = self.current_node() {
            if !self.is_foreign_element(current)
                || self.is_mathml_text_integration_point(current)
                || self.is_html_integration_point(current)
            {
                return;
            }
            self.pop();
        }
    }

    /// 把输入结束这个记号一路走完插入模式链。
    ///
    /// 规范里 EOF 是被逐个模式依次处理的，直到某一处真的停下来为止。只处理
    /// 一遍是不够的：`<!doctype html><script>` 停在文本模式里，那一步只把
    /// script 弹掉、退回原来的模式，head 还没见过 EOF，就永远不会被收掉，
    /// body 也建不出来。
    ///
    /// 每一轮记下模式，模式没变就说明这一处把 EOF 收下了，可以停。次数上限是
    /// 防呆，正常情况下两三轮就走完。
    fn finish(&mut self) {
        for _ in 0..32 {
            let before = self.mode;
            self.process_token(Token::Eof);
            if self.mode == before {
                break;
            }
        }
    }

    /// 文档是否按怪异模式解析。
    pub fn is_quirks_mode(&self) -> bool {
        self.quirks
    }

    // ---- 基础操作 ----

    /// 当前节点，也就是栈顶。
    fn current_node(&self) -> Option<NodeId> {
        self.open_elements.last().copied()
    }

    /// 把元素压入栈。
    fn push(&mut self, id: NodeId) {
        self.open_elements.push(id);
    }

    /// 弹出栈顶元素。
    fn pop(&mut self) -> Option<NodeId> {
        self.open_elements.pop()
    }

    /// 元素是否还在栈上。
    fn is_on_stack(&self, id: NodeId) -> bool {
        self.open_elements.contains(&id)
    }

    /// 从栈里移除指定元素。
    fn remove_from_stack(&mut self, id: NodeId) {
        self.open_elements.retain(|existing| *existing != id);
    }

    /// 按标签名找栈里最靠上的元素下标。
    fn stack_index_of(&self, name: &str) -> Option<usize> {
        self.open_elements
            .iter()
            .rposition(|id| self.document.node(*id).is_element(name))
    }

    /// 按标签名取出栈里最靠上的元素。
    fn stack_element_of(&self, name: &str) -> Option<NodeId> {
        self.stack_index_of(name)
            .map(|index| self.open_elements[index])
    }

    /// 把新节点插到合适的位置。
    fn insert_at_appropriate_place(&mut self, node: NodeId) {
        match self.appropriate_place() {
            Some((parent, before)) => match before {
                Some(reference) => self.document.insert_before(parent, node, reference),
                None => self.document.append_child(parent, node),
            },
            None => self.document.append_child(self.document.root(), node),
        }
    }

    /// 计算插入位置，返回父节点与需要插到它前面的参照节点。
    ///
    /// 表格内容不对时要把节点插到表格之前，这叫寄养。注意寄养不是「在表格
    /// 模式里就自动发生」——`tbody` 与 `tr` 本身就是在表格模式里插入的，
    /// 它们必须留在表格内。规范用一个开关控制，只有处理非表格内容时才打开。
    fn appropriate_place(&self) -> Option<(NodeId, Option<NodeId>)> {
        let target = self.current_node()?;

        let is_table_ish = matches!(
            self.document.node(target).tag_name(),
            Some("table" | "tbody" | "tfoot" | "thead" | "tr")
        );
        if !(self.foster_parenting && is_table_ish) {
            return Some((target, None));
        }

        // 找到栈里最后一个表格元素，插到它前面。
        let table = self
            .open_elements
            .iter()
            .rev()
            .copied()
            .find(|id| self.document.node(*id).is_element("table"))?;
        let table_parent = self.document.parent(table)?;
        Some((table_parent, Some(table)))
    }

    /// 打开寄养开关跑一段逻辑，结束后恢复原值。
    fn with_foster_parenting<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
        let saved = self.foster_parenting;
        self.foster_parenting = true;
        let result = f(self);
        self.foster_parenting = saved;
        result
    }

    /// 建立元素并挂到合适的位置，同时压入栈。
    fn insert_element(
        &mut self,
        name: &str,
        attributes: &[Attribute],
        namespace: Namespace,
    ) -> NodeId {
        let mut element = Element::new(name);
        element.namespace = namespace;
        element.attributes = attributes.to_vec();
        let id = self.document.create_element(element);
        self.insert_at_appropriate_place(id);
        self.push(id);
        id
    }

    /// 按起始标签建立 HTML 元素。
    fn insert_html_element(&mut self, tag: &Tag) -> NodeId {
        self.insert_element(&tag.name, &tag.attributes, Namespace::Html)
    }

    /// 建立元素但不压栈，用于 `html`、`head` 这类特殊元素由调用方决定压栈时机。
    fn create_element(&mut self, name: &str, attributes: &[Attribute]) -> NodeId {
        let mut element = Element::new(name);
        element.attributes = attributes.to_vec();
        let id = self.document.create_element(element);
        self.insert_at_appropriate_place(id);
        id
    }

    /// 插入一段文本，与紧邻的文本节点合并。
    fn insert_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let place = self.appropriate_place();
        let Some((parent, before)) = place else {
            return;
        };

        // 紧挨着已有的文本节点时直接追加，避免碎片化。
        let neighbour = match before {
            Some(reference) => self.document.previous_sibling(reference),
            None => self.document.last_child(parent),
        };
        if let Some(previous) = neighbour
            && let NodeData::Text(existing) = &mut self.document.node_mut(previous).data
        {
            existing.push_str(text);
            return;
        }

        let node = self.document.create_text(text);
        match before {
            Some(reference) => self.document.insert_before(parent, node, reference),
            None => self.document.append_child(parent, node),
        }
    }

    /// 从栈顶往下按标签名逐个弹出，直到弹出指定标签为止。
    fn pop_until(&mut self, name: &str) {
        while let Some(id) = self.pop() {
            if self.document.node(id).is_element(name) {
                break;
            }
        }
    }

    /// 生成隐含的结束标签。
    fn generate_implied_end_tags(&mut self, except: Option<&str>) {
        while let Some(id) = self.current_node() {
            let name = match self.document.node(id).tag_name() {
                Some(name) => name,
                None => break,
            };
            if !is_implied_end_tag(name) {
                break;
            }
            if Some(name) == except {
                break;
            }
            self.pop();
        }
    }

    // ---- 作用域判断 ----

    /// 栈上是否存在处于默认作用域内的指定元素。
    fn has_in_scope(&self, name: &str) -> bool {
        self.has_in_scope_with(name, is_scope_terminator)
    }

    /// 栈上是否存在处于列表项作用域内的指定元素。
    fn has_in_list_item_scope(&self, name: &str) -> bool {
        self.has_in_scope_with(name, |tag| {
            is_scope_terminator(tag) || matches!(tag, "ol" | "ul")
        })
    }

    /// 栈上是否存在处于按钮作用域内的指定元素。
    fn has_in_button_scope(&self, name: &str) -> bool {
        self.has_in_scope_with(name, |tag| is_scope_terminator(tag) || tag == "button")
    }

    /// 栈上是否存在处于表格作用域内的指定元素。
    fn has_in_table_scope(&self, name: &str) -> bool {
        self.has_in_scope_with(name, |tag| matches!(tag, "html" | "table" | "template"))
    }

    /// 通用作用域查找，`terminator` 决定哪些元素构成边界。
    fn has_in_scope_with(&self, name: &str, terminator: impl Fn(&str) -> bool) -> bool {
        for id in self.open_elements.iter().rev() {
            let Some(tag) = self.document.node(*id).tag_name() else {
                continue;
            };
            if tag == name {
                return true;
            }
            if terminator(tag) {
                return false;
            }
        }
        false
    }

    /// 如果栈上有 `p` 元素在按钮作用域内，就把它闭合掉。
    fn close_p_element(&mut self) {
        if self.has_in_button_scope("p") {
            self.generate_implied_end_tags(Some("p"));
            self.pop_until("p");
        }
    }

    // ---- 主循环 ----

    /// 按当前插入模式处理一个记号。
    fn process_token(&mut self, token: Token) {
        // CDATA 段先处理掉：两种上下文里的下场完全不同，而且判断要用到当前
        // 节点，放在模式分派之前最省事。
        if let Token::Cdata(text) = token {
            if self
                .adjusted_current_node()
                .is_some_and(|id| self.is_foreign_element(id))
            {
                self.insert_foreign_text(&text);
            } else {
                // HTML 内容里这是一次解析错误，按不合法注释收场，注释内容就是
                // 从 `<![` 后面一直读到第一个 `>` 的那段。
                let node = self
                    .document
                    .create_comment(format!("[CDATA[{text}]]"));
                self.insert_at_appropriate_place(node);
            }
            return;
        }

        // 外来内容走另一套规则，和插入模式无关。
        if self.in_foreign_content(&token) {
            self.foreign_content(token);
            return;
        }

        self.process_token_html(token);
    }

    /// 按插入模式处理一个记号，不过外来内容那一关。
    ///
    /// 单独拆出来是因为外来内容的结束标签有时要把记号「交给 HTML 那一侧」——
    /// 规范说的是一次直接调用，不是把记号重新丢回入口。丢回入口的话，当前
    /// 节点还是外来元素，判定又会把它送回外来那一侧，来回弹到栈溢出。
    fn process_token_html(&mut self, token: Token) {
        // 攒表格文本的模式要拦住字符记号，先处理掉。
        if self.mode == InsertionMode::InTableText && !matches!(token, Token::Character(_)) {
            self.flush_pending_table_text();
        }

        match self.mode {
            InsertionMode::Initial => self.mode_initial(token),
            InsertionMode::BeforeHtml => self.mode_before_html(token),
            InsertionMode::BeforeHead => self.mode_before_head(token),
            InsertionMode::InHead => self.mode_in_head(token),
            InsertionMode::AfterHead => self.mode_after_head(token),
            InsertionMode::InBody => self.mode_in_body(token),
            InsertionMode::Text => self.mode_text(token),
            InsertionMode::InTable => self.mode_in_table(token),
            InsertionMode::InTableText => self.mode_in_table_text(token),
            InsertionMode::InCaption => self.mode_in_caption(token),
            InsertionMode::InColumnGroup => self.mode_in_column_group(token),
            InsertionMode::InTableBody => self.mode_in_table_body(token),
            InsertionMode::InRow => self.mode_in_row(token),
            InsertionMode::InCell => self.mode_in_cell(token),
            InsertionMode::InSelect => self.mode_in_select(token),
            InsertionMode::InSelectInTable => self.mode_in_select_in_table(token),
            InsertionMode::AfterBody => self.mode_after_body(token),
            InsertionMode::AfterAfterBody => self.mode_after_after_body(token),
        }
    }

    /// 初始模式：处理 DOCTYPE、注释与空白。
    fn mode_initial(&mut self, token: Token) {
        match token {
            Token::Character(text) if text.trim().is_empty() => {}
            Token::Comment(text) => {
                let node = self.document.create_comment(text);
                self.document.append_child(self.document.root(), node);
            }
            Token::Doctype(doctype) => {
                self.apply_doctype(&doctype);
                self.mode = InsertionMode::BeforeHtml;
            }
            other => {
                // 没有 DOCTYPE，按怪异模式处理。
                self.quirks = true;
                self.mode = InsertionMode::BeforeHtml;
                self.process_token(other);
            }
        }
    }

    /// 根据 DOCTYPE 判断是否启用怪异模式。
    fn apply_doctype(&mut self, doctype: &Doctype) {
        let node = self.document.create(NodeData::Doctype(doctype.clone()));
        self.document.append_child(self.document.root(), node);

        let name_ok = doctype.name == "html";
        let legacy_public = doctype
            .public_id
            .as_deref()
            .is_some_and(is_quirky_public_id);
        let missing_system = doctype.system_id.is_none()
            && doctype.public_id.as_deref().is_some_and(|id| {
                id.starts_with("-//W3C//DTD HTML 4.01 Frameset//")
                    || id.starts_with("-//W3C//DTD HTML 4.01 Transitional//")
            });
        self.quirks = doctype.force_quirks || !name_ok || legacy_public || missing_system;
    }

    /// 建立 `html` 元素之前。
    fn mode_before_html(&mut self, token: Token) {
        match token {
            Token::Doctype(_) => {}
            Token::Comment(text) => {
                let node = self.document.create_comment(text);
                self.document.append_child(self.document.root(), node);
            }
            Token::Character(text) if text.trim().is_empty() => {}
            Token::StartTag(tag) if tag.name == "html" => {
                let node = self.create_element("html", &tag.attributes);
                self.document.append_child(self.document.root(), node);
                self.push(node);
                self.mode = InsertionMode::BeforeHead;
            }
            Token::EndTag(tag) if !matches!(tag.name.as_str(), "head" | "body" | "html" | "br") => {
            }
            other => {
                let node = self.create_element("html", &[]);
                self.document.append_child(self.document.root(), node);
                self.push(node);
                self.mode = InsertionMode::BeforeHead;
                self.process_token(other);
            }
        }
    }

    /// 建立 `head` 元素之前。
    fn mode_before_head(&mut self, token: Token) {
        match token {
            Token::Character(text) if text.trim().is_empty() => {}
            Token::Comment(text) => {
                let node = self.document.create_comment(text);
                self.insert_at_appropriate_place(node);
            }
            Token::StartTag(tag) if tag.name == "html" => self.mode_in_body(Token::StartTag(tag)),
            Token::StartTag(tag) if tag.name == "head" => {
                let node = self.insert_html_element(&tag);
                self.head = Some(node);
                self.mode = InsertionMode::InHead;
            }
            Token::EndTag(tag) if !matches!(tag.name.as_str(), "head" | "body" | "html" | "br") => {
            }
            other => {
                let node = self.insert_element("head", &[], Namespace::Html);
                self.head = Some(node);
                self.mode = InsertionMode::InHead;
                self.process_token(other);
            }
        }
    }

    /// `head` 内部。
    fn mode_in_head(&mut self, token: Token) {
        match token {
            Token::Character(text) if text.trim().is_empty() => self.insert_text(&text),
            Token::Comment(text) => {
                let node = self.document.create_comment(text);
                self.insert_at_appropriate_place(node);
            }
            Token::Doctype(_) => {}
            Token::StartTag(tag) => match tag.name.as_str() {
                "html" => self.mode_in_body(Token::StartTag(tag)),
                "base" | "basefont" | "bgsound" | "link" | "meta" => {
                    let node = self.insert_html_element(&tag);
                    self.pop();
                    let _ = node;
                }
                "title" => self.parse_raw_text(&tag, RawTextMode::Rcdata),
                "style" | "noframes" => self.parse_raw_text(&tag, RawTextMode::Rawtext),
                "script" => self.parse_raw_text(&tag, RawTextMode::ScriptData),
                "noscript" => {
                    // 脚本标志按关闭算，这是规范对「脚本关闭」的规定：内容
                    // 整段当文本读，`</noscript>` 就是它的结束标记。脚本开着
                    // 时另有「head 里的 noscript」那一套插入模式，本项目不跑
                    // 脚本，所以按关闭那一套实现。
                    self.parse_raw_text(&tag, RawTextMode::Rawtext);
                }
                "head" => {}
                _ => {
                    self.pop();
                    self.mode = InsertionMode::AfterHead;
                    self.process_token(Token::StartTag(tag));
                }
            },
            Token::EndTag(tag) => match tag.name.as_str() {
                "head" => {
                    self.pop();
                    self.mode = InsertionMode::AfterHead;
                }
                "body" | "html" | "br" => {
                    self.pop();
                    self.mode = InsertionMode::AfterHead;
                    self.process_token(Token::EndTag(tag));
                }
                "template" => {}
                _ => {}
            },
            other => {
                self.pop();
                self.mode = InsertionMode::AfterHead;
                self.process_token(other);
            }
        }
    }

    /// `head` 之后、`body` 之前。
    fn mode_after_head(&mut self, token: Token) {
        match token {
            Token::Character(text) if text.trim().is_empty() => self.insert_text(&text),
            Token::Comment(text) => {
                let node = self.document.create_comment(text);
                self.insert_at_appropriate_place(node);
            }
            Token::Doctype(_) => {}
            Token::StartTag(tag) => match tag.name.as_str() {
                "html" => self.mode_in_body(Token::StartTag(tag)),
                "body" => {
                    let node = self.insert_html_element(&tag);
                    let _ = node;
                    self.mode = InsertionMode::InBody;
                }
                "frameset" => {
                    self.insert_html_element(&tag);
                }
                "base" | "basefont" | "bgsound" | "link" | "meta" | "noframes" | "script"
                | "style" | "title" => {
                    // 这些标签属于 head，临时压回 head 处理。
                    let head = self.head;
                    if let Some(head) = head {
                        self.push(head);
                        self.mode_in_head(Token::StartTag(tag));
                        self.remove_from_stack(head);
                    }
                }
                "head" => {}
                _ => {
                    let node = self.insert_element("body", &[], Namespace::Html);
                    let _ = node;
                    self.mode = InsertionMode::InBody;
                    self.process_token(Token::StartTag(tag));
                }
            },
            Token::EndTag(tag) => match tag.name.as_str() {
                "body" | "html" | "br" => {
                    let node = self.insert_element("body", &[], Namespace::Html);
                    let _ = node;
                    self.mode = InsertionMode::InBody;
                    self.process_token(Token::EndTag(tag));
                }
                "template" => {}
                _ => {}
            },
            other => {
                let node = self.insert_element("body", &[], Namespace::Html);
                let _ = node;
                self.mode = InsertionMode::InBody;
                self.process_token(other);
            }
        }
    }

    /// 解析原始文本元素：插入元素后切换词法模式，读完内容再切回来。
    fn parse_raw_text(&mut self, tag: &Tag, mode: RawTextMode) {
        let node = self.insert_html_element(tag);
        let _ = node;
        self.tokenizer.set_raw_text_mode(mode, &tag.name);
        self.original_mode = self.mode;
        self.mode = InsertionMode::Text;
    }

    /// 原始文本模式：把文本塞进当前元素，遇到结束标签就回来。
    fn mode_text(&mut self, token: Token) {
        match token {
            Token::Character(text) => {
                if let Some(current) = self.current_node() {
                    let node = self.document.create_text(text);
                    self.document.append_child(current, node);
                }
            }
            Token::EndTag(_) => {
                self.pop();
                self.mode = self.original_mode;
            }
            // 原始文本里出现其他记号说明输入提前结束了，按规范把元素闭合掉。
            _ => {
                self.pop();
                self.mode = self.original_mode;
            }
        }
    }
}

impl TreeBuilder<'_> {
    /// `body` 内部，绝大多数标签都在这里处理。
    fn mode_in_body(&mut self, token: Token) {
        match token {
            // CDATA 在进插入模式之前就被 process_token 分流掉了，走不到这里。
            Token::Cdata(_) => {}
            Token::Character(text) => {
                let mut text = text;
                if self.skip_next_newline {
                    self.skip_next_newline = false;
                    if let Some(rest) = text.strip_prefix('\n') {
                        text = rest.to_string();
                    } else if let Some(rest) = text.strip_prefix("\r\n") {
                        text = rest.to_string();
                    }
                }
                // 空字符按规范直接丢掉。
                if text.contains('\0') {
                    text = text.chars().filter(|c| *c != '\0').collect();
                }
                if text.is_empty() {
                    return;
                }
                self.reconstruct_formatting();
                self.insert_text(&text);
            }
            Token::Comment(text) => {
                let node = self.document.create_comment(text);
                self.insert_at_appropriate_place(node);
            }
            Token::Doctype(_) => {}
            Token::StartTag(tag) => self.in_body_start_tag(tag),
            Token::EndTag(tag) => self.in_body_end_tag(tag),
            Token::Eof => {}
        }
    }

    /// `body` 内部遇到的起始标签。
    fn in_body_start_tag(&mut self, tag: Tag) {
        match tag.name.as_str() {
            "html" => {
                // 已经在文档里了，只合并属性。
                if let Some(root) = self.open_elements.first().copied()
                    && let Some(element) = self.document.element_mut(root)
                {
                    for attribute in &tag.attributes {
                        if !element.has_attribute(&attribute.name) {
                            element.attributes.push(attribute.clone());
                        }
                    }
                }
            }
            "base" | "basefont" | "bgsound" | "link" | "meta" | "noframes" | "script" | "style"
            | "title" => self.mode_in_head(Token::StartTag(tag)),

            "body" => {
                // 重复出现的 body 只合并属性，不新建元素。
                if let Some(body) = self.find_second_on_stack()
                    && let Some(element) = self.document.element_mut(body)
                {
                    for attribute in &tag.attributes {
                        if !element.has_attribute(&attribute.name) {
                            element.attributes.push(attribute.clone());
                        }
                    }
                }
            }
            "frameset" => {
                // 用 frameset 取代 body，这里简化成直接在 body 位置插入。
                if let Some(body) = self.find_second_on_stack() {
                    self.document.detach(body);
                    self.remove_from_stack(body);
                }
                self.insert_html_element(&tag);
            }

            "address" | "article" | "aside" | "blockquote" | "center" | "details" | "dialog"
            | "dir" | "div" | "dl" | "fieldset" | "figcaption" | "figure" | "footer" | "header"
            | "hgroup" | "main" | "menu" | "nav" | "ol" | "p" | "search" | "section"
            | "summary" | "ul" => {
                self.close_p_element();
                self.insert_html_element(&tag);
            }

            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                self.close_p_element();
                // 紧挨着的另一个标题先把前一个收掉。
                if let Some(current) = self.current_node()
                    && let Some(name) = self.document.node(current).tag_name()
                    && matches!(name, "h1" | "h2" | "h3" | "h4" | "h5" | "h6")
                {
                    self.pop();
                }
                self.insert_html_element(&tag);
            }

            "pre" | "listing" => {
                self.close_p_element();
                self.insert_html_element(&tag);
                self.skip_next_newline = true;
            }

            "form" => {
                self.close_p_element();
                self.insert_html_element(&tag);
            }

            "li" => {
                self.close_p_element();
                // 找同级的 li，把它收掉。
                let mut index = self.open_elements.len();
                while index > 0 {
                    index -= 1;
                    let id = self.open_elements[index];
                    match self.document.node(id).tag_name() {
                        Some("li") => {
                            self.generate_implied_end_tags(Some("li"));
                            self.pop_until("li");
                            break;
                        }
                        Some(name)
                            if is_special(name) && !matches!(name, "address" | "div" | "p") =>
                        {
                            break;
                        }
                        _ => {}
                    }
                }
                self.close_p_element();
                self.insert_html_element(&tag);
            }

            "dd" | "dt" => {
                self.close_p_element();
                let mut index = self.open_elements.len();
                while index > 0 {
                    index -= 1;
                    let id = self.open_elements[index];
                    match self.document.node(id).tag_name() {
                        Some(name @ ("dd" | "dt")) => {
                            let name = name.to_string();
                            self.generate_implied_end_tags(Some(&name));
                            self.pop_until(&name);
                            break;
                        }
                        Some(name)
                            if is_special(name) && !matches!(name, "address" | "div" | "p") =>
                        {
                            break;
                        }
                        _ => {}
                    }
                }
                self.close_p_element();
                self.insert_html_element(&tag);
            }

            "plaintext" => {
                self.close_p_element();
                self.insert_html_element(&tag);
                self.tokenizer.set_raw_text_mode(RawTextMode::Plaintext, "");
            }

            "button" => {
                if self.has_in_scope("button") {
                    self.generate_implied_end_tags(None);
                    self.pop_until("button");
                }
                self.reconstruct_formatting();
                self.insert_html_element(&tag);
            }

            "table" => {
                self.close_p_element();
                self.insert_html_element(&tag);
                self.mode = InsertionMode::InTable;
            }

            "area" | "br" | "embed" | "img" | "keygen" | "wbr" => {
                self.reconstruct_formatting();
                self.insert_html_element(&tag);
                self.pop();
            }

            "input" | "param" | "source" | "track" => {
                self.insert_html_element(&tag);
                self.pop();
            }

            "hr" => {
                self.close_p_element();
                self.insert_html_element(&tag);
                self.pop();
            }

            "image" => {
                // 历史遗留的写法，按 img 处理。
                let mut tag = tag;
                tag.name = "img".to_string();
                self.in_body_start_tag(tag);
            }

            "textarea" => {
                self.insert_html_element(&tag);
                self.tokenizer
                    .set_raw_text_mode(RawTextMode::Rcdata, "textarea");
                self.skip_next_newline = true;
                self.original_mode = self.mode;
                self.mode = InsertionMode::Text;
            }

            "xmp" => {
                self.close_p_element();
                self.reconstruct_formatting();
                self.insert_html_element(&tag);
                self.tokenizer
                    .set_raw_text_mode(RawTextMode::Rawtext, "xmp");
                self.original_mode = self.mode;
                self.mode = InsertionMode::Text;
            }

            "iframe" | "noembed" => {
                self.insert_html_element(&tag);
                self.tokenizer
                    .set_raw_text_mode(RawTextMode::Rawtext, &tag.name);
                self.original_mode = self.mode;
                self.mode = InsertionMode::Text;
            }

            "select" => {
                self.reconstruct_formatting();
                self.insert_html_element(&tag);
                self.mode = if self.mode == InsertionMode::InTable
                    || self.mode == InsertionMode::InCaption
                    || self.mode == InsertionMode::InTableBody
                    || self.mode == InsertionMode::InRow
                    || self.mode == InsertionMode::InCell
                {
                    InsertionMode::InSelectInTable
                } else {
                    InsertionMode::InSelect
                };
            }

            "optgroup" | "option" => {
                if self
                    .current_node()
                    .is_some_and(|id| self.document.node(id).is_element("option"))
                {
                    self.pop();
                }
                self.reconstruct_formatting();
                self.insert_html_element(&tag);
            }

            "rb" | "rtc" => {
                if self.has_in_scope("ruby") {
                    self.generate_implied_end_tags(None);
                }
                self.insert_html_element(&tag);
            }

            "rp" | "rt" => {
                if self.has_in_scope("ruby") {
                    self.generate_implied_end_tags(Some("rtc"));
                }
                self.insert_html_element(&tag);
            }

            "math" => {
                self.reconstruct_formatting();
                let node = self.insert_element("math", &tag.attributes, Namespace::MathMl);
                let _ = node;
            }

            "svg" => {
                self.reconstruct_formatting();
                let node = self.insert_element("svg", &tag.attributes, Namespace::Svg);
                let _ = node;
            }

            "a" => {
                // 活动表里已经有 a 时，先把旧的按收养机构算法收掉。
                let existing = self.active_formatting.iter().rposition(
                    |entry| matches!(entry, FormattingEntry::Element { name, .. } if name == "a"),
                );
                if let Some(index) = existing
                    && let FormattingEntry::Element { id, .. } = self.active_formatting[index]
                {
                    self.adoption_agency("a");
                    self.active_formatting
                            .retain(|entry| !matches!(entry, FormattingEntry::Element { id: other, .. } if *other == id));
                }
                self.reconstruct_formatting();
                let id = self.insert_html_element(&tag);
                self.push_formatting(id, "a", &tag.attributes);
            }

            "b" | "big" | "code" | "em" | "font" | "i" | "s" | "small" | "strike" | "strong"
            | "tt" | "u" => {
                self.reconstruct_formatting();
                let id = self.insert_html_element(&tag);
                self.push_formatting(id, &tag.name, &tag.attributes);
            }

            "nobr" => {
                self.reconstruct_formatting();
                if self.has_in_scope("nobr") {
                    self.adoption_agency("nobr");
                    self.reconstruct_formatting();
                }
                let id = self.insert_html_element(&tag);
                self.push_formatting(id, "nobr", &tag.attributes);
            }

            "applet" | "marquee" | "object" => {
                self.reconstruct_formatting();
                let node = self.insert_html_element(&tag);
                let _ = node;
                self.active_formatting.push(FormattingEntry::Marker);
            }

            "template" => {}

            // 表格相关的标签出现在这里说明位置不对，直接忽略。
            "caption" | "col" | "colgroup" | "frame" | "head" | "tbody" | "td" | "tfoot" | "th"
            | "thead" | "tr" => {}

            _ => {
                self.reconstruct_formatting();
                self.insert_html_element(&tag);
            }
        }
    }

    /// `body` 内部遇到的结束标签。
    fn in_body_end_tag(&mut self, tag: Tag) {
        match tag.name.as_str() {
            "body" => {
                if self.has_in_scope("body") {
                    self.mode = InsertionMode::AfterBody;
                }
            }
            "html" => {
                if self.has_in_scope("body") {
                    self.mode = InsertionMode::AfterBody;
                    self.process_token(Token::EndTag(tag));
                }
            }
            "address" | "article" | "aside" | "blockquote" | "button" | "center" | "details"
            | "dialog" | "dir" | "div" | "dl" | "fieldset" | "figcaption" | "figure" | "footer"
            | "header" | "hgroup" | "listing" | "main" | "menu" | "nav" | "ol" | "pre"
            | "search" | "section" | "summary" | "ul" => {
                let name = tag.name.clone();
                if self.has_in_scope(&name) {
                    self.generate_implied_end_tags(None);
                    self.pop_until(&name);
                }
            }
            "form" => {
                if self.has_in_scope("form") {
                    self.generate_implied_end_tags(None);
                    self.pop_until("form");
                }
            }
            "p" => {
                if !self.has_in_button_scope("p") {
                    // 没有对应的起始标签时补一个再收掉。
                    self.insert_element("p", &[], Namespace::Html);
                }
                self.generate_implied_end_tags(Some("p"));
                self.pop_until("p");
            }
            "li" => {
                if self.has_in_list_item_scope("li") {
                    self.generate_implied_end_tags(Some("li"));
                    self.pop_until("li");
                }
            }
            "dd" | "dt" => {
                let name = tag.name.clone();
                if self.has_in_scope(&name) {
                    self.generate_implied_end_tags(Some(&name));
                    self.pop_until(&name);
                }
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                let found = ["h1", "h2", "h3", "h4", "h5", "h6"]
                    .iter()
                    .any(|name| self.has_in_scope(name));
                if found {
                    self.generate_implied_end_tags(None);
                    while let Some(id) = self.current_node() {
                        let is_heading = self.document.node(id).tag_name().is_some_and(|name| {
                            matches!(name, "h1" | "h2" | "h3" | "h4" | "h5" | "h6")
                        });
                        self.pop();
                        if is_heading {
                            break;
                        }
                    }
                }
            }
            "a" | "b" | "big" | "code" | "em" | "font" | "i" | "nobr" | "s" | "small"
            | "strike" | "strong" | "tt" | "u" => {
                self.adoption_agency(&tag.name);
            }
            "applet" | "marquee" | "object" => {
                let name = tag.name.clone();
                if self.has_in_scope(&name) {
                    self.generate_implied_end_tags(None);
                    self.pop_until(&name);
                    self.clear_formatting_to_marker();
                }
            }
            "br" => {
                // `</br>` 按 `<br>` 处理。
                let tag = Tag {
                    name: "br".to_string(),
                    attributes: Vec::new(),
                    self_closing: false,
                };
                self.in_body_start_tag(tag);
            }
            "template" => {}
            _ => self.any_other_end_tag(&tag.name),
        }
    }

    /// 通用结束标签的处理：从栈顶往回找同名元素，中途遇到特殊元素就放弃。
    fn any_other_end_tag(&mut self, name: &str) {
        let mut index = self.open_elements.len();
        while index > 0 {
            index -= 1;
            let id = self.open_elements[index];
            let Some(tag) = self.document.node(id).tag_name() else {
                continue;
            };
            if tag == name {
                self.generate_implied_end_tags(Some(name));
                while self.open_elements.len() > index {
                    self.pop();
                }
                return;
            }
            if is_special(tag) {
                return;
            }
        }
    }

    /// 重建活动格式化元素表。
    ///
    /// 块级元素会打断 `<b>` 这类格式化的作用范围，之后再出现文本时按规范
    /// 要把它们重新建出来，否则 `<b>a<div>b</div>c` 里的 c 就丢了样式。
    fn reconstruct_formatting(&mut self) {
        if self.active_formatting.is_empty() {
            return;
        }
        let last = self.active_formatting.len() - 1;
        if matches!(self.active_formatting[last], FormattingEntry::Marker) {
            return;
        }
        if let FormattingEntry::Element { id, .. } = self.active_formatting[last]
            && self.is_on_stack(id)
        {
            return;
        }

        // 往回找到第一个还在栈上或者标记的位置。
        let mut index = last;
        while index > 0 {
            index -= 1;
            match &self.active_formatting[index] {
                FormattingEntry::Marker => {
                    index += 1;
                    break;
                }
                FormattingEntry::Element { id, .. } if self.is_on_stack(*id) => {
                    index += 1;
                    break;
                }
                FormattingEntry::Element { .. } => {}
            }
        }

        // 从找到的位置开始逐个重建。
        while index < self.active_formatting.len() {
            let (name, attributes) = match &self.active_formatting[index] {
                FormattingEntry::Element {
                    name, attributes, ..
                } => (name.clone(), attributes.clone()),
                FormattingEntry::Marker => {
                    index += 1;
                    continue;
                }
            };
            let id = self.insert_element(&name, &attributes, Namespace::Html);
            self.active_formatting[index] = FormattingEntry::Element {
                id,
                name,
                attributes,
            };
            index += 1;
        }
    }

    /// 把元素加进活动格式化元素表。
    fn push_formatting(&mut self, id: NodeId, name: &str, attributes: &[Attribute]) {
        self.active_formatting.push(FormattingEntry::Element {
            id,
            name: name.to_string(),
            attributes: attributes.to_vec(),
        });
    }

    /// 清除活动格式化元素表里到最近一个标记为止的内容。
    fn clear_formatting_to_marker(&mut self) {
        while let Some(entry) = self.active_formatting.pop() {
            if matches!(entry, FormattingEntry::Marker) {
                break;
            }
        }
    }

    /// 收养机构算法，处理交错的格式化元素。
    ///
    /// 典型输入是 `<b>1<p>2</b>3</p>`：`</b>` 出现时 `p` 还在栈上，
    /// 规范要求把 `b` 就地拆开，让 `p` 之后的内容另起一个 `b` 包住。
    fn adoption_agency(&mut self, subject: &str) {
        // 情况一：当前节点就是目标元素，而且不在活动表里，直接弹出。
        if let Some(current) = self.current_node()
            && self.document.node(current).is_element(subject)
            && !self
                .active_formatting
                .iter()
                .any(|entry| matches!(entry, FormattingEntry::Element { id, .. } if *id == current))
        {
            self.pop();
            return;
        }

        // 外层循环限制八次，避免畸形输入把解析拖住。
        for _ in 0..8 {
            // 找活动表里最近一个同名的格式化元素。
            let Some(formatting_index) = self.active_formatting.iter().rposition(
                |entry| matches!(entry, FormattingEntry::Element { name, .. } if name == subject),
            ) else {
                // 活动表里没有，退回通用结束标签处理。
                self.any_other_end_tag(subject);
                return;
            };

            let formatting_element = match &self.active_formatting[formatting_index] {
                FormattingEntry::Element { id, .. } => *id,
                FormattingEntry::Marker => return,
            };

            // 不在栈上就从活动表里删掉。
            if !self.is_on_stack(formatting_element) {
                self.active_formatting.remove(formatting_index);
                return;
            }

            if !self.has_in_scope(subject) {
                return;
            }

            // 在栈上找到目标元素的位置。
            let Some(formatting_stack_index) = self
                .open_elements
                .iter()
                .position(|id| *id == formatting_element)
            else {
                return;
            };

            // 找最上层的块级元素，收养后的内容要放进它里面。
            // 规范说取「比目标元素晚压栈的那些里最靠上的一个」——栈里越靠上
            // 压得越早，所以是从前往后找第一个，不是从后往前。
            let furthest_block = self.open_elements[formatting_stack_index + 1..]
                .iter()
                .copied()
                .find(|id| self.document.node(*id).tag_name().is_some_and(is_special));
            let Some(furthest_block) = furthest_block else {
                // 没有块级元素隔着：从当前节点一路弹到目标元素为止，含它自己。
                //
                // 这里不能只把目标从栈中间抠掉。栈里排在它上面的元素（`<a><b>`
                // 里的 b 就是）也得跟着出栈，否则它们被当成还开着，后面重建
                // 格式化元素时就找不回正确的位置，内容会并进旧的元素里。
                while let Some(popped) = self.pop() {
                    if popped == formatting_element {
                        break;
                    }
                }
                self.active_formatting.remove(formatting_index);
                return;
            };

            let common_ancestor = if formatting_stack_index == 0 {
                self.document.root()
            } else {
                self.open_elements[formatting_stack_index - 1]
            };

            let mut bookmark = formatting_index;
            let mut last_node = furthest_block;
            let mut node = furthest_block;
            let mut inner = 0;

            loop {
                inner += 1;
                // 沿栈往上走一格。
                let Some(node_index) = self.open_elements.iter().position(|id| *id == node) else {
                    break;
                };
                if node_index == 0 {
                    break;
                }
                node = self.open_elements[node_index - 1];
                if node == formatting_element {
                    break;
                }

                // 内层超过三次以后，把活动表里的中间项清掉。
                let active_index = self.active_formatting.iter().position(
                    |entry| matches!(entry, FormattingEntry::Element { id, .. } if *id == node),
                );
                if inner > 3
                    && let Some(index) = active_index
                {
                    self.active_formatting.remove(index);
                    if index < bookmark {
                        bookmark -= 1;
                    }
                }

                let Some(active_index) = active_index else {
                    self.remove_from_stack(node);
                    continue;
                };

                // 给这个节点建一个副本，替换活动表与栈上的位置。
                let (name, attributes) = match &self.active_formatting[active_index] {
                    FormattingEntry::Element {
                        name, attributes, ..
                    } => (name.clone(), attributes.clone()),
                    FormattingEntry::Marker => break,
                };
                let replacement = self.document.create_element({
                    let mut element = Element::new(&name);
                    element.attributes = attributes.clone();
                    element
                });
                self.active_formatting[active_index] = FormattingEntry::Element {
                    id: replacement,
                    name,
                    attributes,
                };
                if let Some(stack_index) = self.open_elements.iter().position(|id| *id == node) {
                    self.open_elements[stack_index] = replacement;
                }

                if last_node == furthest_block {
                    bookmark = active_index + 1;
                }
                self.document.detach(last_node);
                self.document.append_child(replacement, last_node);
                last_node = replacement;
            }

            // 把最后一个节点挪到共同祖先下面。
            self.document.detach(last_node);
            self.insert_at_target(last_node, common_ancestor);

            // 为原目标元素建一个副本，用来装它原来的子节点。
            let (name, attributes) = match &self.active_formatting[formatting_index] {
                FormattingEntry::Element {
                    name, attributes, ..
                } => (name.clone(), attributes.clone()),
                FormattingEntry::Marker => return,
            };
            let replacement = self.document.create_element({
                let mut element = Element::new(&name);
                element.attributes = attributes.clone();
                element
            });
            self.document.reparent_children(furthest_block, replacement);
            self.document.append_child(furthest_block, replacement);

            // 更新活动表与栈。
            self.active_formatting.remove(formatting_index);
            let insert_at = bookmark.min(self.active_formatting.len());
            let replacement_id = replacement;
            self.active_formatting.insert(
                insert_at,
                FormattingEntry::Element {
                    id: replacement_id,
                    name,
                    attributes,
                },
            );

            self.remove_from_stack(formatting_element);
            if let Some(block_index) = self
                .open_elements
                .iter()
                .position(|id| *id == furthest_block)
            {
                self.open_elements.insert(block_index + 1, replacement_id);
            }
        }
    }

    /// 把节点插到指定父节点下面，需要寄养时按表格规则处理。
    fn insert_at_target(&mut self, node: NodeId, parent: NodeId) {
        if matches!(
            self.document.node(parent).tag_name(),
            Some("table" | "tbody" | "tfoot" | "thead" | "tr")
        ) {
            self.insert_at_appropriate_place(node);
        } else {
            self.document.append_child(parent, node);
        }
    }

    /// 找栈上第二个元素，通常是 `body`。
    fn find_second_on_stack(&self) -> Option<NodeId> {
        self.open_elements.get(1).copied()
    }

    // ---- 表格相关模式 ----

    /// 表格内部。
    fn mode_in_table(&mut self, token: Token) {
        match token {
            // CDATA 在进插入模式之前就被 process_token 分流掉了，走不到这里。
            Token::Cdata(_) => {}
            Token::Character(_) => {
                // 表格里的文本要先攒着，全是空白才留在表格里。
                self.pending_table_text = Some(PendingTableText {
                    has_non_whitespace: false,
                    text: String::new(),
                });
                self.original_mode = self.mode;
                self.mode = InsertionMode::InTableText;
                self.process_token(token);
            }
            Token::Comment(text) => {
                let node = self.document.create_comment(text);
                self.insert_at_appropriate_place(node);
            }
            Token::Doctype(_) => {}
            Token::StartTag(tag) => match tag.name.as_str() {
                "caption" => {
                    self.clear_stack_to_table_context();
                    self.active_formatting.push(FormattingEntry::Marker);
                    self.insert_html_element(&tag);
                    self.mode = InsertionMode::InCaption;
                }
                "colgroup" => {
                    self.clear_stack_to_table_context();
                    self.insert_html_element(&tag);
                    self.mode = InsertionMode::InColumnGroup;
                }
                "col" => {
                    self.clear_stack_to_table_context();
                    self.insert_element("colgroup", &[], Namespace::Html);
                    self.mode = InsertionMode::InColumnGroup;
                    self.process_token(Token::StartTag(tag));
                }
                "tbody" | "tfoot" | "thead" => {
                    self.clear_stack_to_table_context();
                    self.insert_html_element(&tag);
                    self.mode = InsertionMode::InTableBody;
                }
                "td" | "th" | "tr" => {
                    self.clear_stack_to_table_context();
                    self.insert_element("tbody", &[], Namespace::Html);
                    self.mode = InsertionMode::InTableBody;
                    self.process_token(Token::StartTag(tag));
                }
                "table" => {
                    // 嵌套表格：把当前表格收掉再来一次。
                    if self.has_in_table_scope("table") {
                        self.pop_until("table");
                        self.mode = InsertionMode::InBody;
                        self.process_token(Token::StartTag(tag));
                    }
                }
                "style" | "script" | "template" => self.mode_in_head(Token::StartTag(tag)),
                "input" => {
                    self.insert_html_element(&tag);
                    self.pop();
                }
                "form" => {
                    self.insert_html_element(&tag);
                    self.pop();
                }
                _ => {
                    // 不属于表格的内容按寄养规则处理，挪到表格之前。
                    self.mode = InsertionMode::InBody;
                    self.with_foster_parenting(|builder| {
                        builder.process_token(Token::StartTag(tag))
                    });
                    self.mode = InsertionMode::InTable;
                }
            },
            Token::EndTag(tag) => match tag.name.as_str() {
                "table" => {
                    if self.has_in_table_scope("table") {
                        self.pop_until("table");
                        self.mode = InsertionMode::InBody;
                    }
                }
                "body" | "caption" | "col" | "colgroup" | "html" | "tbody" | "td" | "tfoot"
                | "th" | "thead" | "tr" => {}
                _ => {
                    self.mode = InsertionMode::InBody;
                    self.with_foster_parenting(|builder| builder.process_token(Token::EndTag(tag)));
                    self.mode = InsertionMode::InTable;
                }
            },
            Token::Eof => {}
        }
    }

    /// 把栈收拢到表格上下文，多余的元素弹掉。
    fn clear_stack_to_table_context(&mut self) {
        while let Some(id) = self.current_node() {
            let stop = self
                .document
                .node(id)
                .tag_name()
                .is_some_and(|name| matches!(name, "table" | "html"));
            if stop {
                break;
            }
            self.pop();
        }
    }

    /// 把栈收拢到表格体的上下文。
    fn clear_stack_to_table_body_context(&mut self) {
        while let Some(id) = self.current_node() {
            let stop = self
                .document
                .node(id)
                .tag_name()
                .is_some_and(|name| matches!(name, "tbody" | "tfoot" | "thead" | "html"));
            if stop {
                break;
            }
            self.pop();
        }
    }

    /// 把栈收拢到表格行的上下文。
    fn clear_stack_to_table_row_context(&mut self) {
        while let Some(id) = self.current_node() {
            let stop = self
                .document
                .node(id)
                .tag_name()
                .is_some_and(|name| matches!(name, "tr" | "html"));
            if stop {
                break;
            }
            self.pop();
        }
    }

    /// 表格里的文本暂存模式。
    fn mode_in_table_text(&mut self, token: Token) {
        let Token::Character(text) = token else {
            return;
        };
        let Some(pending) = self.pending_table_text.as_mut() else {
            return;
        };
        // 空字符在正文里是直接丢掉的，攒表格文本这一路也要丢，不然它会
        // 跟着寄养到表格前面，凭空多出几个看不见的字符。
        let text: String = text.chars().filter(|c| *c != '\0').collect();
        if text.chars().any(|c| !c.is_whitespace()) {
            pending.has_non_whitespace = true;
        }
        pending.text.push_str(&text);
    }

    /// 把暂存的表格文本落下去。
    fn flush_pending_table_text(&mut self) {
        let Some(pending) = self.pending_table_text.take() else {
            return;
        };
        self.mode = self.original_mode;
        if pending.text.is_empty() {
            return;
        }
        if pending.has_non_whitespace {
            // 含非空白字符，按寄养规则把文本挪到表格之前。
            self.mode = InsertionMode::InBody;
            self.with_foster_parenting(|builder| builder.insert_text(&pending.text));
            self.mode = self.original_mode;
        } else {
            self.insert_text(&pending.text);
        }
    }

    /// `caption` 内部。
    fn mode_in_caption(&mut self, token: Token) {
        match &token {
            Token::EndTag(tag) if tag.name == "caption" => {
                if self.has_in_table_scope("caption") {
                    self.generate_implied_end_tags(None);
                    self.pop_until("caption");
                    self.clear_formatting_to_marker();
                    self.mode = InsertionMode::InTable;
                }
            }
            Token::StartTag(tag)
                if matches!(
                    tag.name.as_str(),
                    "caption"
                        | "col"
                        | "colgroup"
                        | "tbody"
                        | "td"
                        | "tfoot"
                        | "th"
                        | "thead"
                        | "tr"
                ) =>
            {
                if self.has_in_table_scope("caption") {
                    self.generate_implied_end_tags(None);
                    self.pop_until("caption");
                    self.clear_formatting_to_marker();
                    self.mode = InsertionMode::InTable;
                    self.process_token(token);
                }
            }
            Token::EndTag(tag) if tag.name == "table" => {
                if self.has_in_table_scope("caption") {
                    self.generate_implied_end_tags(None);
                    self.pop_until("caption");
                    self.clear_formatting_to_marker();
                    self.mode = InsertionMode::InTable;
                    self.process_token(token);
                }
            }
            Token::EndTag(tag)
                if matches!(
                    tag.name.as_str(),
                    "body"
                        | "col"
                        | "colgroup"
                        | "html"
                        | "tbody"
                        | "td"
                        | "tfoot"
                        | "th"
                        | "thead"
                        | "tr"
                ) => {}
            _ => self.mode_in_body(token),
        }
    }

    /// `colgroup` 内部。
    fn mode_in_column_group(&mut self, token: Token) {
        match &token {
            Token::Character(text) if text.trim().is_empty() => {
                let text = text.clone();
                self.insert_text(&text);
            }
            Token::Comment(_) => self.mode_in_body(token),
            Token::StartTag(tag) if tag.name == "col" => {
                let node = self.insert_html_element(match &token {
                    Token::StartTag(tag) => tag,
                    _ => unreachable!("已经确认是起始标签"),
                });
                let _ = node;
                self.pop();
            }
            Token::StartTag(tag) if tag.name == "html" => self.mode_in_body(token),
            Token::EndTag(tag) if tag.name == "colgroup" => {
                if self
                    .current_node()
                    .is_some_and(|id| !self.document.node(id).is_element("colgroup"))
                {
                    return;
                }
                self.pop();
                self.mode = InsertionMode::InTable;
            }
            Token::EndTag(tag) if tag.name == "col" => {}
            _ => {
                if self
                    .current_node()
                    .is_some_and(|id| self.document.node(id).is_element("colgroup"))
                {
                    self.pop();
                    self.mode = InsertionMode::InTable;
                    self.process_token(token);
                }
            }
        }
    }

    /// 表格体内部。
    fn mode_in_table_body(&mut self, token: Token) {
        match &token {
            Token::StartTag(tag) if tag.name == "tr" => {
                self.clear_stack_to_table_body_context();
                let tag = match token {
                    Token::StartTag(tag) => tag,
                    _ => unreachable!("已经确认是起始标签"),
                };
                self.insert_html_element(&tag);
                self.mode = InsertionMode::InRow;
            }
            Token::StartTag(tag) if matches!(tag.name.as_str(), "th" | "td") => {
                self.clear_stack_to_table_body_context();
                self.insert_element("tr", &[], Namespace::Html);
                self.mode = InsertionMode::InRow;
                self.process_token(token);
            }
            Token::StartTag(tag)
                if matches!(
                    tag.name.as_str(),
                    "caption" | "col" | "colgroup" | "tbody" | "tfoot" | "thead"
                ) =>
            {
                let in_scope = self
                    .stack_element_of("tbody")
                    .or_else(|| self.stack_element_of("thead"))
                    .or_else(|| self.stack_element_of("tfoot"))
                    .is_some();
                if in_scope {
                    self.clear_stack_to_table_body_context();
                    self.pop();
                    self.mode = InsertionMode::InTable;
                    self.process_token(token);
                }
            }
            Token::EndTag(tag) if matches!(tag.name.as_str(), "tbody" | "tfoot" | "thead") => {
                let name = tag.name.clone();
                if self.has_in_table_scope(&name) {
                    self.clear_stack_to_table_body_context();
                    self.pop();
                    self.mode = InsertionMode::InTable;
                }
            }
            Token::EndTag(tag) if tag.name == "table" => {
                let in_scope = self
                    .stack_element_of("tbody")
                    .or_else(|| self.stack_element_of("thead"))
                    .or_else(|| self.stack_element_of("tfoot"))
                    .is_some();
                if in_scope {
                    self.clear_stack_to_table_body_context();
                    self.pop();
                    self.mode = InsertionMode::InTable;
                    self.process_token(token);
                }
            }
            _ => self.mode_in_table(token),
        }
    }

    /// 表格行内部。
    fn mode_in_row(&mut self, token: Token) {
        match &token {
            Token::StartTag(tag) if matches!(tag.name.as_str(), "th" | "td") => {
                let tag = match token {
                    Token::StartTag(tag) => tag,
                    _ => unreachable!("已经确认是起始标签"),
                };
                self.insert_html_element(&tag);
                self.mode = InsertionMode::InCell;
                self.active_formatting.push(FormattingEntry::Marker);
            }
            Token::EndTag(tag) if tag.name == "tr" => {
                if self.has_in_table_scope("tr") {
                    self.clear_stack_to_table_row_context();
                    self.pop();
                    self.mode = InsertionMode::InTableBody;
                }
            }
            Token::StartTag(tag)
                if matches!(
                    tag.name.as_str(),
                    "caption" | "col" | "colgroup" | "tbody" | "tfoot" | "thead" | "tr"
                ) =>
            {
                if self.has_in_table_scope("tr") {
                    self.clear_stack_to_table_row_context();
                    self.pop();
                    self.mode = InsertionMode::InTableBody;
                    self.process_token(token);
                }
            }
            Token::EndTag(tag) if tag.name == "table" => {
                if self.has_in_table_scope("tr") {
                    self.clear_stack_to_table_row_context();
                    self.pop();
                    self.mode = InsertionMode::InTableBody;
                    self.process_token(token);
                }
            }
            Token::EndTag(tag) if matches!(tag.name.as_str(), "tbody" | "tfoot" | "thead") => {
                let name = tag.name.clone();
                if self.has_in_table_scope(&name) && self.has_in_table_scope("tr") {
                    self.clear_stack_to_table_row_context();
                    self.pop();
                    self.mode = InsertionMode::InTableBody;
                    self.process_token(token);
                }
            }
            _ => self.mode_in_table(token),
        }
    }

    /// 单元格内部。
    fn mode_in_cell(&mut self, token: Token) {
        match &token {
            Token::EndTag(tag) if matches!(tag.name.as_str(), "td" | "th") => {
                let name = tag.name.clone();
                if self.has_in_table_scope(&name) {
                    self.generate_implied_end_tags(None);
                    self.pop_until(&name);
                    self.clear_formatting_to_marker();
                    self.mode = InsertionMode::InRow;
                }
            }
            Token::StartTag(tag)
                if matches!(
                    tag.name.as_str(),
                    "caption"
                        | "col"
                        | "colgroup"
                        | "tbody"
                        | "td"
                        | "tfoot"
                        | "th"
                        | "thead"
                        | "tr"
                ) =>
            {
                let in_scope = self.has_in_table_scope("td") || self.has_in_table_scope("th");
                if in_scope {
                    self.close_cell();
                    self.process_token(token);
                }
            }
            Token::EndTag(tag)
                if matches!(
                    tag.name.as_str(),
                    "body" | "caption" | "col" | "colgroup" | "html"
                ) => {}
            Token::EndTag(tag)
                if matches!(
                    tag.name.as_str(),
                    "table" | "tbody" | "tfoot" | "thead" | "tr"
                ) =>
            {
                let name = tag.name.clone();
                if self.has_in_table_scope(&name) {
                    self.close_cell();
                    self.process_token(token);
                }
            }
            _ => self.mode_in_body(token),
        }
    }

    /// 关闭当前单元格。
    fn close_cell(&mut self) {
        self.generate_implied_end_tags(None);
        if self.has_in_table_scope("td") {
            self.pop_until("td");
        } else {
            self.pop_until("th");
        }
        self.clear_formatting_to_marker();
        self.mode = InsertionMode::InRow;
    }

    /// `select` 内部。
    fn mode_in_select(&mut self, token: Token) {
        match token {
            // CDATA 在进插入模式之前就被 process_token 分流掉了，走不到这里。
            Token::Cdata(_) => {}
            Token::Character(text) => self.insert_text(&text),
            Token::Comment(text) => {
                let node = self.document.create_comment(text);
                self.insert_at_appropriate_place(node);
            }
            Token::Doctype(_) => {}
            Token::StartTag(tag) => match tag.name.as_str() {
                "html" => self.mode_in_body(Token::StartTag(tag)),
                "option" => {
                    if self
                        .current_node()
                        .is_some_and(|id| self.document.node(id).is_element("option"))
                    {
                        self.pop();
                    }
                    self.insert_html_element(&tag);
                }
                "optgroup" => {
                    if self
                        .current_node()
                        .is_some_and(|id| self.document.node(id).is_element("option"))
                    {
                        self.pop();
                    }
                    if self
                        .current_node()
                        .is_some_and(|id| self.document.node(id).is_element("optgroup"))
                    {
                        self.pop();
                    }
                    self.insert_html_element(&tag);
                }
                "hr" => {
                    if self
                        .current_node()
                        .is_some_and(|id| self.document.node(id).is_element("option"))
                    {
                        self.pop();
                    }
                    if self
                        .current_node()
                        .is_some_and(|id| self.document.node(id).is_element("optgroup"))
                    {
                        self.pop();
                    }
                    self.insert_html_element(&tag);
                    self.pop();
                }
                "select" => {
                    // 嵌套的 select 按结束标签处理。
                    if self.has_in_scope("select") {
                        self.pop_until("select");
                        self.mode = InsertionMode::InBody;
                    }
                }
                "input" | "keygen" | "textarea" => {
                    if self.has_in_scope("select") {
                        self.pop_until("select");
                        self.mode = InsertionMode::InBody;
                        self.process_token(Token::StartTag(tag));
                    }
                }
                "script" | "template" => self.mode_in_head(Token::StartTag(tag)),
                _ => {}
            },
            Token::EndTag(tag) => match tag.name.as_str() {
                "optgroup" => {
                    if self
                        .current_node()
                        .is_some_and(|id| self.document.node(id).is_element("option"))
                        && self
                            .document
                            .parent(self.current_node().expect("刚刚确认有当前节点"))
                            .is_some_and(|parent| self.document.node(parent).is_element("optgroup"))
                    {
                        self.pop();
                    }
                    if self
                        .current_node()
                        .is_some_and(|id| self.document.node(id).is_element("optgroup"))
                    {
                        self.pop();
                    }
                }
                "option" => {
                    if self
                        .current_node()
                        .is_some_and(|id| self.document.node(id).is_element("option"))
                    {
                        self.pop();
                    }
                }
                "select" if self.has_in_scope("select") => {
                    self.pop_until("select");
                    self.mode = InsertionMode::InBody;
                }
                _ => {}
            },
            Token::Eof => {}
        }
    }

    /// `select` 出现在表格里。
    fn mode_in_select_in_table(&mut self, token: Token) {
        match &token {
            Token::StartTag(tag)
                if matches!(
                    tag.name.as_str(),
                    "caption" | "table" | "tbody" | "tfoot" | "thead" | "tr" | "td" | "th"
                ) =>
            {
                if self.has_in_table_scope("select") {
                    self.pop_until("select");
                    self.mode = InsertionMode::InBody;
                    self.process_token(token);
                }
            }
            Token::EndTag(tag)
                if matches!(
                    tag.name.as_str(),
                    "caption" | "table" | "tbody" | "tfoot" | "thead" | "tr" | "td" | "th"
                ) =>
            {
                let name = tag.name.clone();
                if self.has_in_table_scope(&name) && self.has_in_table_scope("select") {
                    self.pop_until("select");
                    self.mode = InsertionMode::InBody;
                    self.process_token(token);
                }
            }
            _ => self.mode_in_select(token),
        }
    }

    /// `body` 结束之后。
    fn mode_after_body(&mut self, token: Token) {
        match token {
            Token::Character(text) if text.trim().is_empty() => {
                self.mode_in_body(Token::Character(text))
            }
            Token::Comment(text) => {
                let root = self
                    .open_elements
                    .first()
                    .copied()
                    .unwrap_or(self.document.root());
                let node = self.document.create_comment(text);
                self.document.append_child(root, node);
            }
            Token::Doctype(_) => {}
            Token::EndTag(tag) if tag.name == "html" => {
                self.mode = InsertionMode::AfterAfterBody;
            }
            other => {
                self.mode = InsertionMode::InBody;
                self.process_token(other);
            }
        }
    }

    /// 整个文档结束之后。
    fn mode_after_after_body(&mut self, token: Token) {
        match token {
            Token::Comment(text) => {
                let node = self.document.create_comment(text);
                self.document.append_child(self.document.root(), node);
            }
            Token::Character(text) if text.trim().is_empty() => {
                self.mode_in_body(Token::Character(text));
            }
            Token::Doctype(_) => {}
            other => {
                self.mode = InsertionMode::InBody;
                self.process_token(other);
            }
        }
    }
}

/// 片段解析的起始插入模式，由上下文元素决定。
///
/// 规范里这一步叫「重置插入模式」，看的是上下文元素而不是栈——片段解析时
/// 栈里只有一个新建的 `html`，按栈推不出任何东西。
fn context_insertion_mode(name: &str) -> InsertionMode {
    match name {
        "select" => InsertionMode::InSelect,
        "td" | "th" => InsertionMode::InCell,
        "tr" => InsertionMode::InRow,
        "tbody" | "thead" | "tfoot" => InsertionMode::InTableBody,
        "caption" => InsertionMode::InCaption,
        "colgroup" => InsertionMode::InColumnGroup,
        "table" => InsertionMode::InTable,
        "head" => InsertionMode::InHead,
        "html" => InsertionMode::BeforeHead,
        // `frameset` 还没有对应的插入模式，按 `body` 处理。
        _ => InsertionMode::InBody,
    }
}

/// 是否是规范里的特殊元素。
///
/// 作用域判断与收养机构算法都要用它划分边界。
fn is_special(name: &str) -> bool {
    matches!(
        name,
        "address"
            | "applet"
            | "area"
            | "article"
            | "aside"
            | "base"
            | "basefont"
            | "bgsound"
            | "blockquote"
            | "body"
            | "br"
            | "button"
            | "caption"
            | "center"
            | "col"
            | "colgroup"
            | "dd"
            | "details"
            | "dir"
            | "div"
            | "dl"
            | "dt"
            | "embed"
            | "fieldset"
            | "figcaption"
            | "figure"
            | "footer"
            | "form"
            | "frame"
            | "frameset"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "head"
            | "header"
            | "hgroup"
            | "hr"
            | "html"
            | "iframe"
            | "img"
            | "input"
            | "keygen"
            | "li"
            | "link"
            | "listing"
            | "main"
            | "marquee"
            | "menu"
            | "meta"
            | "nav"
            | "noembed"
            | "noframes"
            | "noscript"
            | "object"
            | "ol"
            | "p"
            | "param"
            | "plaintext"
            | "pre"
            | "script"
            | "search"
            | "section"
            | "select"
            | "source"
            | "style"
            | "summary"
            | "table"
            | "tbody"
            | "td"
            | "template"
            | "textarea"
            | "tfoot"
            | "th"
            | "thead"
            | "title"
            | "tr"
            | "track"
            | "ul"
            | "wbr"
            | "xmp"
    )
}

/// 是否是隐含结束标签能收掉的元素。
fn is_implied_end_tag(name: &str) -> bool {
    matches!(
        name,
        "dd" | "dt" | "li" | "optgroup" | "option" | "p" | "rb" | "rp" | "rt" | "rtc"
    )
}

/// 是否构成默认作用域的边界。
fn is_scope_terminator(name: &str) -> bool {
    matches!(
        name,
        "applet" | "caption" | "html" | "table" | "td" | "th" | "marquee" | "object" | "template"
    )
}

/// 是否是历史遗留的怪异模式公开标识符。
fn is_quirky_public_id(id: &str) -> bool {
    let lower = id.to_ascii_lowercase();
    lower.starts_with("-//w3o//dtd w3 html strict 3.0//en//")
        || lower.starts_with("-/w3c/dtd html 4.0 transitional/en")
        || lower.starts_with("html")
        || lower == "-//w3c//dtd html 4.01 frameset//"
        || lower == "-//w3c//dtd html 4.01 transitional//"
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 解析一段 HTML 得到文档树。
    fn parse(input: &str) -> Document {
        TreeBuilder::new(input).parse()
    }

    /// 把文档树导出成缩进形式，方便断言整体结构。
    ///
    /// 从文档根的子节点开始写，缩进零层，这样断言里的缩进与实际结构一致。
    fn dump(document: &Document) -> String {
        let mut out = String::new();
        for child in document.children(document.root()) {
            write_node(document, *child, 0, &mut out);
        }
        out.trim_end().to_string()
    }

    /// 递归写出一个节点。
    fn write_node(document: &Document, id: NodeId, depth: usize, out: &mut String) {
        let indent = "  ".repeat(depth);
        match &document.node(id).data {
            NodeData::Document => {}
            NodeData::Doctype(doctype) => {
                out.push_str(&format!("{indent}!doctype {}\n", doctype.name));
            }
            NodeData::Element(element) => {
                out.push_str(&format!("{indent}{}", element.name));
                if !element.attributes.is_empty() {
                    out.push(' ');
                    let attributes: Vec<String> = element
                        .attributes
                        .iter()
                        .map(|attribute| format!("{}={}", attribute.name, attribute.value))
                        .collect();
                    out.push_str(&attributes.join(" "));
                }
                out.push('\n');
            }
            NodeData::Text(text) => {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    out.push_str(&format!("{indent}\"{trimmed}\"\n"));
                } else {
                    return;
                }
            }
            NodeData::Comment(text) => {
                out.push_str(&format!("{indent}!--{text}--\n"));
            }
        }
        for child in document.children(id) {
            write_node(document, *child, depth + 1, out);
        }
    }

    /// 取文档里第一个指定标签的元素内容文本。
    fn text_of(document: &Document, tag: &str) -> String {
        let node = document
            .find_element(document.root(), |element| element.name == tag)
            .unwrap_or_else(|| panic!("文档里找不到 <{tag}>"));
        document.text_content(node)
    }

    #[test]
    fn minimal_document() {
        assert_eq!(
            dump(&parse(
                "<!DOCTYPE html><html><head></head><body></body></html>"
            )),
            "!doctype html\nhtml\n  head\n  body"
        );
    }

    #[test]
    fn implied_html_head_body() {
        assert_eq!(
            dump(&parse("<p>hi")),
            "html\n  head\n  body\n    p\n      \"hi\""
        );
    }

    #[test]
    fn doctype_is_recorded() {
        let document = parse("<!DOCTYPE html><html></html>");
        let doctype = document
            .children(document.root())
            .iter()
            .find(|id| matches!(document.node(**id).data, NodeData::Doctype(_)));
        assert!(doctype.is_some(), "DOCTYPE 应当出现在文档根下面");
    }

    #[test]
    fn standards_mode_with_html5_doctype() {
        let builder = TreeBuilder::new("<!DOCTYPE html><p>x");
        let mut builder = builder;
        builder.mode_initial(Token::Doctype(Doctype {
            name: "html".into(),
            public_id: None,
            system_id: None,
            force_quirks: false,
        }));
        assert!(!builder.quirks, "标准 DOCTYPE 不该触发怪异模式");
    }

    #[test]
    fn quirks_mode_without_doctype() {
        let mut builder = TreeBuilder::new("<p>x");
        builder.mode_initial(Token::Character("x".into()));
        assert!(builder.quirks, "缺少 DOCTYPE 应当触发怪异模式");
    }

    #[test]
    fn quirks_mode_with_html4_transitional() {
        let mut builder = TreeBuilder::new("");
        builder.mode_initial(Token::Doctype(Doctype {
            name: "html".into(),
            public_id: Some("-//W3C//DTD HTML 4.01 Transitional//EN".into()),
            system_id: None,
            force_quirks: false,
        }));
        assert!(builder.quirks, "HTML 4.01 Transitional 应当触发怪异模式");
    }

    #[test]
    fn title_text_is_collected() {
        let document = parse("<title>页面标题</title><p>正文");
        assert_eq!(text_of(&document, "title"), "页面标题");
    }

    #[test]
    fn script_and_style_content_is_raw() {
        let document =
            parse("<script>if (a < b) { c(); }</script><style>a > b { color: red }</style>");
        assert_eq!(text_of(&document, "script"), "if (a < b) { c(); }");
        assert_eq!(text_of(&document, "style"), "a > b { color: red }");
    }

    #[test]
    fn nested_elements() {
        assert_eq!(
            dump(&parse("<div><span><b>x</b></span></div>")),
            "html\n  head\n  body\n    div\n      span\n        b\n          \"x\""
        );
    }

    #[test]
    fn implicit_paragraph_close() {
        // 新的 p 会把上一个 p 收掉。
        assert_eq!(
            dump(&parse("<p>a<p>b")),
            "html\n  head\n  body\n    p\n      \"a\"\n    p\n      \"b\""
        );
    }

    #[test]
    fn block_element_closes_paragraph() {
        assert_eq!(
            dump(&parse("<p>a<div>b</div>")),
            "html\n  head\n  body\n    p\n      \"a\"\n    div\n      \"b\""
        );
    }

    #[test]
    fn list_items_close_each_other() {
        assert_eq!(
            dump(&parse("<ul><li>a<li>b</ul>")),
            "html\n  head\n  body\n    ul\n      li\n        \"a\"\n      li\n        \"b\""
        );
    }

    #[test]
    fn stray_end_tag_is_ignored() {
        // 相邻的两段文本会合成一个文本节点。
        assert_eq!(
            dump(&parse("<div>a</span>b</div>")),
            "html\n  head\n  body\n    div\n      \"ab\""
        );
    }

    #[test]
    fn formatting_elements_adoption_agency() {
        // `<b>1<p>2</b>3</p>` 里的 b 要重新包住 p 的内容。
        let document = parse("<b>1<p>2</b>3</p>");
        let body = document
            .find_element(document.root(), |element| element.name == "body")
            .expect("应当有 body");
        let b_count = document.descendants_of_kind(body, "b").len();
        assert_eq!(b_count, 2, "收养机构算法应当把 b 拆成两个");
    }

    #[test]
    fn formatting_element_stays_open_across_block() {
        // `b` 一直在栈上，所以 div 之后的内容仍然归它管。
        let document = parse("<b>a<div>b</div>c");
        let body = document
            .find_element(document.root(), |element| element.name == "body")
            .expect("应当有 body");
        let bold = document
            .find_element(body, |element| element.name == "b")
            .expect("应当有 b");
        // b 的直接子节点依次是文本、div、文本。
        let kinds: Vec<&str> = document
            .children(bold)
            .iter()
            .map(|id| document.node(*id).tag_name().unwrap_or("#text"))
            .collect();
        assert_eq!(kinds, vec!["#text", "div", "#text"]);
    }

    #[test]
    fn formatting_elements_reconstructed_after_close() {
        // `</div>` 把 b 一起弹出栈，但活动格式化元素表里还留着它，
        // 所以后面的 y 会被重新建出来的 b 包住。
        let document = parse("<div><b>x</div>y");
        let body = document
            .find_element(document.root(), |element| element.name == "body")
            .expect("应当有 body");
        let bolds = document.descendants_of_kind(body, "b");
        assert_eq!(bolds.len(), 2, "应当重建出一个新的 b");
        let last = *bolds.last().expect("至少有一个");
        assert_eq!(document.text_content(last), "y");
    }

    #[test]
    fn table_structure_basics() {
        assert_eq!(
            dump(&parse("<table><tr><td>x</td></tr></table>")),
            "html\n  head\n  body\n    table\n      tbody\n        tr\n          td\n            \"x\""
        );
    }

    #[test]
    fn table_implies_tbody() {
        let document = parse("<table><tr><td>1</td></tr></table>");
        let table = document
            .find_element(document.root(), |element| element.name == "table")
            .expect("应当有 table");
        assert_eq!(
            document
                .first_element_child(table)
                .map(|id| document.node(id).tag_name().unwrap_or("")),
            Some("tbody")
        );
    }

    #[test]
    fn table_gets_foster_parented_text() {
        // 表格里直接出现的文本要挪到表格前面。
        let document = parse("<div><table>stray<tr><td>cell</td></tr></table></div>");
        let div = document
            .find_element(document.root(), |element| element.name == "div")
            .expect("应当有 div");
        let order: Vec<&str> = document
            .children(div)
            .iter()
            .map(|id| document.node(*id).tag_name().unwrap_or("#text"))
            .collect();
        assert_eq!(order, vec!["#text", "table"], "文本应当在表格之前");
    }

    #[test]
    fn table_cells_split_correctly() {
        assert_eq!(
            dump(&parse("<table><tr><td>a<td>b</table>")),
            "html\n  head\n  body\n    table\n      tbody\n        tr\n          td\n            \"a\"\n          td\n            \"b\""
        );
    }

    #[test]
    fn caption_is_placed_inside_table() {
        assert_eq!(
            dump(&parse(
                "<table><caption>标题</caption><tr><td>x</td></tr></table>"
            )),
            "html\n  head\n  body\n    table\n      caption\n        \"标题\"\n      tbody\n        tr\n          td\n            \"x\""
        );
    }

    #[test]
    fn select_only_keeps_options() {
        let document =
            parse("<select><option>a</option><div>ignored</div><option>b</option></select>");
        let select = document
            .find_element(document.root(), |element| element.name == "select")
            .expect("应当有 select");
        assert_eq!(document.descendants_of_kind(select, "option").len(), 2);
        assert_eq!(document.descendants_of_kind(select, "div").len(), 0);
    }

    #[test]
    fn comments_are_preserved() {
        assert_eq!(
            dump(&parse("<!-- top --><p>a")),
            "!-- top --\nhtml\n  head\n  body\n    p\n      \"a\""
        );
    }

    #[test]
    fn attributes_survive_parsing() {
        let document = parse(r#"<a href="/x" class="one two">link</a>"#);
        let anchor = document
            .find_element(document.root(), |element| element.name == "a")
            .expect("应当有 a");
        let element = document.element(anchor).expect("是元素");
        assert_eq!(element.get_attribute("href"), Some("/x"));
        assert!(element.has_token("class", "two"));
    }

    #[test]
    fn unclosed_elements_do_not_panic() {
        let document = parse("<div><span><b>未闭合");
        assert!(document.descendants_of_kind(document.root(), "b").len() == 1);
    }

    #[test]
    fn end_tag_without_start_is_ignored() {
        assert_eq!(
            dump(&parse("</div><p>x")),
            "html\n  head\n  body\n    p\n      \"x\""
        );
    }

    #[test]
    fn head_elements_go_to_head() {
        let document =
            parse("<html><head><meta charset='utf-8'><title>T</title></head><body>b</body></html>");
        let head = document
            .find_element(document.root(), |element| element.name == "head")
            .expect("应当有 head");
        assert_eq!(document.descendants_of_kind(head, "meta").len(), 1);
        assert_eq!(document.descendants_of_kind(head, "title").len(), 1);
    }

    #[test]
    fn head_elements_after_head_are_hoisted_into_head() {
        // head 已经结束、body 还没开始时出现的 title，按规范要放回 head。
        let document = parse("<head></head><title>晚到的标题</title><p>x");
        let head = document
            .find_element(document.root(), |element| element.name == "head")
            .expect("应当有 head");
        assert_eq!(document.descendants_of_kind(head, "title").len(), 1);
        assert_eq!(text_of(&document, "title"), "晚到的标题");
    }

    #[test]
    fn title_inside_body_stays_in_body() {
        // 已经进入 body 之后再出现 title，它就该待在 body 里。
        let document = parse("<body><p>x</p><title>留在原地</title>");
        let body = document
            .find_element(document.root(), |element| element.name == "body")
            .expect("应当有 body");
        assert_eq!(document.descendants_of_kind(body, "title").len(), 1);
    }

    #[test]
    fn br_is_void() {
        assert_eq!(
            dump(&parse("<p>a<br>b")),
            "html\n  head\n  body\n    p\n      \"a\"\n      br\n      \"b\""
        );
    }

    #[test]
    fn hr_closes_paragraph() {
        assert_eq!(
            dump(&parse("<p>a<hr>")),
            "html\n  head\n  body\n    p\n      \"a\"\n    hr"
        );
    }

    #[test]
    fn deeply_nested_markup() {
        let document = parse("<div><div><div><div><div>x</div></div></div></div></div>");
        assert_eq!(
            document.descendants_of_kind(document.root(), "div").len(),
            5
        );
    }

    #[test]
    fn empty_table() {
        assert_eq!(
            dump(&parse("<table></table>")),
            "html\n  head\n  body\n    table"
        );
    }

    #[test]
    fn nested_tables() {
        let document =
            parse("<table><tr><td><table><tr><td>inner</td></tr></table></td></tr></table>");
        assert_eq!(
            document.descendants_of_kind(document.root(), "table").len(),
            2
        );
        assert_eq!(text_of(&document, "td"), "inner");
    }

    #[test]
    fn form_and_input() {
        let document = parse("<form><input name=a><button>go</button></form>");
        assert_eq!(
            document.descendants_of_kind(document.root(), "input").len(),
            1
        );
        assert_eq!(
            document
                .descendants_of_kind(document.root(), "button")
                .len(),
            1
        );
    }

    #[test]
    fn whitespace_only_text_does_not_create_body_early() {
        let document = parse("  \n  <p>x");
        assert_eq!(text_of(&document, "p"), "x");
    }
}
