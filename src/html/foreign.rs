//! 外来内容（SVG 与 MathML）的名字与属性调整表。
//!
//! HTML 解析器在建外来元素时要做两件在 HTML 里不存在的事：把标签名恢复成
//! SVG 规定的大小写写法（源码里写 `foreignobject`，元素名得是 `foreignObject`），
//! 以及把属性名恢复成驼峰写法（`viewbox` 变成 `viewBox`）。
//!
//! 表照抄自 HTML 规范里「adjust SVG tag names」「adjust SVG attributes」
//! 「adjust MathML attributes」三节，没有增删。这些名字没法用规则推出来——
//! `feblend` 变 `feBlend` 而 `filter` 不变，只能逐个列。

use super::tokenizer::Attribute;

/// 把 SVG 元素名恢复成规定的大小写写法。
///
/// 表里没有的原样返回：SVG 里绝大多数元素名本来就是全小写。
pub fn adjust_svg_tag_name(name: &str) -> &str {
    match name {
        "altglyph" => "altGlyph",
        "altglyphdef" => "altGlyphDef",
        "altglyphitem" => "altGlyphItem",
        "animatecolor" => "animateColor",
        "animatemotion" => "animateMotion",
        "animatetransform" => "animateTransform",
        "clippath" => "clipPath",
        "feblend" => "feBlend",
        "fecolormatrix" => "feColorMatrix",
        "fecomponenttransfer" => "feComponentTransfer",
        "fecomposite" => "feComposite",
        "feconvolvematrix" => "feConvolveMatrix",
        "fediffuselighting" => "feDiffuseLighting",
        "fedisplacementmap" => "feDisplacementMap",
        "fedistantlight" => "feDistantLight",
        "fedropshadow" => "feDropShadow",
        "feflood" => "feFlood",
        "fefunca" => "feFuncA",
        "fefuncb" => "feFuncB",
        "fefuncg" => "feFuncG",
        "fefuncr" => "feFuncR",
        "fegaussianblur" => "feGaussianBlur",
        "feimage" => "feImage",
        "femerge" => "feMerge",
        "femergenode" => "feMergeNode",
        "femorphology" => "feMorphology",
        "feoffset" => "feOffset",
        "fepointlight" => "fePointLight",
        "fespecularlighting" => "feSpecularLighting",
        "fespotlight" => "feSpotLight",
        "fetile" => "feTile",
        "feturbulence" => "feTurbulence",
        "foreignobject" => "foreignObject",
        "glyphref" => "glyphRef",
        "lineargradient" => "linearGradient",
        "radialgradient" => "radialGradient",
        "textpath" => "textPath",
        other => other,
    }
}

/// 把 SVG 属性名恢复成规定的驼峰写法，就地改。
pub fn adjust_svg_attributes(attributes: &mut [Attribute]) {
    for attribute in attributes {
        attribute.name = match attribute.name.as_str() {
            "attributename" => "attributeName",
            "attributetype" => "attributeType",
            "basefrequency" => "baseFrequency",
            "baseprofile" => "baseProfile",
            "calcmode" => "calcMode",
            "clippathunits" => "clipPathUnits",
            "diffuseconstant" => "diffuseConstant",
            "edgemode" => "edgeMode",
            "filterunits" => "filterUnits",
            "glyphref" => "glyphRef",
            "gradienttransform" => "gradientTransform",
            "gradientunits" => "gradientUnits",
            "kernelmatrix" => "kernelMatrix",
            "kernelunitlength" => "kernelUnitLength",
            "keypoints" => "keyPoints",
            "keysplines" => "keySplines",
            "keytimes" => "keyTimes",
            "lengthadjust" => "lengthAdjust",
            "limitingconeangle" => "limitingConeAngle",
            "markerheight" => "markerHeight",
            "markerunits" => "markerUnits",
            "markerwidth" => "markerWidth",
            "maskcontentunits" => "maskContentUnits",
            "maskunits" => "maskUnits",
            "numoctaves" => "numOctaves",
            "pathlength" => "pathLength",
            "patterncontentunits" => "patternContentUnits",
            "patterntransform" => "patternTransform",
            "patternunits" => "patternUnits",
            "pointsatx" => "pointsAtX",
            "pointsaty" => "pointsAtY",
            "pointsatz" => "pointsAtZ",
            "preservealpha" => "preserveAlpha",
            "preserveaspectratio" => "preserveAspectRatio",
            "primitiveunits" => "primitiveUnits",
            "refx" => "refX",
            "refy" => "refY",
            "repeatcount" => "repeatCount",
            "repeatdur" => "repeatDur",
            "requiredextensions" => "requiredExtensions",
            "requiredfeatures" => "requiredFeatures",
            "specularconstant" => "specularConstant",
            "specularexponent" => "specularExponent",
            "spreadmethod" => "spreadMethod",
            "startoffset" => "startOffset",
            "stddeviation" => "stdDeviation",
            "stitchtiles" => "stitchTiles",
            "surfacescale" => "surfaceScale",
            "systemlanguage" => "systemLanguage",
            "tablevalues" => "tableValues",
            "targetx" => "targetX",
            "targety" => "targetY",
            "textlength" => "textLength",
            "viewbox" => "viewBox",
            "viewtarget" => "viewTarget",
            "xchannelselector" => "xChannelSelector",
            "ychannelselector" => "yChannelSelector",
            "zoomandpan" => "zoomAndPan",
            other => other,
        }
        .to_string();
    }
}

/// 把 MathML 属性名恢复成规定的写法，就地改。
///
/// 只有一条：`definitionurl` 要写成 `definitionURL`。
pub fn adjust_mathml_attributes(attributes: &mut [Attribute]) {
    for attribute in attributes {
        if attribute.name == "definitionurl" {
            attribute.name = "definitionURL".to_string();
        }
    }
}

/// 这个起始标签会不会把外来子树打断。
///
/// 表里那些标签一出现，就说明外层写的是 HTML 而不是外来内容，要把整棵外来
/// 子树弹掉，然后按 HTML 的规则重新处理这个标签。表照抄规范。
///
/// `font` 单独判：只有带 `color`、`face`、`size` 之一时才打断，不带属性的
/// `font` 在外来内容里就是个普通的 SVG 元素。
pub fn breaks_out(name: &str, attributes: &[Attribute]) -> bool {
    match name {
        "b" | "big" | "blockquote" | "body" | "br" | "center" | "code" | "dd" | "div" | "dl"
        | "dt" | "em" | "embed" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "head" | "hr"
        | "i" | "img" | "li" | "listing" | "menu" | "meta" | "nobr" | "ol" | "p" | "pre"
        | "ruby" | "s" | "small" | "span" | "strong" | "strike" | "sub" | "sup" | "table"
        | "tt" | "u" | "ul" | "var" => true,
        "font" => attributes.iter().any(|attribute| {
            matches!(attribute.name.as_str(), "color" | "face" | "size")
        }),
        _ => false,
    }
}

/// 这个结束标签会不会把外来子树打断。
///
/// 只有 `br` 与 `p` 两个，规范里就是这么列的。
pub fn breaks_out_end_tag(name: &str) -> bool {
    matches!(name, "br" | "p")
}
