# html5lib-tests 的树构建用例

这份数据来自 <https://github.com/html5lib/html5lib-tests>，取的是提交
`9329e64694e7835d0dcff9811e22856ef6ad16f9`（2026-06-22）的快照。

那次之后树构建用例搬去了 web-platform-tests，原仓库只留词法用例，所以这里
钉的是一个具体提交而不是 master。用例本身是 MIT 许可，见同目录的 LICENSE。

用例格式见 `tree-construction/README.md`。运行方式是
`cargo test -p ysu --test html5lib_tree`。
