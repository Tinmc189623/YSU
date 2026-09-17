#!/usr/bin/env python3
"""从 HTML 规范生成字符引用表。

数据来源是 WHATWG 的 https://html.spec.whatwg.org/entities.json，
它列出全部具名字符引用，包括带分号与不带分号两种形式。

用法：
    curl -sS -o /tmp/entities.json https://html.spec.whatwg.org/entities.json
    python3 crates/ysu/tools/generate_entities.py /tmp/entities.json \
        crates/ysu/src/html/entities.rs
"""

import json
import sys
import unicodedata

HEADER = '''//! HTML 具名字符引用表。
//!
//! 本文件由 `tools/generate_entities.py` 从 WHATWG 的 `entities.json`
//! 生成，不要手工编辑。表按名字的字节序排列，查询走二分查找。
//!
//! 数据来源：<https://html.spec.whatwg.org/entities.json>

/// 具名字符引用，键是不含 `&` 的名字，值是对应的字符。
///
/// 名字里带分号的是常规形式，不带分号的是历史遗留形式，只在特定上下文
/// 里允许省略分号，两种形式都列在表里由调用方决定如何使用。
pub static NAMED_REFERENCES: &[(&str, &str)] = &[
'''


def is_invisible(ch: str) -> bool:
    """判断字符是否该写成转义形式。

    零宽字符、格式控制符与各类不占宽度的空格直接写在源码里既看不见也容易
    被编辑器吃掉，clippy 还会直接报错，所以一律写成 `\\u{...}`。
    """
    category = unicodedata.category(ch)
    if category.startswith('C'):
        # Cc 控制符、Cf 格式符、Cn 未分配、Co 私用、Cs 代理。
        return True
    if category in ('Zl', 'Zp'):
        return True
    # 不换行空格是常见写法，保留原样，其余不占宽度的空格都转义。
    return category == 'Zs' and ch not in (' ', ' ')


def rust_string(text: str) -> str:
    """把 Python 字符串转成 Rust 字符串字面量，转义反斜杠、引号与控制字符。"""
    out = []
    for ch in text:
        code = ord(ch)
        if ch == '\\':
            out.append('\\\\')
        elif ch == '"':
            out.append('\\"')
        elif ch == '\n':
            out.append('\\n')
        elif ch == '\r':
            out.append('\\r')
        elif ch == '\t':
            out.append('\\t')
        elif code < 0x20 or code == 0x7f or is_invisible(ch):
            out.append('\\u{%x}' % code)
        else:
            out.append(ch)
    return '"' + ''.join(out) + '"'


def main() -> int:
    if len(sys.argv) != 3:
        print(__doc__)
        return 2

    source, target = sys.argv[1], sys.argv[2]
    with open(source, encoding='utf-8') as handle:
        data = json.load(handle)

    entries = []
    for key, value in data.items():
        if not key.startswith('&'):
            raise ValueError(f'键 {key!r} 不是以 & 开头')
        entries.append((key[1:], value['characters']))

    # 按名字的字节序排序，与 Rust 字符串的比较规则一致，二分查找才成立。
    entries.sort(key=lambda item: item[0].encode('utf-8'))

    with open(target, 'w', encoding='utf-8') as handle:
        handle.write(HEADER)
        for name, characters in entries:
            handle.write(f'    ({rust_string(name)}, {rust_string(characters)}),\n')
        handle.write('];\n')

    print(f'写入 {target}，共 {len(entries)} 条')
    return 0


if __name__ == '__main__':
    sys.exit(main())
