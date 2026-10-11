// Nowebiew M1 - HTML 子集解析
// 支持：常见容器/文本标签、id/class 属性、<title>、注释、实体、文本空白折叠。
// 不支持（M1 明确不做）：脚本内嵌、任意属性、嵌套表格等。

#[derive(Clone)]
pub struct El {
    pub tag: String,
    pub id: Option<String>,
    pub classes: Vec<String>,
    pub attrs: Vec<(String, String)>,   // 其余属性（含 data-*，供属性选择器匹配）
    pub style_attr: Option<String>,     // style="..." 内联样式（级联优先级最高）
    pub children: Vec<Child>,
}

#[derive(Clone)]
pub enum Child {
    El(El),
    Text(String),
}

/// CSS 来源（按文档顺序参与级联）：link 外部表 / <style> 内联块
#[derive(Clone)]
pub enum CssSource {
    Link(String),
    Inline(String),
}

/// TS 来源（按文档顺序拼接，同一编译管线）：src= 外部文件 / 内联脚本内容
#[derive(Clone)]
pub enum ScriptCode {
    File(String),
    Inline(String),
}

pub struct Doc {
    pub title: String,
    pub body: El, // <body> 元素本身（tag = "body"）
    pub html_attrs: Vec<(String, String)>, // <html> 标签属性（级联链的根，不渲染）
    pub styles: Vec<CssSource>,   // 级联来源，按文档顺序
    pub scripts: Vec<ScriptCode>, // 脚本来源，按文档顺序
    pub noscript_styles: Vec<String>, // <noscript> 内 <style> 原始 CSS（页脚本全部未执行时生效）
}

pub fn parse_html(src: &str) -> Result<Doc, String> {
    let b = src.as_bytes();
    let mut i = 0usize;
    let mut stack: Vec<El> = Vec::new();
    let mut root_children: Vec<Child> = Vec::new();
    let mut title = String::new();
    let mut in_title = false;
    let mut styles: Vec<CssSource> = Vec::new();
    let mut scripts: Vec<ScriptCode> = Vec::new();
    let mut noscript_styles: Vec<String> = Vec::new();
    let mut html_attrs: Vec<(String, String)> = Vec::new();
    let mut body_info: Option<(Option<String>, Vec<String>, Vec<(String, String)>, Option<String>)> = None;

    macro_rules! push_text {
        ($t:expr) => {{
            let t = collapse_ws(&$t);
            if !t.is_empty() {
                if let Some(top) = stack.last_mut() {
                    top.children.push(Child::Text(t));
                } else {
                    root_children.push(Child::Text(t));
                }
            }
        }};
    }

    while i < b.len() {
        if b[i] == b'<' {
            // 注释
            if src[i..].starts_with("<!--") {
                match src[i..].find("-->") {
                    Some(p) => { i += p + 3; continue; }
                    None => return Err("HTML 注释未闭合".into()),
                }
            }
            // doctype / 处理指令
            if src[i..].starts_with("<!") {
                match src[i..].find('>') {
                    Some(p) => { i += p + 1; continue; }
                    None => return Err("声明未闭合".into()),
                }
            }
            if src[i..].starts_with("</") {
                // 结束标签：html/body 不建栈，其闭合标签直接跳过；
                // 其余按名字找栈内最近同名元素，弹到它为止（浏览器隐式闭合语义）；找不到则忽略
                let end = src[i..].find('>').ok_or("结束标签缺 '>'")?;
                let name = src[i + 2..i + end].trim().to_ascii_lowercase();
                if name == "html" || name == "body" {
                    i += end + 1;
                    continue;
                }
                if let Some(pos) = stack.iter().rposition(|e| e.tag == name) {
                    while stack.len() > pos + 1 {
                        let orphan = stack.pop().unwrap();
                        attach_el(&mut stack, &mut root_children, orphan);
                    }
                    let el = stack.pop().unwrap();
                    if el.tag == "title" {
                        title = collect_text(&el);
                        in_title = false;
                    }
                    attach_el(&mut stack, &mut root_children, el);
                }
                i += end + 1;
                continue;
            }
            // 开始标签
            let end = find_tag_end(src, i).ok_or("标签未闭合")?;
            let inner = &src[i + 1..end]; // 不含 < >
            i = end + 1;
            let self_closing = inner.ends_with('/');
            let inner = inner.trim_end_matches('/').trim();
            // 属性解析
            let mut chars = inner.char_indices().peekable();
            let mut tag = String::new();
            for (_, c) in chars.by_ref() {
                if c.is_whitespace() { break; }
                tag.push(c.to_ascii_lowercase());
            }
            let rest = &inner[tag.len()..];
            let (id, classes, style_attr, attrs) = parse_attrs(rest)?;
            let tag_l = tag.to_ascii_lowercase();

            if tag_l == "title" { in_title = true; }
            if tag_l == "html" {
                html_attrs = attrs.clone();
                continue; /* html 不建 DOM（属性进级联根） */
            }
            if tag_l == "body" {
                /* 显式 <body>：属性记录到合成 body，内容直接进根层 */
                body_info = Some((id, classes, attrs, style_attr));
                continue;
            }
            if tag_l == "style" {
                /* RAWTEXT：内容整体作为 CSS 内联块（head/body 任意位置均参与级联），不建 DOM 节点 */
                match src[i..].find("</style>") {
                    Some(p) => {
                        styles.push(CssSource::Inline(src[i..i + p].to_string()));
                        i += p + "</style>".len();
                        continue;
                    }
                    None => return Err("<style> 未闭合".to_string()),
                }
            }
            if tag_l == "link" {
                let rel = attrs.iter()
                    .find(|(k, _)| k == "rel")
                    .map(|(_, v)| v.to_ascii_lowercase())
                    .unwrap_or_default();
                if rel == "stylesheet" {
                    if let Some((_, href)) = attrs.iter().find(|(k, _)| k == "href") {
                        styles.push(CssSource::Link(href.clone()));
                    }
                }
            }
            if tag_l == "noscript" {
                /* RAWTEXT 直取闭合。浏览器语义：脚本不可用/未执行时 noscript 内容生效；
                 * 这里只提取其中的 <style> 块（生效时机由 main 按页脚本是否执行裁决），
                 * 其余子内容（提示文本/meta refresh）暂不渲染。 */
                match src[i..].find("</noscript>") {
                    Some(p) => {
                        let inner = &src[i..i + p];
                        let mut j = 0usize;
                        while let Some(sp) = inner[j..].find("<style") {
                            let s0 = j + sp;
                            let Some(gt) = inner[s0..].find('>') else { break };
                            let c0 = s0 + gt + 1;
                            let Some(ep) = inner[c0..].find("</style>") else { break };
                            noscript_styles.push(inner[c0..c0 + ep].to_string());
                            j = c0 + ep + "</style>".len();
                        }
                        i += p + "</noscript>".len();
                        continue;
                    }
                    None => return Err("<noscript> 未闭合".to_string()),
                }
            }
            if tag_l == "script" {
                /* 脚本：<script src="..."> 外部 TS 文件；无 src 的内联内容整体作为 TS
                 * （J0：与外部文件同一条编译管线，文档顺序保留）。RAWTEXT 直取闭合标签。 */
                let src_attr = attrs.iter().find(|(k, _)| k == "src").map(|(_, v)| v.clone());
                match src[i..].find("</script>") {
                    Some(p) => {
                        let body = src[i..i + p].trim().to_string();
                        match src_attr {
                            Some(s) => scripts.push(ScriptCode::File(s)),
                            None => if !body.is_empty() { scripts.push(ScriptCode::Inline(body)); },
                        }
                        i += p + "</script>".len();
                    }
                    None => return Err("<script> 未闭合".to_string()),
                }
                continue;
            }
            if VOID.contains(&tag_l.as_str()) {
                /* img 是替换型元素（浏览器进 DOM、按 src 渲染图片），其余 VOID 标签不建节点 */
                if tag_l == "img" {
                    attach_el(&mut stack, &mut root_children, El {
                        tag: tag_l, id, classes, attrs, style_attr, children: Vec::new(),
                    });
                }
                continue;
            }
            if self_closing {
                continue;
            }
            stack.push(El { tag: tag_l, id, classes, attrs, style_attr, children: Vec::new() });
            let _ = in_title;
        } else {
            // 文本
            let start = i;
            while i < b.len() && b[i] != b'<' { i += 1; }
            push_text!(decode_entities(&src[start..i]));
        }
    }

    if !stack.is_empty() {
        return Err(format!("标签 <{}> 未闭合", stack.last().unwrap().tag));
    }

    // 合成 body（显式 <body> 的属性已记录在 body_info，内容本就直接进根层）
    let (bid, bclasses, battrs, bstyle) = body_info
        .unwrap_or((None, Vec::new(), Vec::new(), None));
    let body = El {
        tag: "body".into(), id: bid, classes: bclasses, attrs: battrs, style_attr: bstyle,
        children: root_children,
    };
    Ok(Doc { title, body, html_attrs, styles, scripts, noscript_styles })
}

const VOID: &[&str] = &["br", "hr", "img", "input", "meta", "link", "area", "base", "col", "embed", "source", "track", "wbr"];

fn find_tag_end(src: &str, from: usize) -> Option<usize> {
    let b = src.as_bytes();
    let mut i = from;
    let mut quote: Option<u8> = None;
    while i < b.len() {
        match quote {
            Some(q) => { if b[i] == q { quote = None; } }
            None => {
                if b[i] == b'"' || b[i] == b'\'' { quote = Some(b[i]); }
                else if b[i] == b'>' { return Some(i); }
            }
        }
        i += 1;
    }
    None
}

fn parse_attrs(rest: &str) -> Result<(Option<String>, Vec<String>, Option<String>, Vec<(String, String)>), String> {
    let mut id = None;
    let mut classes = Vec::new();
    let mut style_attr = None;
    let mut attrs: Vec<(String, String)> = Vec::new();
    let b = rest.as_bytes();
    let mut i = 0usize;
    while i < b.len() {
        while i < b.len() && b[i].is_ascii_whitespace() { i += 1; }
        if i >= b.len() { break; }
        // 属性名
        let start = i;
        while i < b.len() && b[i] != b'=' && !b[i].is_ascii_whitespace() { i += 1; }
        let name = rest[start..i].to_ascii_lowercase();
        // 跳过空白
        while i < b.len() && b[i].is_ascii_whitespace() { i += 1; }
        let value;
        if i < b.len() && b[i] == b'=' {
            i += 1;
            while i < b.len() && b[i].is_ascii_whitespace() { i += 1; }
            if i < b.len() && (b[i] == b'"' || b[i] == b'\'') {
                let q = b[i]; i += 1;
                let vs = i;
                while i < b.len() && b[i] != q { i += 1; }
                value = rest[vs..i].to_string();
                i += 1;
            } else {
                let vs = i;
                while i < b.len() && !b[i].is_ascii_whitespace() { i += 1; }
                value = rest[vs..i].to_string();
            }
        } else {
            value = String::new();
        }
        match name.as_str() {
            "id" => id = Some(value),
            "class" => classes = value.split_whitespace().map(|s| s.to_string()).collect(),
            "style" => style_attr = Some(value),
            _ => attrs.push((name, value)),
        }
    }
    Ok((id, classes, style_attr, attrs))
}

fn decode_entities(s: &str) -> String {
    if !s.contains('&') { return s.to_string(); }
    let mut o = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(p) = rest.find('&') {
        o.push_str(&rest[..p]);
        let tail = &rest[p..];
        let semi = match tail.find(';') { Some(x) if x <= 8 => x, _ => { o.push('&'); rest = &tail[1..]; continue; } };
        let ent = &tail[1..semi];
        match ent {
            "amp" => o.push('&'),
            "lt" => o.push('<'),
            "gt" => o.push('>'),
            "quot" => o.push('"'),
            "#39" | "apos" => o.push('\''),
            "nbsp" => o.push(' '),
            e if e.starts_with('#') => {
                let cp = if let Some(hex) = e[1..].strip_prefix('x').or_else(|| e[1..].strip_prefix('X')) {
                    u32::from_str_radix(hex, 16).ok()
                } else { e[1..].parse::<u32>().ok() };
                match cp.and_then(char::from_u32) { Some(c) => o.push(c), None => o.push_str(&tail[..semi + 1]) }
            }
            _ => o.push_str(&tail[..semi + 1]),
        }
        rest = &tail[semi + 1..];
    }
    o.push_str(rest);
    o
}

fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn collect_text(el: &El) -> String {
    let mut o = String::new();
    for c in &el.children {
        match c {
            Child::Text(t) => { if !o.is_empty() { o.push(' '); } o.push_str(t); }
            Child::El(_) => {}
        }
    }
    o
}

/// 把完成解析的元素挂到栈顶（栈空则进根层）
fn attach_el(stack: &mut Vec<El>, root_children: &mut Vec<Child>, el: El) {
    let child = Child::El(el);
    if let Some(top) = stack.last_mut() {
        top.children.push(child);
    } else {
        root_children.push(child);
    }
}
