// Nowebiew M1 - CSS 子集解析
// 选择器：#id / .class / tag（单简单选择器，逗号列表）。
// 值：px / % / em / 颜色（#hex、rgb/rgba、常见命名色）/ 关键字。

/// 复合选择器（同一元素上的 tag/id/class/属性组合），如 h1#title.card[data-x="y"]
/// hover: 带 :hover 伪类（运行时求值——鼠标悬停状态）
#[derive(Debug, Clone)]
pub struct Compound {
    pub tag: Option<String>,
    pub id: Option<String>,
    pub classes: Vec<String>,
    pub attrs: Vec<(String, Option<String>)>, // [attr] 或 [attr="val"]
    pub universal: bool,                      // * 通配符
    pub hover: bool,                          // :hover
}

/// 选择器：复合部分按文档树从左（最外祖先）到右（主体元素）排列，空格 = 后代组合器
#[derive(Debug, Clone)]
pub struct Sel {
    pub parts: Vec<Compound>,
}

/// 媒体查询条件（编译期按目标视口求值；全部条件 AND 语义）
/// orient: 0=无方向条件 1=landscape(w>h) 2=portrait(h>=w)
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Media {
    pub minw: Option<f32>,
    pub maxw: Option<f32>,
    pub minh: Option<f32>,
    pub maxh: Option<f32>,
    pub orient: u8,
}

#[derive(Debug, Clone)]
pub struct Rule {
    pub sels: Vec<Sel>,
    pub decls: Vec<(String, String)>,
    pub media: Option<Media>,
    pub ua: bool, // 浏览器 UA 默认样式（作者规则在级联中恒胜出，与特异性无关）
}

pub fn parse_css(src: &str) -> Result<(Vec<Rule>, Vec<String>), String> {
    parse_css_mq(src, None)
}

/// 解析媒体查询条件；返回 None = 屏幕·当前视口下永不匹配（如 @media print）
fn parse_media_query(q: &str) -> Option<Media> {
    let ql = q.to_ascii_lowercase();
    let mut m = Media { minw: None, maxw: None, minh: None, maxh: None, orient: 0 };
    for t in ql.split(" and ") {
        let t = t.trim();
        if t.is_empty() { continue; }
        match t {
            "screen" | "all" => continue,
            "print" => return None, /* 打印样式：屏幕永不应用 */
            _ => {}
        }
        let inner = t.trim_start_matches('(').trim_end_matches(')');
        let (k, v) = inner.split_once(':')?;
        let (k, v) = (k.trim(), v.trim());
        if k == "orientation" {
            /* 方向条件：宽高比较，非 px 值 */
            m.orient = match v {
                "landscape" => 1,
                "portrait" => 2,
                _ => return None,
            };
            continue;
        }
        let px: f32 = v.trim_end_matches("px").trim().parse().ok()?;
        match k.trim() {
            "min-width" => m.minw = Some(px),
            "max-width" => m.maxw = Some(px),
            "min-height" => m.minh = Some(px),
            "max-height" => m.maxh = Some(px),
            _ => return None,
        }
    }
    Some(m)
}

fn parse_css_mq(src: &str, mq: Option<Media>) -> Result<(Vec<Rule>, Vec<String>), String> {
    let stripped = strip_comments(src);
    let mut rules = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    let b = stripped.as_bytes();
    let mut i = 0usize;
    while i < b.len() {
        while i < b.len() && b[i].is_ascii_whitespace() { i += 1; }
        if i >= b.len() { break; }
        // 选择器
        let sel_start = i;
        while i < b.len() && b[i] != b'{' {
            if b[i] == b'}' { return Err(format!("CSS: 意外的 '}}'（位置 {}）", i)); }
            i += 1;
        }
        if i >= b.len() { return Err("CSS: 缺少 '{'".into()); }
        let sels_str = &stripped[sel_start..i];

        /* @media：解析条件 + 递归解析块内规则（嵌套大括号按配对取块）；
           其余 @ 规则（@keyframes/@supports 等）整块跳过 */
        let trimmed = sels_str.trim();
        if trimmed.starts_with("@media") {
            let cond = trimmed["@media".len()..].trim();
            i += 1; // {
            let start = i;
            let mut depth = 1usize;
            while i < b.len() && depth > 0 {
                match b[i] {
                    b'{' => depth += 1,
                    b'}' => depth -= 1,
                    _ => {}
                }
                i += 1;
            }
            let inner = &stripped[start..i.saturating_sub(1)];
            match parse_media_query(cond) {
                Some(m) => {
                    if let Ok((sub, mut w2)) = parse_css_mq(inner, Some(m)) {
                        warnings.append(&mut w2);
                        rules.extend(sub); /* 递归已带 mq，文档序保持 */
                    }
                }
                None => {
                    warnings.push(format!("@media {} 不适用于屏幕，已跳过", cond));
                }
            }
            continue;
        }
        /* @ 规则（@keyframes/@supports 等）：块内嵌套大括号，按配对整块跳过 */
        if trimmed.starts_with('@') {
            i += 1; // {
            let mut depth = 1usize;
            while i < b.len() && depth > 0 {
                match b[i] {
                    b'{' => depth += 1,
                    b'}' => depth -= 1,
                    _ => {}
                }
                i += 1;
            }
            warnings.push(format!("@ 规则暂不支持，已跳过: {}", trimmed));
            continue;
        }

        i += 1; // {
        // 声明
        let decl_start = i;
        while i < b.len() && b[i] != b'}' { i += 1; }
        if i >= b.len() { return Err("CSS: 缺少 '}'".into()); }
        let decls_str = &stripped[decl_start..i];
        i += 1; // }

        /* 浏览器语义：不支持的选择器只丢弃该条规则并警告，不整体报错 */
        let sels = match parse_selectors(sels_str, warnings.len()) {
            Some(s) => s,
            None => {
                warnings.push(sels_str.trim().to_string());
                continue;
            }
        };
        let decls = parse_decls(decls_str)?;
        if !sels.is_empty() && !decls.is_empty() {
            rules.push(Rule { sels, decls, media: mq, ua: false });
        }
    }
    Ok((rules, warnings))
}

/// 媒体条件是否命中视口（None = 无条件，恒真）
pub fn media_matches(m: Option<&Media>, vw: f32, vh: f32) -> bool {
    match m {
        None => true,
        Some(m) => {
            if let Some(v) = m.minw { if vw < v { return false; } }
            if let Some(v) = m.maxw { if vw > v { return false; } }
            if let Some(v) = m.minh { if vh < v { return false; } }
            if let Some(v) = m.maxh { if vh > v { return false; } }
            if m.orient == 1 && vw <= vh { return false; } /* landscape: w > h */
            if m.orient == 2 && vh < vw { return false; }  /* portrait: h >= w */
            true
        }
    }
}

/// 视口段：语义为 wlo < w <= whi 且 hlo < h <= hhi（首段下界用 -1.0，判 w > -1 恒真）
/// orient: 0=全域 1=仅 landscape(w>h) 2=仅 portrait(h>=w)——对角线穿越的矩形段拆成两半
#[derive(Debug, Clone, Copy)]
pub struct MediaSeg {
    pub wlo: f32, pub whi: f32,
    pub hlo: f32, pub hhi: f32,
    pub orient: u8,
}

impl MediaSeg {
    pub fn contains(&self, w: f32, h: f32) -> bool {
        if !(w > self.wlo && w <= self.whi && h > self.hlo && h <= self.hhi) { return false; }
        match self.orient {
            1 => w > h,
            2 => h >= w,
            _ => true,
        }
    }
    /// 段内代表视口（编译期求值用；运行时重建后按真实尺寸重算）。
    /// 无界方向用默认视口；上有界取段内中点偏下；下有界上无界取 lo+1000。
    /// 带 orient 约束时微调代表点使其落在段对应的对角半平面内。
    pub fn representative(&self, def_w: f32, def_h: f32) -> (f32, f32) {
        let axis = |lo: f32, hi: f32, def: f32| -> f32 {
            let un_lo = lo < 0.0;
            let un_hi = hi >= 1.0e9;
            match (un_lo, un_hi) {
                (true, true) => def,
                (true, false) => hi.min(def),
                (false, true) => lo + 1000.0,
                (false, false) => (lo + hi) * 0.5,
            }
        };
        let mut w = axis(self.wlo, self.whi, def_w);
        let mut h = axis(self.hlo, self.hhi, def_h);
        match self.orient {
            1 => { /* landscape: 保证 w > h */
                if w <= h {
                    w = if self.whi >= 1.0e9 { h + 1000.0 } else { self.whi };
                }
                if w <= h && self.hlo >= 0.0 {
                    h = self.hlo.min(w - 1.0).max(0.0);
                }
            }
            2 => { /* portrait: 保证 h >= w */
                if h < w {
                    h = if self.hhi >= 1.0e9 { w + 1000.0 } else { self.hhi };
                }
                if h < w && self.wlo >= 0.0 {
                    w = self.wlo.max(h);
                }
            }
            _ => {}
        }
        (w, h)
    }
}

/// 从规则集提取媒体断点，把视口平面划分为「每条规则的 media 求值结果恒定」的段。
/// 无任何媒体规则的退化结果：单个全覆盖段。
/// orientation 条件沿 w=h 对角线切分平面：跨越对角线的矩形段拆成 landscape/portrait
/// 两个半段（点集不相交，并集不变），保证段内所有规则的求值结果恒定。
pub fn media_segments(rules: &[Rule]) -> Vec<MediaSeg> {
    let mut wt: Vec<f32> = Vec::new();
    let mut ht: Vec<f32> = Vec::new();
    let mut has_orient = false;
    for r in rules {
        if let Some(m) = &r.media {
            if m.orient != 0 { has_orient = true; }
            if let Some(v) = m.minw { wt.push(v); }
            if let Some(v) = m.maxw { wt.push(v); }
            if let Some(v) = m.minh { ht.push(v); }
            if let Some(v) = m.maxh { ht.push(v); }
        }
    }
    wt.sort_by(|a, b| a.partial_cmp(b).unwrap());
    wt.dedup();
    ht.sort_by(|a, b| a.partial_cmp(b).unwrap());
    ht.dedup();
    /* 断点两侧各一段：(-inf, t0], (t0, t1], ..., (tn, +inf) */
    let mut wb: Vec<(f32, f32)> = Vec::new();
    let mut lo = -1.0f32;
    for &t in &wt { wb.push((lo, t)); lo = t; }
    wb.push((lo, 1.0e9));
    let mut hb: Vec<(f32, f32)> = Vec::new();
    let mut lo = -1.0f32;
    for &t in &ht { hb.push((lo, t)); lo = t; }
    hb.push((lo, 1.0e9));
    let mut out = Vec::new();
    for &(wlo, whi) in &wb {
        for &(hlo, hhi) in &hb {
            /* 矩形整体在 landscape 半平面（最小 w ≥ 最大 h）或 portrait 半平面
               （最小 h ≥ 最大 w）时不拆，保持段数精简 */
            let all_land = wlo >= hhi;
            let all_port = hlo >= whi;
            if has_orient && !all_land && !all_port {
                out.push(MediaSeg { wlo, whi, hlo, hhi, orient: 1 });
                out.push(MediaSeg { wlo, whi, hlo, hhi, orient: 2 });
            } else {
                out.push(MediaSeg { wlo, whi, hlo, hhi, orient: 0 });
            }
        }
    }
    out
}


pub fn strip_comments(src: &str) -> String {
    if !src.contains("/*") { return src.to_string(); }
    let mut o = String::with_capacity(src.len());
    let mut rest = src;
    while let Some(p) = rest.find("/*") {
        o.push_str(&rest[..p]);
        match rest[p..].find("*/") {
            Some(e) => rest = &rest[p + e + 2..],
            None => break,
        }
    }
    o.push_str(rest);
    o
}

/// 返回 None = 该规则含不支持的选择器（伪类/属性/通配符等），整条丢弃。
/// 支持逗号列表；每个选择器内空格 = 后代组合器（.card h3），复合 = tag#id.a.b
fn parse_selectors(s: &str, _wctx: usize) -> Option<Vec<Sel>> {
    let mut out = Vec::new();
    for part in s.split(',') {
        let p = part.trim();
        if p.is_empty() { continue; }
        let mut parts = Vec::new();
        for tok in p.split_whitespace() {
            match parse_compound(tok) {
                Some(c) => parts.push(c),
                None => return None,
            }
        }
        if parts.is_empty() { return None; }
        out.push(Sel { parts });
    }
    if out.is_empty() { return None; }
    Some(out)
}

/// 解析单个复合选择器 token：tag#id.a.b[attr="v"]、:root、*；
/// 其余伪类/操作符（:[>*+~ 等）不支持
fn parse_compound(tok: &str) -> Option<Compound> {
    let b = tok.as_bytes();
    let mut i = 0usize;
    let mut tag = None;
    let universal = false;
    if tok == ":root" {
        /* :root = 文档根元素（Nowebiew 级联根为 html 节点） */
        return Some(Compound { tag: Some("html".into()), id: None, classes: Vec::new(), attrs: Vec::new(), universal: false, hover: false });
    }
    if tok == "*" {
        return Some(Compound { tag: None, id: None, classes: Vec::new(), attrs: Vec::new(), universal: true, hover: false });
    }
    let s = i;
    while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'-' || b[i] == b'_') { i += 1; }
    if i > s {
        tag = Some(tok[s..i].to_ascii_lowercase());
    } else if i < b.len() && !(b[i] == b'.' || b[i] == b'#' || b[i] == b'[') {
        return None;
    }
    let mut id = None;
    let mut classes = Vec::new();
    let mut attrs: Vec<(String, Option<String>)> = Vec::new();
    let mut hover = false;
    while i < b.len() {
        match b[i] {
            b'#' => {
                i += 1;
                let st = i;
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'-' || b[i] == b'_') { i += 1; }
                if i == st { return None; }
                id = Some(tok[st..i].to_string());
            }
            b'.' => {
                i += 1;
                let st = i;
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'-' || b[i] == b'_') { i += 1; }
                if i == st { return None; }
                classes.push(tok[st..i].to_string());
            }
            b'[' => {
                i += 1;
                let ns = i;
                while i < b.len() && b[i] != b'=' && b[i] != b']' { i += 1; }
                if i >= b.len() { return None; }
                let name = tok[ns..i].trim().to_ascii_lowercase();
                if name.is_empty() { return None; }
                if b[i] == b']' {
                    i += 1;
                    attrs.push((name, None)); /* [attr] 存在性匹配 */
                } else {
                    i += 1; // =
                    /* 可选引号值 */
                    let val;
                    if i < b.len() && (b[i] == b'"' || b[i] == b'\'') {
                        let q = b[i];
                        i += 1;
                        let vs = i;
                        while i < b.len() && b[i] != q { i += 1; }
                        if i >= b.len() { return None; }
                        val = tok[vs..i].to_string();
                        i += 1;
                    } else {
                        let vs = i;
                        while i < b.len() && b[i] != b']' { i += 1; }
                        if i >= b.len() { return None; }
                        val = tok[vs..i].trim().to_string();
                    }
                    i += 1; // ]
                    /* 仅支持精确匹配 [a=v]；*= ^= 等子串操作符不支持 */
                    attrs.push((name, Some(val)));
                }
            }
            b':' => {
                /* 伪类：仅支持 :hover（运行时求值）；其余伪类规则整体不支持 */
                i += 1;
                let st = i;
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'-') { i += 1; }
                if i == st { return None; }
                match tok[st..i].to_ascii_lowercase().as_str() {
                    "hover" => hover = true,
                    _ => return None,
                }
            }
            _ => return None, /* > + ~ 组合器 / 其他 */
        }
    }
    if tag.is_none() && id.is_none() && classes.is_empty() && attrs.is_empty() && !universal && !hover { return None; }
    Some(Compound { tag, id, classes, attrs, universal, hover })
}

/// 选择器是否含运行时条件（类名 / :hover）——这类规则不能在编译期一次烘焙，
/// 需生成运行时守卫（matcher），classList 变更 / 悬停变化后重求值
pub fn sel_is_dynamic(sel: &Sel) -> bool {
    sel.parts.iter().any(|c| !c.classes.is_empty() || c.hover)
}

/// grid-template-columns 子集：repeat(N, 1fr) 或空格分隔的 N 个 fr → 列数 N
pub fn parse_grid_cols(s: &str) -> Option<usize> {
    let s = s.trim();
    if let Some(rest) = s.strip_prefix("repeat(") {
        let inner = rest.strip_suffix(')')?;
        let mut it = inner.split(',');
        let n: usize = it.next()?.trim().parse().ok()?;
        let unit = it.next()?.trim();
        if it.next().is_some() { return None; }
        return if unit.ends_with("fr") && n > 0 { Some(n) } else { None };
    }
    let parts: Vec<&str> = s.split_whitespace().collect();
    if !parts.is_empty() && parts.iter().all(|p| p.ends_with("fr")) {
        return Some(parts.len());
    }
    None
}

pub fn parse_decls(s: &str) -> Result<Vec<(String, String)>, String> {
    let mut out = Vec::new();
    for part in s.split(';') {
        let p = part.trim();
        if p.is_empty() { continue; }
        let colon = p.find(':').ok_or_else(|| format!("CSS 声明缺少 ':': '{}'", p))?;
        let name = p[..colon].trim().to_ascii_lowercase();
        let mut value = p[colon + 1..].trim().to_string();
        /* !important：子集暂不实现重要性语义，剥掉尾缀按普通声明处理 */
        if let Some(stripped) = value.strip_suffix("!important") {
            value = stripped.trim_end().to_string();
        }
        if name.is_empty() || value.is_empty() { continue; }
        out.push((name, value));
    }
    Ok(out)
}

// ===== 值解析 =====

#[derive(Debug, Clone, Copy)]
pub enum Len {
    Px(f32),
    Pct(f32),
    Em(f32),
    Vw(f32),
    Vh(f32),
}

pub fn parse_len(s: &str) -> Option<Len> {
    let s = s.trim();
    if let Some(v) = s.strip_suffix("px") { v.trim().parse::<f32>().ok().map(Len::Px) }
    else if let Some(v) = s.strip_suffix('%') { v.trim().parse::<f32>().ok().map(Len::Pct) }
    else if let Some(v) = s.strip_suffix("em") { v.trim().parse::<f32>().ok().map(Len::Em) }
    else if let Some(v) = s.strip_suffix("rem") { v.trim().parse::<f32>().ok().map(Len::Px) }
    else if let Some(v) = s.strip_suffix("vw") { v.trim().parse::<f32>().ok().map(Len::Vw) }
    else if let Some(v) = s.strip_suffix("vh") { v.trim().parse::<f32>().ok().map(Len::Vh) }
    else { s.parse::<f32>().ok().map(Len::Px) } // 无单位数字按 px（宽松）
}

/// 尺寸表达式：clamp()/min()/max() 包裹的长度（编译期不定值 → 生成 C 运行时表达式）
#[derive(Debug, Clone)]
pub enum Sz {
    Len(Len),
    Clamp(Box<Sz>, Box<Sz>, Box<Sz>), /* clamp(min, val, max) */
    Min(Box<Sz>, Box<Sz>),
    Max(Box<Sz>, Box<Sz>),
}

fn split_top_commas(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut cur = String::new();
    for ch in s.chars() {
        match ch {
            '(' => { depth += 1; cur.push(ch); }
            ')' => { depth -= 1; cur.push(ch); }
            ',' if depth == 0 => { out.push(cur.trim().to_string()); cur = String::new(); }
            _ => cur.push(ch),
        }
    }
    if !cur.trim().is_empty() { out.push(cur.trim().to_string()); }
    out
}

/// 解析尺寸：Len 或 clamp/min/max 表达式
pub fn parse_sz(s: &str) -> Option<Sz> {
    let s = s.trim();
    for (name, ctor) in [("clamp", 3), ("min", 2), ("max", 2)] {
        let prefix = format!("{}(", name);
        if let Some(rest) = s.strip_prefix(&prefix) {
            if !rest.ends_with(')') { return None; }
            let inner = &rest[..rest.len() - 1];
            let parts = split_top_commas(inner);
            if parts.len() != ctor { return None; }
            let parsed: Vec<Sz> = parts.iter().map(|p| parse_sz(p)).collect::<Option<Vec<_>>>()?;
            return match ctor {
                3 => Some(Sz::Clamp(Box::new(parsed[0].clone()), Box::new(parsed[1].clone()), Box::new(parsed[2].clone()))),
                _ => {
                    if name == "min" { Some(Sz::Min(Box::new(parsed[0].clone()), Box::new(parsed[1].clone()))) }
                    else { Some(Sz::Max(Box::new(parsed[0].clone()), Box::new(parsed[1].clone()))) }
                }
            };
        }
    }
    parse_len(s).map(Sz::Len)
}

/// 生成 C 运行时表达式（vw/vh 用视口运行时求值）
pub fn sz_to_c(sz: &Sz, font: f32) -> String {
    match sz {
        Sz::Len(l) => len_to_c(l, font),
        Sz::Clamp(a, b, c) => format!("fminf(fmaxf({}, {}), {})", sz_to_c(a, font), sz_to_c(b, font), sz_to_c(c, font)),
        Sz::Min(a, b) => format!("fminf({}, {})", sz_to_c(a, font), sz_to_c(b, font)),
        Sz::Max(a, b) => format!("fmaxf({}, {})", sz_to_c(a, font), sz_to_c(b, font)),
    }
}

fn len_to_c(l: &Len, font: f32) -> String {
    match l {
        Len::Px(v) => format!("{:.1}f", v),
        Len::Pct(v) => format!("{:.1}f", v), /* 旧有简化：按 px 处理 */
        Len::Em(v) => format!("{:.1}f", v * font),
        Len::Vw(v) => format!("nw_vw({:.2})", v),
        Len::Vh(v) => format!("nw_vh({:.2})", v),
    }
}

/// 编译期常量近似求值（继承链用）：vw/vh 按默认窗口 900x640 假设
pub fn len_const(l: &Len, font: f32) -> f32 {
    match l {
        Len::Px(v) => *v,
        Len::Pct(v) => *v,
        Len::Em(v) => v * font,
        Len::Vw(v) => v * 9.0,
        Len::Vh(v) => v * 6.4,
    }
}

/// clamp/min/max 的常量近似（继承链用）
pub fn sz_const(sz: &Sz, font: f32) -> f32 {
    match sz {
        Sz::Len(l) => len_const(l, font),
        Sz::Clamp(a, b, c) => sz_const(a, font).max(sz_const(b, font).min(sz_const(c, font))),
        Sz::Min(a, b) => sz_const(a, font).min(sz_const(b, font)),
        Sz::Max(a, b) => sz_const(a, font).max(sz_const(b, font)),
    }
}

/// 1-4 值简写切分（括号感知空间切分），返回原始 token。
/// 供 expand_box4_sz 与 margin auto 检测共用（margin 值可为 auto，Sz 表达不了）
pub fn split_box4_tokens(s: &str) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();
    let mut depth = 0i32;
    let mut cur = String::new();
    for ch in s.chars() {
        match ch {
            '(' => { depth += 1; cur.push(ch); }
            ')' => { depth -= 1; cur.push(ch); }
            c if c.is_whitespace() && depth == 0 => {
                if !cur.is_empty() { parts.push(std::mem::take(&mut cur)); }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() { parts.push(cur); }
    parts
}

/// 解析 1-4 值简写（CSS 盒模型顺序），返回 [top, right, bottom, left]。
/// 括号感知切分（clamp(1px,2vh,3px) 不被空格拆散），各部分支持 clamp/min/max/vw/vh
pub fn expand_box4_sz(s: &str) -> Option<[Sz; 4]> {
    let parts = split_box4_tokens(s);
    let lens: Vec<Sz> = parts.iter().map(|p| parse_sz(p)).collect::<Option<Vec<_>>>()?;
    match lens.len() {
        1 => { let v = lens[0].clone(); Some([v.clone(), v.clone(), v.clone(), v]) }
        2 => Some([lens[0].clone(), lens[1].clone(), lens[0].clone(), lens[1].clone()]),
        3 => Some([lens[0].clone(), lens[1].clone(), lens[2].clone(), lens[1].clone()]),
        4 => Some([lens[0].clone(), lens[1].clone(), lens[2].clone(), lens[3].clone()]),
        _ => None,
    }
}

/// 纯长度版（旧接口，clamp 部分退化为 0）
pub fn expand_box4(s: &str) -> Option<[Len; 4]> {
    expand_box4_sz(s).map(|b| b.map(|sz| match sz {
        Sz::Len(l) => l,
        _ => Len::Px(0.0),
    }))
}

/// linear-gradient(...) → 纯色近似：取所有颜色 stop 的平均值（忽略角度/百分比位置）。
/// 传入值应为已做 var() 替换的 resolved 值
pub fn gradient_solid(s: &str) -> Option<[f32; 4]> {
    let start = s.find("linear-gradient(")? + "linear-gradient(".len();
    let b = s.as_bytes();
    let mut depth = 1i32;
    let mut i = start;
    while i < b.len() {
        match b[i] {
            b'(' => depth += 1,
            b')' => { depth -= 1; if depth == 0 { break; } }
            _ => {}
        }
        i += 1;
    }
    let inner = &s[start..i.min(s.len())];
    /* 去掉开头的角度/方向（如 90deg、to right） */
    let stops_part = match inner.find(',') {
        Some(c) => {
            let head = inner[..c].trim();
            if head.ends_with("deg") || head.parse::<f32>().is_ok() || head.starts_with("to ") {
                &inner[c + 1..]
            } else {
                inner
            }
        }
        None => inner,
    };
    /* 括号感知切分 stop（rgb() 内含逗号） */
    let mut stops: Vec<String> = Vec::new();
    let mut d = 0i32;
    let mut cur = String::new();
    for ch in stops_part.chars() {
        match ch {
            '(' => { d += 1; cur.push(ch); }
            ')' => { d -= 1; cur.push(ch); }
            ',' if d == 0 => stops.push(std::mem::take(&mut cur)),
            _ => cur.push(ch),
        }
    }
    if !cur.trim().is_empty() { stops.push(cur); }
    let mut cols: Vec<[f32; 4]> = Vec::new();
    for st in &stops {
        let t = st.trim();
        /* 去掉尾部位置（"#fff 30%" → "#fff"） */
        let tok = if let Some(sp) = t.rfind(' ') {
            let tail = t[sp + 1..].trim();
            if tail.ends_with('%') { t[..sp].trim() } else { t }
        } else { t };
        if let Some(c) = parse_color(tok) { cols.push(c); }
    }
    if cols.is_empty() { return None; }
    if cols.len() == 1 { return Some(cols[0]); }
    let n = cols.len() as f32;
    let mut out = [0f32; 4];
    for k in 0..4 { out[k] = cols.iter().map(|c| c[k]).sum::<f32>() / n; }
    Some(out)
}

/// linear-gradient(...) → (角度deg, [(位置0..1, RGBA), ...])。
/// 角度：CSS 语义（0=向上，90=向右，默认 180=向下）。位置缺省时均分。
pub fn parse_linear_gradient(s: &str) -> Option<(f32, Vec<(f32, [f32; 4])>)> {
    let start = s.find("linear-gradient(")? + "linear-gradient(".len();
    let b = s.as_bytes();
    let mut depth = 1i32;
    let mut i = start;
    while i < b.len() {
        match b[i] {
            b'(' => depth += 1,
            b')' => { depth -= 1; if depth == 0 { break; } }
            _ => {}
        }
        i += 1;
    }
    let inner = &s[start..i.min(s.len())];
    let mut deg = 180.0f32; /* CSS 默认 to bottom */
    let stops_part = match inner.find(',') {
        Some(c) => {
            let head = inner[..c].trim();
            if let Some(d) = head.strip_suffix("deg").and_then(|x| x.trim().parse::<f32>().ok()) {
                deg = d;
                &inner[c + 1..]
            } else if head.starts_with("to ") {
                deg = match head[3..].trim() {
                    "top" => 0.0,
                    "right" => 90.0,
                    "bottom" => 180.0,
                    "left" => 270.0,
                    "top right" | "right top" => 45.0,
                    "bottom right" | "right bottom" => 135.0,
                    "bottom left" | "left bottom" => 225.0,
                    "top left" | "left top" => 315.0,
                    _ => 180.0,
                };
                &inner[c + 1..]
            } else {
                inner
            }
        }
        None => return None,
    };
    /* 括号感知切分 stop */
    let mut stops: Vec<String> = Vec::new();
    let mut d = 0i32;
    let mut cur = String::new();
    for ch in stops_part.chars() {
        match ch {
            '(' => { d += 1; cur.push(ch); }
            ')' => { d -= 1; cur.push(ch); }
            ',' if d == 0 => stops.push(std::mem::take(&mut cur)),
            _ => cur.push(ch),
        }
    }
    if !cur.trim().is_empty() { stops.push(cur); }
    /* 解析每个 stop：颜色 + 可选位置（%/px） */
    let mut out: Vec<(f32, [f32; 4])> = Vec::new();
    for st in &stops {
        let t = st.trim();
        if t.is_empty() { continue; }
        /* 尾部位置（"#fff 30%" / "red 20px"） */
        let (tok, mut pos): (&str, Option<f32>) = if let Some(sp) = t.rfind(' ') {
            let tail = t[sp + 1..].trim();
            if let Some(p) = tail.strip_suffix('%').and_then(|x| x.parse::<f32>().ok()) {
                (t[..sp].trim(), Some(p / 100.0))
            } else if let Ok(px) = tail.parse::<f32>() {
                (t[..sp].trim(), Some(px)) /* px 位置运行时按渐变线长折算困难，先按比例待定 → 用 None 语义 */
            } else { (t, None) }
        } else { (t, None) };
        let _ = &mut pos;
        if let Some(c) = parse_color(tok) {
            out.push((pos.unwrap_or(-1.0), c));
        }
    }
    if out.len() < 2 { return None; }
    /* 缺省位置均分（CSS 规则：无位置 stop 均匀分布） */
    let n = out.len();
    for (k, item) in out.iter_mut().enumerate() {
        if item.0 < 0.0 { item.0 = k as f32 / (n - 1) as f32; }
    }
    Some((deg, out))
}

pub fn parse_color(s: &str) -> Option<[f32; 4]> {
    let s = s.trim().to_ascii_lowercase();
    if let Some(hex) = s.strip_prefix('#') {
        let h = hex.as_bytes();
        let hv = |b: &[u8]| -> Option<f32> {
            if b.len() == 2 { u8::from_str_radix(std::str::from_utf8(b).ok()?, 16).ok().map(|v| v as f32 / 255.0) }
            else if b.len() == 1 { u8::from_str_radix(std::str::from_utf8(b).ok()?, 16).ok().map(|v| (v as f32 * 17.0) / 255.0) }
            else { None }
        };
        return match h.len() {
            3 => Some([hv(&h[0..1])?, hv(&h[1..2])?, hv(&h[2..3])?, 1.0]),
            4 => Some([hv(&h[0..1])?, hv(&h[1..2])?, hv(&h[2..3])?, hv(&h[3..4])?]),
            6 => Some([hv(&h[0..2])?, hv(&h[2..4])?, hv(&h[4..6])?, 1.0]),
            8 => Some([hv(&h[0..2])?, hv(&h[2..4])?, hv(&h[4..6])?, hv(&h[6..8])?]),
            _ => None,
        };
    }
    if let Some(rest) = s.strip_prefix("rgb(") {
        let rest = rest.strip_suffix(')')?;
        let v: Vec<f32> = rest.split(',').map(|p| p.trim().parse().ok()).collect::<Option<Vec<_>>>()?;
        if v.len() == 3 { return Some([v[0] / 255.0, v[1] / 255.0, v[2] / 255.0, 1.0]); }
        return None;
    }
    if let Some(rest) = s.strip_prefix("rgba(") {
        let rest = rest.strip_suffix(')')?;
        let v: Vec<f32> = rest.split(',').map(|p| p.trim().parse().ok()).collect::<Option<Vec<_>>>()?;
        if v.len() == 4 { return Some([v[0] / 255.0, v[1] / 255.0, v[2] / 255.0, v[3]]); }
        return None;
    }
    named_color(&s).map(|c| c)
}

fn named_color(s: &str) -> Option<[f32; 4]> {
    let c = match s {
        "white" => [1.0, 1.0, 1.0, 1.0],
        "black" => [0.0, 0.0, 0.0, 1.0],
        "red" => [1.0, 0.0, 0.0, 1.0],
        "green" => [0.0, 0.5, 0.0, 1.0],
        "lime" => [0.0, 1.0, 0.0, 1.0],
        "blue" => [0.0, 0.0, 1.0, 1.0],
        "yellow" => [1.0, 1.0, 0.0, 1.0],
        "orange" => [1.0, 0.647, 0.0, 1.0],
        "purple" => [0.5, 0.0, 0.5, 1.0],
        "pink" => [1.0, 0.753, 0.796, 1.0],
        "brown" => [0.647, 0.165, 0.165, 1.0],
        "gray" | "grey" => [0.5, 0.5, 0.5, 1.0],
        "silver" => [0.753, 0.753, 0.753, 1.0],
        "navy" => [0.0, 0.0, 0.5, 1.0],
        "teal" => [0.0, 0.5, 0.5, 1.0],
        "maroon" => [0.5, 0.0, 0.0, 1.0],
        "gold" => [1.0, 0.843, 0.0, 1.0],
        "coral" => [1.0, 0.498, 0.314, 1.0],
        "crimson" => [0.863, 0.078, 0.235, 1.0],
        "transparent" => [0.0, 0.0, 0.0, 0.0],
        _ => return None,
    };
    Some(c)
}
