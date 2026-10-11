// Nowebiew M1 - UI 代码生成（J3：DOM 永生 + 运行时级联重算）
// 输入：HTML 文档 + CSS 规则；输出：
//   1. DOM 构建函数（只建一次：结构/文本/属性，不含任何样式）
//   2. 每媒体段一份「每节点样式重算函数」表（编译期级联结果 → 直发 setter）
//   3. nw_app_restyle(variant)：重算入口（变体样式 → 主题重放 → 状态叠加）
// 运行时窗口尺寸变化只调 restyle 重算样式，DOM/类/行内声明/事件绑定全程不动。
// CSS 级联与继承在编译期静态解析（DOM 是编译期已知的）：
//   - 级联：specificity（id=1000 > class=100 > tag=1），同特异性后者胜
//   - 继承：color / font-size / font-weight / font-family / text-align
//   - em：font-size 相对父级字号，margin/padding 相对本元素字号（与浏览器一致）

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use crate::css::{self, Len, Rule};
use crate::html::{Child, Doc, El};

pub struct UiPlan {
    pub code: String,       // 全局变量 + 样式重算表 + nw_app_build_dom + nw_app_restyle + nw_media_init + nw_title
    pub ids: HashSet<String>,
    pub n_variants: usize,        // 媒体段数（运行时按窗口尺寸切换重算样式）
    pub initial_variant: usize,   // 初始窗口 (900,640) 所在段
    pub extra_js: String,         // HTML 内联事件属性编译出的处理函数源码（并入主 JS 管线）
    pub handler_names: Vec<String>, // 内联事件处理函数名（页面脚本降级跳过时需发射空壳包装器）
}

#[derive(Clone)]
struct Inh {
    font_size: f32,
    font_expr: String, // 本节点 font-size 的运行时 C 表达式（vw/vh/% 随窗口实算）
    weight: i32,
    family: Option<String>,
    color: [f32; 4],
    align: i32,
    upper: bool,                 /* text-transform: uppercase（编译期变换文本） */
    color_tmpl: Option<String>,  /* 当前颜色的原始 var() 模板（运行时主题切换用） */
    tbl_colw: Option<std::rc::Rc<Vec<Option<String>>>>, /* 表格首行列宽（浏览器表格布局语义：列宽全表共享） */
    cell_w: Option<String>,      /* 本单元格应取的列宽百分比（tr 级注入） */
}

impl Default for Inh {
    fn default() -> Self {
        Inh { font_size: 16.0, font_expr: "16.0f".into(), weight: 400, family: None, color: [0.0, 0.0, 0.0, 1.0], align: 0,
              upper: false, color_tmpl: None, tbl_colw: None, cell_w: None }
    }
}

// 浏览器 UA 默认样式（M1 子集）
const UA_CSS: &str = "
body { margin: 8px; }
p { margin: 1em 0; }
h1 { font-size: 2em; font-weight: bold; margin: 0.67em 0; }
h2 { font-size: 1.5em; font-weight: bold; margin: 0.83em 0; }
h3 { font-size: 1.17em; font-weight: bold; margin: 1em 0; }
button { background: #efefef; padding: 1px 6px; }
textarea { width: 200px; height: 100px; background: #ffffff; font-family: Consolas; }
head, title, meta, link, style, script { display: none; }
table { display: flex; flex-direction: column; width: 100%; }
tr { display: flex; flex-direction: row; width: 100%; }
td, th { flex-grow: 1; padding: 6px 10px; }
";

/// 目标视口（与 codegen 发射的 nw_app_run 窗口客户区一致）：
/// 初始变体选择与 vw/vh 编译期近似按此视口
pub const VIEWPORT_W: f32 = 900.0;
pub const VIEWPORT_H: f32 = 640.0;

/// 单次走树的发射器。每个媒体段走一遍树：
///   - 初始段（do_cr=true）：创建代码进 cr，样式进每节点重算函数
///   - 其余段：只产生样式重算函数（结构判定全部来自初始段规则，保证节点序对齐）
struct Walk<'a> {
    vk: usize,
    nv: usize,
    pfx: String,                           /* 多页面符号前缀（p0_/p1_...，单页为空） */
    do_cr: bool,
    rules: &'a [Rule],                     /* 本段样式级联用（@media 已过滤） */
    srules: &'a [Rule],                    /* 结构判定用（恒为初始段规则，保证跨段节点序一致） */
    rules_v: &'a [Vec<Rule>],              /* 各段规则集（display 基线规则注册用） */
    seg_bounds: &'a [(f32, f32, f32, f32, u8)],/* 各段视口边界 + 方向（基线规则媒体条件） */
    all_rules: &'a [Rule],                 /* 未过滤全量规则（display 动态规则注册） */
    theme_names: &'a HashSet<String>,
    cr: String,                            /* 创建代码（仅 do_cr 时有意义） */
    stfns: String,                         /* 本段样式重算函数体 */
    sttab: Vec<String>,                    /* 本段函数名表（节点创建序） */
    counter: usize,
    globals: Vec<String>,                  /* 节点全局变量名（创建序） */
}

impl<'a> Walk<'a> {
    fn create(&mut self, line: String) {
        if self.do_cr { let _ = writeln!(self.cr, "    {}", line); }
    }
    /* 发射一个节点的样式重算函数（ord = 创建序号；v = 节点变量名） */
    fn emit_style_fn(&mut self, ord: usize, v: &str, sst: &str) {
        let name = format!("{}nws_v{}_{}", self.pfx, self.vk, ord);
        let _ = writeln!(self.stfns, "static void {}(void) {{", name);
        let _ = writeln!(self.stfns, "    nw_st_reset({});", v);
        let _ = write!(self.stfns, "{}", sst);
        let _ = writeln!(self.stfns, "}}");
        self.sttab.push(name);
    }
}

pub fn generate(doc: &Doc, user_rules: &[Rule], pfx: &str, multi: bool) -> Result<UiPlan, String> {
    let (mut ua_rules, _) = css::parse_css(UA_CSS)?; /* UA 固定子集，无警告 */
    for r in ua_rules.iter_mut() { r.ua = true; }
    let mut base_rules: Vec<Rule> = ua_rules;
    base_rules.extend(user_rules.iter().cloned());
    /* 响应式（@media）：按断点把视口划分为若干段，每段一份「每节点样式重算表」。
       运行时窗口尺寸跨越断点时重算样式（DOM 永生，不重建树）。 */
    let mut segs = css::media_segments(user_rules);
    if segs.len() > 16 { segs.truncate(16); } /* 断点爆炸保护 */
    let mut initial_variant = 0usize;
    for (k, seg) in segs.iter().enumerate() {
        if seg.contains(VIEWPORT_W, VIEWPORT_H) { initial_variant = k; break; }
    }
    let nv = segs.len();

    /* 每段规则集（按段代表视口过滤 @media）+ 段边界（基线规则媒体条件用） */
    let mut rules_v: Vec<Vec<Rule>> = Vec::with_capacity(nv);
    let mut seg_bounds: Vec<(f32, f32, f32, f32, u8)> = Vec::with_capacity(nv);
    for seg in &segs {
        let (vp_w, vp_h) = seg.representative(VIEWPORT_W, VIEWPORT_H);
        let mut rules: Vec<Rule> = base_rules.clone();
        rules.retain(|r| css::media_matches(r.media.as_ref(), vp_w, vp_h));
        let b = |lo: f32, hi: f32| (if lo <= 0.0 { 0.0 } else { lo }, if hi >= 1.0e9 { 1.0e9 } else { hi });
        let (wlo, whi) = b(seg.wlo, seg.whi);
        let (hlo, hhi) = b(seg.hlo, seg.hhi);
        seg_bounds.push((wlo, whi, hlo, hhi, seg.orient));
        rules_v.push(rules);
    }

    /* ===== 运行时主题表：html[data-theme="X"] 规则中的自定义属性按主题分组（视口无关） ===== */
    let mut theme_names: Vec<String> = Vec::new();
    let mut theme_vars: Vec<HashMap<String, String>> = Vec::new();
    let mut base_vars_raw: Vec<(String, String)> = Vec::new();
    for rule in user_rules {
        for sel in &rule.sels {
            let subj = match sel.parts.last() { Some(c) => c, None => continue };
            if !(subj.universal || subj.tag.as_deref() == Some("html")) { continue; }
            let custom: Vec<(String, String)> = rule.decls.iter()
                .filter(|(k, _)| k.starts_with("--")).cloned().collect();
            if custom.is_empty() { continue; }
            let dt = subj.attrs.iter().find(|(k, _)| k == "data-theme").and_then(|(_, v)| v.clone());
            match dt {
                Some(v) => {
                    let idx = match theme_names.iter().position(|t| t == &v) {
                        Some(i) => i,
                        None => { theme_names.push(v); theme_vars.push(HashMap::new()); theme_names.len() - 1 }
                    };
                    for (k, val) in custom {
                        theme_vars[idx].insert(k.trim_start_matches('-').to_string(), val);
                    }
                }
                None => base_vars_raw.extend(custom),
            }
        }
    }
    let mut theme_var_names: HashSet<String> = HashSet::new();
    for tv in &theme_vars { for k in tv.keys() { theme_var_names.insert(k.clone()); } }
    for (k, _) in &base_vars_raw { theme_var_names.insert(k.trim_start_matches('-').to_string()); }
    let init_theme = doc.html_attrs.iter()
        .find(|(k, _)| k == "data-theme")
        .and_then(|(_, v)| theme_names.iter().position(|t| t == v))
        .unwrap_or(0);

    let no_id: Option<String> = None;
    let no_classes: Vec<String> = Vec::new();
    let html_ref = ElemRef { tag: "html", id: &no_id, classes: &no_classes, attrs: &doc.html_attrs };
    let body_ref = ElemRef { tag: "body", id: &doc.body.id, classes: &doc.body.classes, attrs: &doc.body.attrs };

    /* ===== 每段走一遍树 ===== */
    let rootv = format!("{}nwR", pfx); /* 页面根节点变量（多页时带前缀避免冲突） */
    let mut handlers: Vec<(String, String)> = Vec::new(); /* (函数名, JS 体)：内联事件属性 */
    let sm_reg = std::cell::RefCell::new(SmReg::default()); /* 页级 matcher 注册表（跨段共享去重） */
    let mut walks: Vec<Walk> = Vec::with_capacity(nv);
    for vk in 0..nv {
        walks.push(Walk {
            vk, nv,
            pfx: pfx.to_string(),
            do_cr: vk == initial_variant,
            rules: &rules_v[vk],
            srules: &rules_v[initial_variant],
            rules_v: &rules_v,
            seg_bounds: &seg_bounds,
            all_rules: &base_rules,
            theme_names: &theme_var_names,
            cr: String::new(),
            stfns: String::new(),
            sttab: Vec::new(),
            counter: 0,
            globals: Vec::new(),
        });
    }

    for w in walks.iter_mut() {
        /* 级联根：<html> 节点自定义属性（var() 继承链起点，按段求值） */
        let html_decls = cascade(&html_ref, &[], None, w.rules);
        let mut root_vars: HashMap<String, String> = HashMap::new();
        for (k, v) in &html_decls {
            if k.starts_with("--") {
                if let Some(rv) = substitute_vars(v, &root_vars) { root_vars.insert(k.clone(), rv); }
            }
        }

        /* body：创建（仅初始段）+ 每段样式重算函数（ord 0） */
        w.create(format!("{} = nw_dom_create(\"body\", 0, 0);", rootv));
        let mut sst = String::new();
        let body_decls_g = cascade_dyn(&body_ref, &[html_ref.clone()], doc.body.style_attr.as_deref(),
                                       w.rules, &sm_reg, &rootv);
        let body_guards: Vec<Option<String>> = body_decls_g.iter().map(|(_, _, g)| g.clone()).collect();
        let body_decls: Vec<(String, String)> = body_decls_g.iter().map(|(k, v, _)| (k.clone(), v.clone())).collect();
        let mut body_vars = root_vars.clone();
        let mut resolved_body: Vec<(String, String)> = Vec::new();
        let mut resolved_body_g: Vec<Option<String>> = Vec::new();
        for (k, v, g) in &body_decls_g {
            if k.starts_with("--") {
                if g.is_none() {
                    if let Some(rv) = substitute_vars(v, &body_vars) { body_vars.insert(k.clone(), rv); }
                }
            } else if let Some(rv) = substitute_vars(v, &body_vars) {
                resolved_body.push((k.clone(), rv));
                resolved_body_g.push(g.clone());
            }
        }
        let mut inh = Inh::default();
        let mut body_margin = [0.0f32; 4];
        let mut body_bg_url: Option<String> = None;
        let mut body_bg_fit = 3i32;
        let body_raw: HashMap<&String, &String> = body_decls.iter().map(|(k, v)| (k, v)).collect();
        for (gi, (k, v)) in resolved_body.iter().enumerate() {
            let g_on = resolved_body_g[gi].is_some();
            if g_on { sst.push_str(&format!("/*@GS{}*/\n", gi)); }
            match k.as_str() {
                "margin" => {
                    if let Some(b4) = css::expand_box4(v) {
                        for i in 0..4 { body_margin[i] = resolve_len(&b4[i], 16.0); }
                    }
                }
                "background-image" => { if let Some(p) = css_url(v) { body_bg_url = Some(p); } }
                "background-size" => { body_bg_fit = parse_bg_fit(v); }
                "background-color" | "background" => {
                    if let Some(c) = css::parse_color(v) {
                        let _ = writeln!(sst, "    nw_st_bg({rootv}, {});", color_args(c));
                        /* 整窗清屏底色同步：内容不足一屏时窗口其余部分不露白 */
                        let _ = writeln!(sst, "    nw_set_page_bg({});", color_args(c));
                    }
                    if g_on { /* 守卫声明不注册主题重放（重放是无条件应用） */ }
                    else if let Some(raw) = body_raw.get(k) {
                        if raw.contains("var(") && tmpl_theme_ok(raw, w.theme_names) {
                            let _ = writeln!(sst, "    nw_reg_var_decl({rootv}, \"background-color\", \"{}\");", escape_c(raw));
                        }
                    }
                }
                "color" => {
                    /* 守卫（如 body.dark）的 color 不改编译期继承链——子树经
                     * nw_st_color_inh 运行时取色，全量重算后自然传播 */
                    if g_on {
                        if let Some(c) = css::parse_color(v) {
                            let _ = writeln!(sst, "    nw_st_color({rootv}, {});", color_args(c));
                        }
                    } else {
                        if let Some(c) = css::parse_color(v) { inh.color = c; }
                        if let Some(raw) = body_raw.get(k) {
                            if raw.contains("var(") && tmpl_theme_ok(raw, w.theme_names) {
                                let _ = writeln!(sst, "    nw_reg_var_decl({rootv}, \"color\", \"{}\");", escape_c(raw));
                                inh.color_tmpl = Some((*raw).clone());
                            }
                        }
                    }
                }
                "font-size" => {
                    if g_on { /* 同上：不改继承链 */ }
                    else if let Some(l) = css::parse_len(v) {
                        /* 浏览器：em 相对父字号（此前硬编码 16）；% 相对父字号 */
                        inh.font_size = match &l {
                            css::Len::Pct(p) => inh.font_size * p / 100.0,
                            _ => css::len_const(&l, inh.font_size),
                        };
                        inh.font_expr = font_len_c(&l, &inh.font_expr);
                    }
                }
                "font-weight" => { if !g_on { inh.weight = parse_weight(v); } }
                "font-family" => { if !g_on { inh.family = parse_family(v); } }
                "text-align" => { if !g_on { inh.align = parse_align(v); } }
                _ => {}
            }
            if g_on { sst.push_str("/*@GE*/\n"); }
        }
        let _ = writeln!(sst, "    nw_st_padding({rootv}, {:.1}f, {:.1}f, {:.1}f, {:.1}f);",
            body_margin[0], body_margin[1], body_margin[2], body_margin[3]);
        if let Some(p) = body_bg_url {
            let _ = writeln!(sst, "    nw_st_bgimg({rootv}, L\"{}\", {});", escape_c(&p), body_bg_fit);
        }
        /* <body onload="...">：内联事件属性 → 生成处理函数 + load 注册 */
        if w.do_cr {
            if let Some((_, ov)) = doc.body.attrs.iter().find(|(k, _)| k == "onload") {
                let hname = format!("nwh_{}body_l", pfx); /* 页内唯一（多页符号隔离） */
                handlers.push((hname.clone(), ov.clone()));
                w.create(format!("nw_dom_on_load({});", hname));
            }
        }
        let sst = wrap_guards(sst, &resolved_body_g);
        w.emit_style_fn(0, &rootv, &sst);
        w.globals.push(rootv.clone());

        for child in &doc.body.children {
            emit_child(w, &rootv, child, &inh, &mut HashSet::new(), &[],
                       &body_vars, None, false, &mut handlers, &sm_reg)?;
        }
    }

    /* ===== 内联事件属性 → 处理函数源码（并入主 JS 管线一起编译） ===== */
    let mut extra_js = String::new();
    for (name, code) in &handlers {
        let _ = writeln!(extra_js, "function {}() {{\n{}\n}}", name, code);
    }

    /* ===== 组装输出 ===== */
    let iv_walk = &walks[initial_variant];
    let mut out = String::new();
    let _ = writeln!(out, "/* ===== HTML/CSS -> DOM + 样式重算表（编译期合成，媒体段 x{}） ===== */", nv);
    let _ = writeln!(out, "#include \"nw_platform.h\" /* M2: fs/dialog/clipboard */");
    let _ = writeln!(out, "#include <math.h> /* clamp()/min()/max() 生成的 fminf/fmaxf */");
    /* id 全局变量（多页时带页前缀，避免跨页同名 id 冲突） */
    let mut ids: HashSet<String> = HashSet::new();
    collect_ids(&doc.body, &mut ids);
    for id in &ids {
        let _ = writeln!(out, "NWNode* {}nw_el_{} = 0;", pfx, sanitize(id));
    }
    /* 节点全局变量（创建序；样式重算函数与构建函数共享） */
    for g in &iv_walk.globals {
        let _ = writeln!(out, "static NWNode* {} = 0;", g);
    }
    /* 内联事件包装器前置声明（定义在页面 JS 程序内或降级空壳）：
     * 必须在 nw_app_build_dom 之前——构建代码 nw_dom_on_click 按名引用 */
    for (name, _) in &handlers {
        let _ = writeln!(out, "static void {}nww_{}(NWNode* self);", pfx, name);
    }
    let node_count = iv_walk.sttab.len();

    /* 每段样式重算函数 + 函数名表 */
    for w in walks.iter() {
        let _ = writeln!(out, "/* ----- 媒体段 {} 每节点样式重算（编译期级联结果） ----- */", w.vk);
        let _ = write!(out, "{}", w.stfns);
        let _ = writeln!(out, "static void (*{}nws_v{}_tab[])(void) = {{", pfx, w.vk);
        for name in &w.sttab {
            let _ = writeln!(out, "    {},", name);
        }
        let _ = writeln!(out, "}};");
    }

    /* DOM 构建（一次） */
    let _ = writeln!(out);
    let _ = writeln!(out, "void {}nw_app_build_dom(void) {{", pfx);
    /* documentElement 初始属性播种（浏览器语义：getAttribute 反映标记属性；
     * 否则 JS 读 data-theme 永远 NULL → 主题切换每次都重算出同一主题） */
    if !doc.html_attrs.is_empty() {
        let _ = writeln!(out, "    /* documentElement 初始属性 */");
        for (k, v) in &doc.html_attrs {
            let _ = writeln!(out, "    nw_doc_set_attr(nw_str_to_wide(nw_slit(\"{}\")), nw_str_to_wide(nw_slit(\"{}\")));",
                escape_c(k), escape_c(v.as_str()));
        }
    }
    /* 运行时主题变量表（html[data-theme] 自定义属性；注册一次） */
    if !theme_names.is_empty() {
        let _ = writeln!(out, "    /* 运行时主题变量表 */");
        for (i, tv) in theme_vars.iter().enumerate() {
            for (k, v) in &base_vars_raw {
                let _ = writeln!(out, "    nw_var_add({}, \"{}\", \"{}\");",
                    i, k.trim_start_matches('-'), escape_c(v));
            }
            for (k, v) in tv {
                let _ = writeln!(out, "    nw_var_add({}, \"{}\", \"{}\");", i, k, escape_c(v));
            }
        }
    }
    let _ = write!(out, "{}", walks[initial_variant].cr);
    let _ = writeln!(out, "    (void){};", rootv);
    if !theme_names.is_empty() {
        let _ = writeln!(out, "    nw_theme_set({}); /* 初始主题（<html data-theme>） */", init_theme);
    }
    let _ = writeln!(out, "}}");

    /* 样式重算入口：变体基础样式 → 主题 var() 重解析 → 行内声明/display 规则叠加 */
    let _ = writeln!(out);
    let _ = writeln!(out, "void {}nw_app_restyle(int nw_variant) {{", pfx);
    let _ = writeln!(out, "    int nw_n = (int)(sizeof({}nws_v0_tab)/sizeof({}nws_v0_tab[0]));", pfx, pfx);
    let _ = writeln!(out, "    (void)nw_n;");
    for w in walks.iter() {
        let _ = writeln!(out, "    if (nw_variant == {}) {{ for (int nw_i = 0; nw_i < nw_n; nw_i++) {}nws_v{}_tab[nw_i](); }}",
            w.vk, pfx, w.vk);
    }
    if nv == 0 {
        let _ = writeln!(out, "    (void)0;");
    }
    let _ = writeln!(out, "    nw_theme_replay(); /* 当前主题 var() 重解析（重算覆盖了解析值） */");
    let _ = writeln!(out, "    nw_state_reapply(); /* 行内声明叠加 + display 动态规则（级联叠加） */");
    let _ = writeln!(out, "}}");
    let _ = writeln!(out);

    /* 单节点样式重算（JS style.* 写入触发；ordinal = 节点创建序） */
    let _ = writeln!(out, "void {}nw_nws_one(int nw_variant, int nw_i) {{", pfx);
    let _ = writeln!(out, "    int nw_n = (int)(sizeof({}nws_v0_tab)/sizeof({}nws_v0_tab[0]));", pfx, pfx);
    let _ = writeln!(out, "    if (nw_i < 0 || nw_i >= nw_n) return;");
    for w in walks.iter() {
        let _ = writeln!(out, "    if (nw_variant == {}) {{ {}nws_v{}_tab[nw_i](); return; }}", w.vk, pfx, w.vk);
    }
    let _ = writeln!(out, "    {}nws_v0_tab[nw_i]();", pfx);
    let _ = writeln!(out, "}}");
    let _ = writeln!(out);

    let _ = writeln!(out, "const wchar_t* {}nw_title(void) {{ return {}; }}", pfx, wide_lit(&doc.title));
    let _ = writeln!(out);
    /* 页面根节点访问器（多页导航：运行时切换 g_root 用；单页不会被引用） */
    let _ = writeln!(out, "NWNode* {}nw_page_root(void) {{ return {}; }}", pfx, rootv);
    let _ = writeln!(out, "/* 主题名 → 变量表序号（document.setTheme 用；未知名回退 0） */");
    let _ = writeln!(out, "static int {}nw_theme_index(const wchar_t* s) {{", pfx);
    if theme_names.is_empty() {
        let _ = writeln!(out, "    (void)s;");
    }
    for (i, name) in theme_names.iter().enumerate() {
        let _ = writeln!(out, "    if (wcscmp(s, L\"{}\") == 0) return {};", escape_c(name), i);
    }
    let _ = writeln!(out, "    return 0;");
    let _ = writeln!(out, "}}");
    /* data-theme 属性桥：document.documentElement.setAttribute('data-theme', t)
     * 走文档级属性表，运行时按名切主题（与 document.setTheme 同语义） */
    let _ = writeln!(out, "static void {}nw_theme_by_name(const wchar_t* s) {{ nw_theme_set({}nw_theme_index(s)); }}", pfx, pfx);
    let _ = writeln!(out, "static void {}nw_theme_bridge_init(void) {{ nw_dom_on_theme_name({}nw_theme_by_name); }}", pfx, pfx);

    /* 媒体段表（文件作用域：多页模式下经页面注册表交给运行时按需取用） */
    let _ = writeln!(out);
    let _ = writeln!(out, "static NWMediaSeg {}nw_segs_tab[{}] = {{", pfx, nv);
    for seg in &segs {
        let f = |v: f32| if v >= 1.0e9 { "1e9f".to_string() } else { format!("{:.1}f", v) };
        let _ = writeln!(out, "        {{ {} , {} , {} , {} , {} }},", f(seg.wlo), f(seg.whi), f(seg.hlo), f(seg.hhi), seg.orient);
    }
    let _ = writeln!(out, "}};");
    let _ = writeln!(out, "void {}nw_media_init(void) {{", pfx);
    if !multi {
        /* 单页：保持 M1 语义——绑段表 + 重算回调，运行时 resize 自动重算 */
        let _ = writeln!(out, "    nw_media_bind({}nw_segs_tab, {}, {}, {}nw_app_restyle);", pfx, nv, initial_variant, pfx);
    }
    let _ = writeln!(out, "    {}nw_theme_bridge_init(); /* data-theme 属性桥（documentElement.setAttribute） */", pfx);
    let _ = writeln!(out, "}}");
    let _ = writeln!(out);
    let _ = writeln!(out, "/* 节点总数（调试/校验用）：{} */", node_count);

    Ok(UiPlan { code: out, ids, n_variants: nv, initial_variant, extra_js, handler_names: handlers.iter().map(|(n, _)| n.clone()).collect() })
}

fn collect_ids(el: &El, ids: &mut HashSet<String>) {
    if let Some(i) = &el.id { ids.insert(i.clone()); }
    for c in &el.children {
        if let Child::El(e) = c { collect_ids(e, ids); }
    }
}

#[derive(Clone)]
struct ElemRef<'a> {
    tag: &'a str,
    id: &'a Option<String>,
    classes: &'a [String],
    attrs: &'a [(String, String)],
}

/// 编译期级联：命中规则按 (specificity, 文档序) 排序后顺序拼接声明，
/// 内联 style 最后追加（同属性覆盖一切规则）
fn cascade<'a>(
    el: &ElemRef,
    ancestors: &[ElemRef],
    style_attr: Option<&str>,
    rules: &'a [Rule],
) -> Vec<(String, String)> {
    let mut matched: Vec<(u32, usize, &Vec<(String, String)>)> = Vec::new();
    for (ri, rule) in rules.iter().enumerate() {
        let mut best_spec = 0u32;
        let mut hit = false;
        for sel in &rule.sels {
            if let Some(spec) = match_selector(sel, el, ancestors) {
                hit = true;
                best_spec = best_spec.max(spec);
            }
        }
        if hit {
            /* 浏览器来源语义：作者规则恒胜 UA 规则（与特异性无关），加来源位差 */
            let spec = if rule.ua { best_spec } else { best_spec + 1_000_000 };
            matched.push((spec, ri, &rule.decls));
        }
    }
    matched.sort_by_key(|(spec, ri, _)| (*spec, *ri));
    let mut out = Vec::new();
    for (_, _, decls) in matched {
        for d in decls {
            out.push(d.clone());
        }
    }
    /* 内联 style 最后追加：同属性覆盖前面所有规则（就近原则的终点） */
    if let Some(sa) = style_attr {
        match css::parse_decls(sa) {
            Ok(inline) => out.extend(inline),
            Err(e) => { /* 内联样式解析失败按浏览器宽松语义忽略该声明 */ let _ = e; }
        }
    }
    out
}

fn compound_spec(c: &css::Compound) -> u32 {
    /* 浏览器规则：id×1000 + class/属性×100 + tag×1（通配符 0） */
    c.classes.len() as u32 * 100 + c.attrs.len() as u32 * 100
        + c.tag.is_some() as u32 + c.id.is_some() as u32 * 1000
}

fn compound_matches(c: &css::Compound, tag: &str, id: &Option<String>, classes: &[String], attrs: &[(String, String)]) -> bool {
    if c.universal { return true; }
    if let Some(t) = &c.tag {
        if tag != t { return false; }
    }
    if let Some(i) = &c.id {
        if id.as_deref() != Some(i.as_str()) { return false; }
    }
    for cl in &c.classes {
        if !classes.contains(cl) { return false; }
    }
    for (an, av) in &c.attrs {
        let hit = attrs.iter().any(|(k, v)| {
            k == an && match av {
                Some(want) => v == want,
                None => true,
            }
        });
        if !hit { return false; }
    }
    true
}

/// 主体部分匹配元素本身；其余部分从最近祖先向上贪心匹配（后代组合器）。
/// 返回 specificity；None = 不匹配
/// 模板中引用的所有 var() 名是否都在主题变量表内（决定是否注册运行时重解析）
fn tmpl_theme_ok(tmpl: &str, names: &HashSet<String>) -> bool {
    let b = tmpl.as_bytes();
    let mut i = 0usize;
    while i < b.len() {
        if b[i] == b'v' && tmpl[i..].starts_with("var(") {
            let rest = &tmpl[i + 4..];
            let end = rest.find(|c| c == ',' || c == ')').unwrap_or(rest.len());
            let mut nm = rest[..end].trim();
            if let Some(s) = nm.strip_prefix("--") { nm = s; }
            if !names.contains(nm) { return false; }
            i += 4 + end;
        } else {
            i += 1;
        }
    }
    true
}

/// inset 声明值 → (C 表达式, kind)：百分比→kind 2，其余（px/em/vw/vh/clamp）→kind 1 运行时求值
fn inset_val_c(val: &str, font: f32) -> Option<(String, i32)> {
    if let Some(l) = css::parse_len(val) {
        if let Len::Pct(p) = l { return Some((format!("{:.2}f", p), 2)); }
        return Some((css::sz_to_c(&css::Sz::Len(l), font), 1));
    }
    if let Some(sz) = css::parse_sz(val) { return Some((css::sz_to_c(&sz, font), 1)); }
    None
}

fn color_args(c: [f32; 4]) -> String {
    format!("{:.4}f, {:.4}f, {:.4}f, {:.4}f", c[0], c[1], c[2], c[3])
}

/// 解析 border 简写 "1px solid <color>" → (宽px, 颜色原始串)；解析失败返回 None
fn parse_border_shorthand(val: &str) -> Option<(f32, String)> {
    /* 括号感知切分（var(--x, #fff) 不被空格拆散） */
    let mut parts: Vec<String> = Vec::new();
    let mut depth = 0i32;
    let mut cur = String::new();
    for ch in val.chars() {
        match ch {
            '(' => { depth += 1; cur.push(ch); }
            ')' => { depth -= 1; cur.push(ch); }
            c if c.is_whitespace() && depth == 0 => {
                if !cur.is_empty() { parts.push(std::mem::take(&mut cur)); }
            }
            c => cur.push(ch),
        }
    }
    if !cur.is_empty() { parts.push(cur); } /* 收尾：最后一段（无尾随空格时） */
    if parts.len() < 2 { return None; }
    let w = css::parse_len(&parts[0]).map(|l| css::len_const(&l, 16.0))?;
    /* 第二个 token 是样式（solid/dashed...），颜色 = 其后第一个可解析为颜色的 token 或 var() */
    let col = parts[2..].iter().find(|p| p.contains("var(") || css::parse_color(p).is_some())?;
    Some((w, col.clone()))
}

/// 解析 box-shadow "<ox> <oy> [<blur> [<spread>]] <color>"（外阴影；inset 返回 None）
struct BoxShadow { lens: Vec<String>, color: String }

fn parse_box_shadow(val: &str) -> Option<BoxShadow> {
    /* 括号感知切分（rgba(0,0,0,.5) 不被空格拆散） */
    let mut parts: Vec<String> = Vec::new();
    let mut depth = 0i32;
    let mut cur = String::new();
    for ch in val.chars() {
        match ch {
            '(' => { depth += 1; cur.push(ch); }
            ')' => { depth -= 1; cur.push(ch); }
            c if c.is_whitespace() && depth == 0 => {
                if !cur.is_empty() { parts.push(std::mem::take(&mut cur)); }
            }
            c => cur.push(ch),
        }
    }
    if !cur.is_empty() { parts.push(cur); } /* 收尾：最后一段（无尾随空格时） */
    if parts.iter().any(|p| p == "inset") { return None; }
    /* 颜色 = 最后一个可解析为颜色/含 var 的 token；其余为长度（2~4 个） */
    let ci = parts.iter().rposition(|p| p.contains("var(") || css::parse_color(p).is_some())?;
    if ci < 2 || ci > 4 { return None; }
    Some(BoxShadow {
        lens: parts[..ci].to_vec(),
        color: parts[ci].clone(),
    })
}

fn match_selector(sel: &css::Sel, el: &ElemRef, ancestors: &[ElemRef]) -> Option<u32> {
    match_selector_impl(sel, el, ancestors, false)
}

/// display 规则注册专用放宽匹配：主 compound 的类名与元素静态类**交集非空**即可
/// （JS 运行时 classList 变更后，由 reapply_display 按真实类名重评）；
fn match_selector_display(sel: &css::Sel, el: &ElemRef, ancestors: &[ElemRef]) -> Option<u32> {
    match_selector_impl(sel, el, ancestors, true)
}

fn match_selector_impl(sel: &css::Sel, el: &ElemRef, ancestors: &[ElemRef], relax_classes: bool) -> Option<u32> {
    let subject = sel.parts.last()?;
    if relax_classes {
        if !subject.classes.iter().any(|c| el.classes.iter().any(|x| x == c)) { return None; }
        /* 类已验证交集：把主体自己的类当作元素类传入，跳过类全含检查 */
        if !compound_matches(subject, el.tag, el.id, &subject.classes, el.attrs) { return None; }
    } else {
        if !compound_matches(subject, el.tag, el.id, el.classes, el.attrs) { return None; }
    }
    let mut spec = compound_spec(subject);
    let mut top = ancestors.len(); /* 从最近祖先开始向上找 */
    for part in sel.parts[..sel.parts.len() - 1].iter().rev() {
        let mut found = None;
        for k in (0..top).rev() {
            if compound_matches(part, ancestors[k].tag, ancestors[k].id, ancestors[k].classes, ancestors[k].attrs) {
                found = Some(k);
                break;
            }
        }
        let k = found?;
        spec += compound_spec(part);
        top = k; /* 更外层的部分只能在 k 以左匹配 */
    }
    Some(spec)
}

/// ===== 运行时选择器 matcher（编译期生成守卫函数） =====
/// 含类名/:hover 的选择器不能编译期一次烘焙（classList 变更、悬停会改变匹配）。
/// 策略：静态命中的规则照旧烘焙；纯动态命中（编译期类名未出现）的规则生成
/// `if (sm_K(节点)) { ...声明... }` 守卫块，运行时 classList/悬停变化后全量重算。
/// cp_K = 单 compound 测试（tag/id/类/:hover/属性），sm_K = 整条选择器（后代组合器
/// 沿祖先链贪心匹配，与编译期 match_selector 同语义）。
/// 动态规则警告去重（守卫本体是运行时 nw_sel_match 规格串，无需函数注册表）
#[derive(Default)]
struct SmReg {
    warned: std::collections::HashSet<String>,
}

/// 选择器 → 运行时规格串：compound = 逗号分隔 token（t:tag i:id c:cls h a:name[=val]），
/// 部分（Part）之间 ">>"（后代组合器，左=最外祖先，右=主体）。运行时 nw_sel_match 求值。
fn sel_spec(sel: &css::Sel) -> String {
    let mut parts = Vec::new();
    for c in &sel.parts {
        let mut toks: Vec<String> = Vec::new();
        if let Some(t) = &c.tag { toks.push(format!("t:{}", t)); }
        if let Some(i) = &c.id { toks.push(format!("i:{}", i)); }
        for cl in &c.classes { toks.push(format!("c:{}", cl)); }
        if c.hover { toks.push("h".into()); }
        for (an, av) in &c.attrs {
            match av {
                Some(v) => toks.push(format!("a:{}={}", an, v)),
                None => toks.push(format!("a:{}", an)),
            }
        }
        parts.push(if toks.is_empty() { "*".into() } else { toks.join(",") });
    }
    parts.join(">>")
}

/// 守卫块不支持编译期收集类属性（margin/padding/定位/背景图等在循环后统一发射，
/// 无法条件化）；含 var() 的声明会被主题重放无条件重应用；display 走注册表。
/// 这些声明在动态规则里 → 丢弃 + 警告（浏览器语义子集边界）。
fn guard_unsupported(k: &str, val: &str) -> bool {
    const U: &[&str] = &[
        "margin", "margin-top", "margin-right", "margin-bottom", "margin-left",
        "padding", "padding-top", "padding-right", "padding-bottom", "padding-left",
        "position", "top", "right", "bottom", "left", "inset",
        "border-radius", "box-shadow", "background-image", "background-size",
        "grid-template-columns", "display", "gap", "flex-wrap", "transition",
    ];
    U.contains(&k) || val.contains("var(") || k.starts_with("--")
}

/// 动态级联：静态命中 → 无守卫（今日路径原样）；未命中但有动态选择器 → 声明带
/// 守卫发出（无论编译期是否命中）；结构 spec 排序与 cascade 一致。
/// 返回 (声明流, 每条声明的守卫条件 None=无条件)。
fn cascade_dyn<'a>(
    el: &ElemRef,
    ancestors: &[ElemRef],
    style_attr: Option<&str>,
    rules: &'a [Rule],
    sm: &std::cell::RefCell<SmReg>,
    var: &str,
) -> Vec<(String, String, Option<String>)> {
    let mut sm = sm.borrow_mut();
    let mut matched: Vec<(u32, usize, &Vec<(String, String)>, Option<String>)> = Vec::new();
    for (ri, rule) in rules.iter().enumerate() {
        let mut best_spec = 0u32;
        let mut static_hit = false;
        let mut has_dyn = false;
        let mut dyn_spec = 0u32;
        for sel in &rule.sels {
            let has_hover = sel.parts.iter().any(|p| p.hover);
            if !has_hover {
                /* 编译期匹配（类名以 HTML 静态类为准）：命中 → 无条件烘焙（今日路径）。
                 * 注意带 :hover 的选择器不能算静态命中（悬停态运行时才知道） */
                if let Some(spec) = match_selector(sel, el, ancestors) {
                    static_hit = true;
                    best_spec = best_spec.max(spec);
                }
            }
            if css::sel_is_dynamic(sel) {
                has_dyn = true;
                /* 结构 spec：subject + 全部祖先部分的和（与 match_selector 同式） */
                let mut s = 0u32;
                for p in &sel.parts { s += compound_spec(p); }
                dyn_spec = dyn_spec.max(s);
            }
        }
        if !static_hit && !has_dyn { continue; }
        let spec = if rule.ua { best_spec.max(dyn_spec) } else { best_spec.max(dyn_spec) + 1_000_000 };
        let guard = if static_hit || !has_dyn {
            None
        } else {
            let ors: Vec<String> = rule.sels.iter()
                .filter(|s2| css::sel_is_dynamic(s2))
                .map(|s2| format!("nw_sel_match({}, \"{}\")", var, sel_spec(s2)))
                .collect();
            Some(ors.join(" || "))
        };
        matched.push((spec, ri, &rule.decls, guard));
    }
    matched.sort_by_key(|(spec, ri, _, _)| (*spec, *ri));
    let mut out = Vec::new();
    for (_, ri, decls, guard) in &matched {
        let rule = &rules[*ri];
        for (k, v) in decls.iter() {
            /* 有守卫且属性属于编译期收集类/var/display → 丢弃（guard_unsupported 语义）；
             * 无守卫（静态命中）时全部照旧烘焙 */
            if guard.is_some() && guard_unsupported(k, v) {
                /* 仅当规则主体与本元素相关（tag/类/id 有交集）才告警：
                 * 未命中元素走守卫分支丢弃声明是正常路径，不是损失 */
                if sels_relevant(&rule.sels, el) && sm.warned.insert(format!("{}|{}", ri, k)) {
                    eprintln!("警告: 动态规则属性 '{}' 暂不支持运行时条件（相关声明已忽略）", k);
                }
                continue;
            }
            out.push((k.clone(), v.clone(), guard.clone()));
        }
    }
    /* 内联 style 最后追加（无守卫） */
    if let Some(sa) = style_attr {
        match css::parse_decls(sa) {
            Ok(inline) => for (k, v) in inline { out.push((k, v, None)); },            Err(_) => {}
        }
    }
    out
}

/// 规则主体选择器是否可能指向该元素（tag/类/id 任一交集）——告警相关性过滤
fn sels_relevant(sels: &[css::Sel], el: &ElemRef) -> bool {
    sels.iter().any(|sel| {
        let Some(subj) = sel.parts.last() else { return false };
        if let Some(t) = &subj.tag {
            if t != el.tag { return false; }
        }
        if let Some(i) = &subj.id {
            if el.id.as_deref() != Some(i.as_str()) { return false; }
        }
        if !subj.classes.is_empty()
            && !subj.classes.iter().any(|c| el.classes.iter().any(|x| x == c))
        {
            return false;
        }
        true
    })
}

/// 把 sst 中由 /*@GS{i}*/ ... /*@GE*/ 标记的段包成 `if (守卫) { ... }`
fn wrap_guards(sst: String, guards: &[Option<String>]) -> String {
    if !guards.iter().any(|g| g.is_some()) { return sst; }
    let mut res = String::new();
    let mut cur: Option<(usize, Vec<String>)> = None; /* (声明序, 行缓冲) */
    for line in sst.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("/*@GS") {
            if let Some((idx, buf)) = cur.take() { flush_guard(&mut res, idx, buf, guards); }
            let idx: usize = rest.trim_end_matches("*/").trim().parse().unwrap_or(usize::MAX);
            cur = Some((idx, Vec::new()));
            continue;
        }
        if t == "/*@GE*/" {
            if let Some((idx, buf)) = cur.take() { flush_guard(&mut res, idx, buf, guards); }
            continue;
        }
        match &mut cur {
            Some((_, buf)) => buf.push(line.to_string()),
            None => { res.push_str(line); res.push('\n'); }
        }
    }
    if let Some((idx, buf)) = cur.take() { flush_guard(&mut res, idx, buf, guards); }
    res
}

fn flush_guard(res: &mut String, idx: usize, buf: Vec<String>, guards: &[Option<String>]) {
    if buf.is_empty() { return; }
    match guards.get(idx).cloned().flatten() {
        Some(g) => {
            res.push_str(&format!("    if ({}) {{\n", g));
            for l in &buf { res.push_str(l); res.push('\n'); }
            res.push_str("    }\n");
        }
        None => for l in &buf { res.push_str(l); res.push('\n'); },
    }
}

/// @media 条件 → (minw, maxw, minh, maxh, orient)（无界用 1e9；无条件全 0/1e9）
fn media_bounds(m: &Option<css::Media>) -> (f32, f32, f32, f32, u8) {
    match m {
        None => (0.0, 1.0e9, 0.0, 1.0e9, 0),
        Some(m) => (
            m.minw.unwrap_or(0.0),
            m.maxw.unwrap_or(1.0e9),
            m.minh.unwrap_or(0.0),
            m.maxh.unwrap_or(1.0e9),
            m.orient,
        ),
    }
}

fn fb(v: f32) -> String { if v >= 1.0e9 { "1e9f".into() } else { format!("{:.1}f", v) } }

#[allow(clippy::too_many_arguments)]
fn emit_child(
    w: &mut Walk,
    parent_var: &str,
    child: &Child,
    inh: &Inh,
    _ids: &mut HashSet<String>,
    chain: &[ElemRef],
    vars: &HashMap<String, String>,
    grid_child_pct: Option<f32>,
    skip_marker: bool,
    handlers: &mut Vec<(String, String)>,
    sm: &std::cell::RefCell<SmReg>,
) -> Result<(), String> {
    match child {
        Child::Text(text) => {
            w.counter += 1;
            let ord = w.counter;
            let v = format!("{}nwT{}", w.pfx, ord);
            /* ul>li 列表标记（::before 近似）：前置到首个文本子节点（匿名 linebox 层不计入祖先） */
            let mut t = text.clone();
            let eff: Vec<&ElemRef> = chain.iter().filter(|e| e.tag != "linebox").collect();
            if !skip_marker {
                if let Some(p) = eff.last() {
                    if p.tag == "li" {
                        if let Some(gp) = eff.iter().rev().nth(1) {
                            if gp.tag == "ul" {
                                let has = |c: &str| gp.classes.iter().any(|x| x == c);
                                let m = if has("arrow") { "▸ " }
                                    else if has("check") { "✓ " }
                                    else if has("no") { "✕ " }
                                    else { "• " };
                                t = format!("{}{}", m, t);
                            }
                        }
                    }
                }
            }
            if inh.upper { t = t.to_uppercase(); }
            w.create(format!("{} = nw_dom_text({});", v, wide_lit(&t)));
            w.create(format!("nw_dom_append({}, {});", parent_var, v));
            w.globals.push(v.clone());
            let mut sst = String::new();
            /* 文本颜色运行时继承：沿祖先链找最近显式设色节点（body.dark 等
             * 守卫颜色经全量重算自然传播，无需编译期继承重烘焙） */
            let _ = writeln!(sst, "    nw_st_color_inh({});", v);
            if let Some(tmpl) = &inh.color_tmpl {
                if tmpl.contains("var(") && tmpl_theme_ok(tmpl, w.theme_names) {
                    let _ = writeln!(sst, "    nw_reg_var_decl({}, \"color\", \"{}\");", v, escape_c(tmpl));
                }
            }
            let fam = match &inh.family {
                Some(f) => wide_lit(f),
                None => "0".to_string(),
            };
            let _ = writeln!(sst, "    nw_st_font({}, {:.1}f, {}, {});", v, inh.font_size, inh.weight, fam);
            let _ = writeln!(sst, "    nw_st_text_align({}, {});", v, inh.align);
            w.emit_style_fn(ord, &v, &sst);
            Ok(())
        }
        Child::El(el) => {
            w.counter += 1;
            let ord = w.counter;
            let v = format!("{}nwE{}", w.pfx, ord);
            let id_c = match &el.id {
                Some(i) => format!("\"{}\"", escape_c(i)),
                None => "0".to_string(),
            };
            let cls_c = if el.classes.is_empty() {
                "0".to_string()
            } else {
                format!("\"{}\"", escape_c(&el.classes.join(" ")))
            };
            w.create(format!("{} = nw_dom_create(\"{}\", {}, {});", v, el.tag, id_c, cls_c));
            if let Some(i) = &el.id {
                w.create(format!("{}nw_el_{} = {};", w.pfx, sanitize(i), v));
            }
            w.create(format!("nw_dom_append({}, {});", parent_var, v));
            /* 属性登记（dataset/getAttribute 的运行时数据源；id/class 已单独传） */
            for (ak, av) in &el.attrs {
                if ak == "id" || ak == "class" || ak == "style" { continue; }
                w.create(format!("nw_el_attr({}, \"{}\", \"{}\");", v, escape_c(ak), escape_c(av)));
            }
            /* 内联事件属性（浏览器 on* 属性语义）：onclick 编译为处理函数并绑定 */
            if w.do_cr {
                for (ak, av) in &el.attrs {
                    if ak == "onclick" {
                        let fname = format!("nwh_{}{}_c", w.pfx, ord);
                        handlers.push((fname.clone(), av.clone()));
                        /* nwh_* 是 ()->void 的编译器内部函数，经 codegen 的 nww_ 包装器
                         * 适配 C 回调签名 void(*)(NWNode*)（与 .onclick=fn 通道同一路径） */
                        w.create(format!("nw_dom_on_click({}, {}nww_{});", v, w.pfx, fname));
                    }
                }
            }
            /* <img src>：替换型元素（浏览器 img 语义——资源已在编译期嵌入，运行时解码绘制） */
            if el.tag == "img" {
                if let Some((_, sv)) = el.attrs.iter().find(|(k, _)| k == "src") {
                    w.create(format!("nw_el_img_src({}, L\"{}\");", v, escape_c(&norm_rel(sv))));
                }
            }
            w.globals.push(v.clone());

            let el_ref = ElemRef { tag: &el.tag, id: &el.id, classes: &el.classes, attrs: &el.attrs };

            /* 结构判定（恒用初始段规则 → 各段节点序严格一致） */
            let sdecls = cascade(&el_ref, chain, el.style_attr.as_deref(), w.srules);
            let disp_is_flexgrid_s = sdecls.iter().any(|(k, val)| {
                k == "display" && matches!(val.trim(), "flex" | "grid")
            });
            let is_grid_s = sdecls.iter().any(|(k, val)| k == "display" && val.trim() == "grid");
            let ncols_s: Option<usize> = sdecls.iter().rev().find(|(k, _)| k == "grid-template-columns")
                .and_then(|(_, val)| css::parse_grid_cols(val));

            /* ===== 本段样式级联 ===== */
            let mut sst = String::new();
            /* <img> 宽高属性（浏览器表现属性语义）：发射在守卫调用**之前**——
             * 表现属性 = 特异性 0 的作者声明，命中规则（含动态守卫）运行时覆盖。
             * 是否跳过只看静态级联 sdecls：动态守卫规则（如 .background{width:100%}）
             * 在 resolved 里恒在但运行时未必命中，不能据此抑制属性尺寸（#49） */
            if el.tag == "img" {
                if !sdecls.iter().any(|(k, _)| k == "width") {
                    if let Some((_, wv)) = el.attrs.iter().find(|(k, _)| k == "width") {
                        emit_size(&mut sst, &v, wv, inh.font_size, true);
                    }
                }
                if !sdecls.iter().any(|(k, _)| k == "height") {
                    if let Some((_, hv)) = el.attrs.iter().find(|(k, _)| k == "height") {
                        emit_size(&mut sst, &v, hv, inh.font_size, false);
                    }
                }
            }
            let decls_g = cascade_dyn(&el_ref, chain, el.style_attr.as_deref(), w.rules,
                                      &sm, &v);
            let decls: Vec<(String, String)> = decls_g.iter().map(|(k, v2, _)| (k.clone(), v2.clone())).collect();
            /* 自定义属性收集 + var() 编译期替换。
               两遍处理：先收集全部 --xx（CSS 语义：自定义属性与使用顺序无关），
               再替换普通声明——否则同元素内「--sc 定义在 var(--sc) 使用之后」（内联在后）
               会错误落到 fallback。 */
            let mut my_vars = vars.clone();
            for (k, v) in &decls {
                if k.starts_with("--") {
                    if let Some(rv) = substitute_vars(v, &my_vars) { my_vars.insert(k.clone(), rv); }
                }
            }
            let mut resolved: Vec<(String, String)> = Vec::new();
            let mut resolved_g: Vec<Option<String>> = Vec::new();
            for (k, v, g) in &decls_g {
                if !k.starts_with("--") {
                    if let Some(rv) = substitute_vars(v, &my_vars) {
                        resolved.push((k.clone(), rv));
                        resolved_g.push(g.clone());
                    }
                }
            }
            let mut inh2 = inh.clone();
            let mut margin: Option<[String; 4]> = None;
            let mut margin_auto = [false; 4]; /* margin:auto 侧（水平 auto=块盒居中） */
            let mut padding: Option<[String; 4]> = None;
            let mut has_padding = false;
            let mut my_grid_child_pct: Option<f32> = None;
            /* 定位收集（position 可先于/后于 inset 声明，循环后统一发射） */
            let mut pos_kind = 0i32;
            let mut inset_expr: [Option<String>; 4] = [None, None, None, None];
            let mut inset_kind: [i32; 4] = [0, 0, 0, 0];
            /* 原始（未替换 var）声明值：主题注册用 */
            let raw_of: HashMap<&String, &String> = decls.iter().map(|(k, v)| (k, v)).collect();
            /* 浏览器语义：display:flex 的默认方向是 row（仅当未显式声明 flex-direction）。
             * 只看 sdecls（静态命中）——decls 含未命中动态规则的守卫声明（任何动态
             * flex-direction 规则会全局抑制默认 row 发射，曾致全部 flex 容器竖排）；
             * 动态 flex-direction 由守卫在运行时覆盖，guard 假时回落默认 row 正确 */
            let has_flex_dir = sdecls.iter().any(|(k, _)| k == "flex-direction");
            let mut is_grid = false;
            let mut grid_cols: Option<usize> = None;
            let mut grid_gap_c: String = "0.0f".into(); /* gap 运行时表达式 */
            /* 背景图（background-image + background-size 按级联序组合，循环后统一发射） */
            let mut bg_url: Option<String> = None;
            let mut bg_fit = 3i32; /* 浏览器默认：固有尺寸平铺（此处近似不平铺） */
            for (gi, (k, val)) in resolved.iter().enumerate() {
                let g_on = resolved_g[gi].is_some();
                if g_on { sst.push_str(&format!("/*@GS{}*/\n", gi)); }
                match k.as_str() {
                    "display" => match val.trim() {
                        "flex" => {
                            let _ = writeln!(sst, "    nw_st_display({}, 1);", v);
                            if !has_flex_dir {
                                let _ = writeln!(sst, "    nw_st_flex_dir({}, 0); /* flex 默认 row */", v);
                            }
                        }
                        "grid" => {
                            /* grid 子集：方向与等宽子项在声明收集后统一处理 */
                            let _ = writeln!(sst, "    nw_st_display({}, 1);", v);
                            is_grid = true;
                        }
                        "block" => {
                            /* block 流 = 纵向堆叠（flex column 近似） */
                            let _ = writeln!(sst, "    nw_st_display({}, 1);", v);
                            let _ = writeln!(sst, "    nw_st_flex_dir({}, 1);", v);
                        }
                        "none" => { let _ = writeln!(sst, "    nw_st_display({}, 0);", v); }
                        _ => {}
                    },
                    "grid-template-columns" => {
                        if let Some(n) = css::parse_grid_cols(val) { grid_cols = Some(n); }
                    }
                    "flex-direction" => match val.trim() {
                        "row" => { let _ = writeln!(sst, "    nw_st_flex_dir({}, 0);", v); }
                        "column" => { let _ = writeln!(sst, "    nw_st_flex_dir({}, 1);", v); }
                        _ => {}
                    },
                    "justify-content" => {
                        let j = match val.trim() {
                            "center" => 1, "flex-end" | "end" => 2,
                            "space-between" => 3, "space-around" => 4, "space-evenly" => 5,
                            _ => 0,
                        };
                        let _ = writeln!(sst, "    nw_st_justify({}, {});", v, j);
                    }
                    "align-items" => {
                        let a = match val.trim() {
                            "center" => 1, "flex-end" | "end" => 2,
                            "stretch" => 3, "baseline" => 4,
                            _ => 0,
                        };
                        let _ = writeln!(sst, "    nw_st_align({}, {});", v, a);
                    }
                    "flex-wrap" => match val.trim() {
                        "wrap" => { let _ = writeln!(sst, "    nw_st_flex_wrap({}, 1);", v); }
                        _ => {}
                    },
                    "gap" => {
                        if let Some(sz) = css::parse_sz(val) {
                            grid_gap_c = css::sz_to_c(&sz, inh2.font_size);
                            let _ = writeln!(sst, "    nw_st_gap({}, {});", v, grid_gap_c);
                        }
                    }
                    "padding" => {
                        if let Some(b4) = css::expand_box4_sz(val) {
                            let f: Vec<String> = b4.iter().map(|l| css::sz_to_c(l, inh2.font_size)).collect();
                            padding = Some([f[0].clone(), f[1].clone(), f[2].clone(), f[3].clone()]);
                            has_padding = true;
                        }
                    }
                    /* 单边 padding：与简写同通道，级联序后者覆盖同边 */
                    "padding-top" | "padding-right" | "padding-bottom" | "padding-left" => {
                        if let Some(sz) = css::parse_sz(val) {
                            let side = match k.as_str() {
                                "padding-top" => 0, "padding-right" => 1,
                                "padding-bottom" => 2, _ => 3,
                            };
                            let p = padding.get_or_insert([
                                "0.0f".into(), "0.0f".into(), "0.0f".into(), "0.0f".into(),
                            ]);
                            p[side] = css::sz_to_c(&sz, inh2.font_size);
                            has_padding = true;
                        }
                    }
                    "margin" => {
                        /* 逐值解析，支持 auto（浏览器块盒：水平 auto margin = 居中）。
                         * auto 无法用 Sz 表达（parse_sz 返回 None 会整条丢弃），
                         * 故按 token 切分逐值判定 */
                        let parts = css::split_box4_tokens(val);
                        let ok = !parts.is_empty() && parts.len() <= 4
                            && parts.iter().all(|p| p == "auto" || css::parse_sz(p).is_some());
                        if ok {
                            let pick = |i: usize| -> usize {
                                match parts.len() {
                                    1 => 0,
                                    2 => if i % 2 == 0 { 0 } else { 1 },
                                    3 => if i == 3 { 1 } else { i },
                                    _ => i,
                                }
                            };
                            let mut m = margin.take().unwrap_or([
                                "0.0f".into(), "0.0f".into(), "0.0f".into(), "0.0f".into(),
                            ]);
                            for i in 0..4 {
                                let p = parts[pick(i)].clone();
                                if p == "auto" {
                                    margin_auto[i] = true;
                                    m[i] = "0.0f".into();
                                } else {
                                    margin_auto[i] = false;
                                    m[i] = css::sz_to_c(&css::parse_sz(&p).unwrap(), inh2.font_size);
                                }
                            }
                            margin = Some(m);
                        }
                    }
                    /* 单边 margin（margin-top 等）：与简写同通道，级联序后者覆盖 */
                    "margin-top" | "margin-right" | "margin-bottom" | "margin-left" => {
                        let side = match k.as_str() {
                            "margin-top" => 0, "margin-right" => 1,
                            "margin-bottom" => 2, _ => 3,
                        };
                        if val.trim() == "auto" {
                            margin_auto[side] = true;
                            let m = margin.get_or_insert([
                                "0.0f".into(), "0.0f".into(), "0.0f".into(), "0.0f".into(),
                            ]);
                            m[side] = "0.0f".into();
                        } else if let Some(sz) = css::parse_sz(val) {
                            margin_auto[side] = false;
                            let m = margin.get_or_insert([
                                "0.0f".into(), "0.0f".into(), "0.0f".into(), "0.0f".into(),
                            ]);
                            m[side] = css::sz_to_c(&sz, inh2.font_size);
                        }
                    }
                    "overflow" => {
                        /* hidden=内容裁剪到盒；visible/auto/scroll 暂同 visible（滚动系统 T1） */
                        let h = if val.trim() == "hidden" { 1 } else { 0 };
                        let _ = writeln!(sst, "    nw_st_overflow({}, {});", v, h);
                    }
                    "width" | "height" => {
                        if val.trim() == "auto" {
                            /* auto = 清除显式尺寸（kind=0）。守卫级联下可覆盖静态 px
                             * 高度（如 .article-content{height:0} + .open{height:auto}） */
                            let setter = if k == "width" { "nw_st_width_auto" } else { "nw_st_height_auto" };
                            let _ = writeln!(sst, "    {}({});", setter, v);
                        } else if k == "width" {
                            emit_size(&mut sst, &v, val, inh2.font_size, true);
                        } else {
                            emit_size(&mut sst, &v, val, inh2.font_size, false);
                        }
                    }
                    "max-width" => emit_max_w(&mut sst, &v, val, inh2.font_size),
                    "background-image" => { if let Some(p) = css_url(val) { bg_url = Some(p); } }
                    "background-size" => { bg_fit = parse_bg_fit(val); }
                    "background-color" | "background" => {
                        if let Some(c) = css::parse_color(val) {
                            let _ = writeln!(sst, "    nw_st_bg({}, {});", v, color_args(c));
                        } else if let Some((deg, stops)) = css::parse_linear_gradient(val) {
                            /* 真线性渐变：运行时 LinearGradientBrush */
                            let _ = writeln!(sst, "    nw_st_gradient_begin({}, {:.1}f);", v, deg);
                            for (p, c) in &stops {
                                let _ = writeln!(sst, "    nw_st_gradient_stop({}, {:.4}f, {:.4}f, {:.4}f, {:.4}f, {:.4}f);",
                                    v, p, c[0], c[1], c[2], c[3]);
                            }
                            let _ = writeln!(sst, "    nw_st_gradient_end({});", v);
                        } else if let Some(c) = css::gradient_solid(val) {
                            /* 无法解析的渐变 → 纯色近似兜底 */
                            let _ = writeln!(sst, "    nw_st_bg({}, {});", v, color_args(c));
                        }
                        if let Some(raw) = raw_of.get(k) {
                            if raw.contains("var(") && tmpl_theme_ok(raw, w.theme_names) {
                                let _ = writeln!(sst, "    nw_reg_var_decl({}, \"background-color\", \"{}\");", v, escape_c(raw));
                            }
                        }
                    }
                    "border-radius" => {
                        /* 多值（"0 12px 12px 0"）：D2D RoundedRect 单一半径，取 max 近似 */
                        let radius = val.split_whitespace()
                            .filter_map(|tok| css::parse_len(tok).map(|l| resolve_len(&l, inh2.font_size)))
                            .fold(None::<f32>, |acc, x| Some(acc.map_or(x, |m: f32| m.max(x))));
                        /* 兼容 "12px/20px" 椭圆写法与单值 */
                        let radius = radius.or_else(|| css::parse_len(val).map(|l| resolve_len(&l, inh2.font_size)));
                        if let Some(r) = radius {
                            let _ = writeln!(sst, "    nw_st_radius({}, {:.1}f);", v, r);
                        }
                    }
                    "color" => {
                        if let Some(c) = css::parse_color(val) {
                            if !g_on { inh2.color = c; } /* 守卫态不改继承链（同 font-size） */
                            let _ = writeln!(sst, "    nw_st_color({}, {});", v, color_args(c));
                        }
                        if let Some(raw) = raw_of.get(k) {
                            if raw.contains("var(") && tmpl_theme_ok(raw, w.theme_names) {
                                let _ = writeln!(sst, "    nw_reg_var_decl({}, \"color\", \"{}\");", v, escape_c(raw));
                                if !g_on { inh2.color_tmpl = Some((*raw).clone()); }
                            }
                        }
                    }
                    "position" => {
                        pos_kind = match val.trim() { "absolute" => 1, "fixed" => 2, _ => 0 };
                    }
                    "top" | "right" | "bottom" | "left" => {
                        let idx = match k.as_str() { "top" => 0, "right" => 1, "bottom" => 2, _ => 3 };
                        if val.trim() == "auto" {
                            inset_expr[idx] = None; inset_kind[idx] = 0;
                        } else if let Some((e, kd)) = inset_val_c(val, inh2.font_size) {
                            inset_expr[idx] = Some(e); inset_kind[idx] = kd;
                        }
                    }
                    "inset" => {
                        if let Some(b4) = css::expand_box4_sz(val) {
                            for idx in 0..4 {
                                let item = if let css::Sz::Len(Len::Pct(p)) = &b4[idx] {
                                    Some((format!("{:.2}f", p), 2))
                                } else {
                                    Some((css::sz_to_c(&b4[idx], inh2.font_size), 1))
                                };
                                if let Some((e, kd)) = item {
                                    inset_expr[idx] = Some(e);
                                    inset_kind[idx] = kd;
                                }
                            }
                        }
                    }
                    "z-index" => {
                        if let Ok(z) = val.trim().parse::<i32>() {
                            let _ = writeln!(sst, "    nw_st_z({}, {});", v, z);
                        }
                    }
                    "flex-grow" => {
                        if let Ok(g) = val.trim().parse::<f32>() {
                            let _ = writeln!(sst, "    nw_st_grow({}, {:.2});", v, g);
                        }
                    }
                    "border" | "border-top" | "border-right" | "border-bottom" | "border-left" => {
                        if let Some((bw, col)) = parse_border_shorthand(val) {
                            let sides: [(&str, usize); 4] =
                                [("top", 0), ("right", 1), ("bottom", 2), ("left", 3)];
                            let list: Vec<(&str, usize)> = if k == "border" {
                                sides.to_vec()
                            } else {
                                let si = match k.as_str() {
                                    "border-top" => 0, "border-right" => 1,
                                    "border-bottom" => 2, _ => 3,
                                };
                                vec![sides[si]]
                            };
                            for (sname, sidx) in list {
                                if let Some(c) = css::parse_color(&col) {
                                    let _ = writeln!(sst, "    nw_st_border_side({}, {}, {:.1}f, {});",
                                        v, sidx, bw, color_args(c));
                                } else if col.contains("var(") {
                                    /* var 颜色：编译期先解析近似（当前主题），并注册运行时重解析 */
                                    if let Some(rv) = substitute_vars(&col, &my_vars) {
                                        if let Some(c) = css::parse_color(&rv) {
                                            let _ = writeln!(sst, "    nw_st_border_side({}, {}, {:.1}f, {});",
                                                v, sidx, bw, color_args(c));
                                        }
                                    }
                                    if tmpl_theme_ok(&col, w.theme_names) {
                                        let _ = writeln!(sst, "    nw_reg_var_decl({}, \"border-{}-color\", \"{}\");",
                                            v, sname, escape_c(&col));
                                    }
                                }
                            }
                        }
                    }
                    "border-top-color" | "border-right-color" | "border-bottom-color" | "border-left-color" => {
                        /* 单边颜色声明：保留该边宽度，只覆盖颜色 */
                        let sidx = match k.as_str() {
                            "border-top-color" => 0, "border-right-color" => 1,
                            "border-bottom-color" => 2, _ => 3,
                        };
                        let sname = ["top", "right", "bottom", "left"][sidx];
                        if let Some(c) = css::parse_color(val) {
                            let _ = writeln!(sst, "    nw_st_border_color({}, {}, {});", v, sidx, color_args(c));
                        } else if val.contains("var(") {
                            if let Some(rv) = substitute_vars(val, &my_vars) {
                                if let Some(c) = css::parse_color(&rv) {
                                    let _ = writeln!(sst, "    nw_st_border_color({}, {}, {});", v, sidx, color_args(c));
                                }
                            }
                            if tmpl_theme_ok(val, w.theme_names) {
                                let _ = writeln!(sst, "    nw_reg_var_decl({}, \"border-{}-color\", \"{}\");",
                                    v, sname, escape_c(val));
                            }
                        }
                    }
                    "text-transform" => {
                        if !g_on && val.trim() == "uppercase" { inh2.upper = true; }
                    }
                    "box-shadow" => {
                        /* 外阴影："<ox> <oy> <blur> [spread] <color>"（inset 不支持则忽略） */
                        if let Some(sh) = parse_box_shadow(val) {
                            if let Some(c) = css::parse_color(&sh.color) {
                                let f: Vec<String> = sh.lens.iter()
                                    .map(|l| css::sz_to_c(&css::parse_sz(l).unwrap_or(css::Sz::Len(css::Len::Px(0.0))), inh2.font_size))
                                    .collect();
                                let _ = writeln!(sst,
                                    "    nw_st_shadow({}, {}, {}, {}, {}, {});",
                                    v, f[0], f[1],
                                    f.get(2).map(|s| s.as_str()).unwrap_or("0.0f"),
                                    f.get(3).map(|s| s.as_str()).unwrap_or("0.0f"),
                                    color_args(c));
                            }
                        }
                        if let Some(raw) = raw_of.get(k) {
                            if raw.contains("var(") && tmpl_theme_ok(raw, w.theme_names) {
                                let _ = writeln!(sst, "    nw_reg_var_decl({}, \"box-shadow\", \"{}\");",
                                    v, escape_c(raw));
                            }
                        }
                    }
                    "font-size" => {
                        if let Some(sz) = css::parse_sz(val) {
                            /* 浏览器语义：% / em 相对父字号；vw/vh 随窗口实算。
                             * 守卫态（g_on）：只发射运行时行（wrap_guards 包守卫），
                             * 不改继承链——守卫规则运行时可能不生效，编译期继承链
                             * 只近似静态级联；否则守卫声明污染后代继承值
                             * （Blog .nav a{font-size:250%} 曾令 nav div 把 250%
                             * 复合进继承链 → a 250%×250%=100px 巨字号） */
                            let expr = font_sz_c(&sz, &inh.font_expr);
                            if !g_on {
                                inh2.font_size = match &sz {
                                    css::Sz::Len(css::Len::Pct(p)) => inh.font_size * p / 100.0,
                                    _ => css::sz_const(&sz, inh.font_size),
                                };
                                inh2.font_expr = expr.clone();
                            }
                            let _ = writeln!(sst, "    nw_st_font({}, {}, 0, 0);", v, expr);
                        }
                    }
                    "font-weight" => {
                        let wgt = parse_weight(val);
                        if !g_on { inh2.weight = wgt; }
                        let _ = writeln!(sst, "    nw_st_font({}, 0, {}, 0);", v, wgt);
                    }
                    "font-family" => {
                        let fam_v = parse_family(val);
                        if !g_on { inh2.family = fam_v.clone(); }
                        let fam = match &fam_v { Some(f) => wide_lit(f), None => "0".to_string() };
                        let _ = writeln!(sst, "    nw_st_font({}, 0, 0, {});", v, fam);
                    }
                    "text-align" => {
                        let al = parse_align(val);
                        if !g_on { inh2.align = al; }
                        let _ = writeln!(sst, "    nw_st_text_align({}, {});", v, al);
                    }
                    "transform" => {
                        /* translate(tx[,ty]) / translateX / translateY：绘制偏移（不影响布局，
                           浏览器 transform 语义），% 相对自身盒尺寸运行时实算 */
                        if let Some((dx, kx, dy, ky)) = parse_translate(val) {
                            let _ = writeln!(sst, "    nw_st_translate({}, {}, {}, {}, {});", v, dx, kx, dy, ky);
                        }
                    }
                    "text-shadow" => {
                        /* "<dx> <dy> [<blur>] <color>"：blur 忽略（M 近似） */
                        if let Some(sh) = parse_text_shadow(val) {
                            let _ = writeln!(sst, "    nw_st_tshadow({}, {}, {}, {}, {}, {});",
                                v, sh.0, sh.1, sh.2, sh.3, color_args(sh.4));
                        }
                    }
                    "letter-spacing" => {
                        if let Some((lv, lu)) = parse_len_unit(val) {
                            let _ = writeln!(sst, "    nw_st_lspace({}, {:.4}, {});", v, lv, lu);
                        }
                    }
                    "line-height" => {
                        /* unitless 数字 = 字号倍数；% 同倍数；px/em 固定值 */
                        let t = val.trim();
                        if let Ok(m) = t.parse::<f32>() {
                            let _ = writeln!(sst, "    nw_st_lineh({}, {:.4}, 0);", v, m);
                        } else if let Some(p) = t.strip_suffix('%').and_then(|s| s.parse::<f32>().ok()) {
                            let _ = writeln!(sst, "    nw_st_lineh({}, {:.4}, 0);", v, p / 100.0);
                        } else if let Some((lv, lu)) = parse_len_unit(t) {
                            let _ = writeln!(sst, "    nw_st_lineh({}, {:.4}, {});", v, lv, lu);
                        }
                    }
                    // M1 未支持属性：静默忽略（保持前向兼容）
                    _ => {}
                }
                if g_on { sst.push_str("/*@GE*/\n"); }
            }
            /* 背景图（级联序：background-size 无论先后都作用于 background-image） */
            if let Some(p) = bg_url {
                let _ = writeln!(sst, "    nw_st_bgimg({}, L\"{}\", {});", v, escape_c(&p), bg_fit);
            }
            /* <img> 宽高属性 → 已前移到守卫循环**之前**发射（表现属性 = 特异性 0
             * 的作者声明，命中规则运行时覆盖）。是否发射只看静态级联 sdecls：
             * resolved 含全部动态守卫规则（.background{width:100%} 对每元素都
             * 返回），会全局抑制属性尺寸 → Blog 水印回落自然尺寸铺满主页（#49） */
            /* grid 等宽子项：子元素未显式声明 width 时注入等宽百分比 */
            if let Some(pct) = grid_child_pct {
                if !sdecls.iter().any(|(k, _)| k == "width") {
                    let _ = writeln!(sst, "    nw_st_width_pct({}, {:.2});", v, pct);
                }
                /* 关键：清 min-width（taffy 默认 min-content），列 gap 超出宽度才能被
                   flex shrink 均摊——否则长文本子项撑爆行宽（浏览器 grid 无此问题） */
                let _ = writeln!(sst, "    nw_st_min_w({}, 0.0f);", v);
            }
            /* 表格列宽传播（浏览器表格布局语义）：首行单元格的显式宽度 = 整列宽度，
               共享到所有行的同列单元格——每行独立 flex 时列起点才能对齐 */
            if el.tag == "table" && inh2.tbl_colw.is_none() {
                if let Some(Child::El(first_tr)) =
                    el.children.iter().find(|c| matches!(c, Child::El(e) if e.tag == "tr"))
                {
                    let tr_ref = ElemRef {
                        tag: &first_tr.tag, id: &first_tr.id,
                        classes: &first_tr.classes, attrs: &first_tr.attrs,
                    };
                    let mut tr_chain: Vec<ElemRef> = chain.to_vec();
                    tr_chain.push(el_ref.clone());
                    tr_chain.push(tr_ref);
                    let mut ws: Vec<Option<String>> = Vec::new();
                    for c in &first_tr.children {
                        if let Child::El(cell) = c {
                            let cref = ElemRef {
                                tag: &cell.tag, id: &cell.id,
                                classes: &cell.classes, attrs: &cell.attrs,
                            };
                            let d = cascade(&cref, &tr_chain, cell.style_attr.as_deref(), w.rules);
                            let wd = d.iter().rev().find(|(k, _)| k == "width")
                                .map(|(_, val)| val.trim().to_string())
                                .filter(|val| val.ends_with('%'));
                            ws.push(wd);
                        }
                    }
                    inh2.tbl_colw = Some(std::rc::Rc::new(ws));
                }
            }
            /* 定位（position: absolute/fixed + inset/top/right/bottom/left） */
            if pos_kind != 0 {
                let g = |o: &Option<String>| o.clone().unwrap_or_else(|| "0.0f".into());
                let _ = writeln!(sst,
                    "    nw_st_position({}, {}, {}, {}, {}, {}, {}, {}, {}, {});",
                    v, pos_kind,
                    g(&inset_expr[0]), inset_kind[0], g(&inset_expr[1]), inset_kind[1],
                    g(&inset_expr[2]), inset_kind[2], g(&inset_expr[3]), inset_kind[3]);
            }
            /* 表格单元格：min_width=0（对齐不被长文本 min-content 撑爆）；
               显式宽度时不等分增长（其余列分摊剩余空间）；
               无显式宽度时继承首行列宽（浏览器表格布局：列宽全表共享） */
            if el.tag == "td" || el.tag == "th" {
                let _ = writeln!(sst, "    nw_st_min_w({}, 0.0f);", v);
                /* 只看静态级联（resolved 含未命中动态规则的守卫声明，会误判） */
                if sdecls.iter().any(|(k, _)| k == "width") {
                    let _ = writeln!(sst, "    nw_st_grow({}, 0.0);", v);
                } else if let Some(cw) = &inh.cell_w {
                    if let Some(p) = cw.trim().strip_suffix('%') {
                        if let Ok(pct) = p.trim().parse::<f32>() {
                            let _ = writeln!(sst, "    nw_st_width_pct({}, {:.2});", v, pct);
                            let _ = writeln!(sst, "    nw_st_grow({}, 0.0);", v);
                        }
                    }
                }
            }
            /* display 动态规则注册（一次，进创建代码；媒体条件随规则携带，
               运行时 reapply_display 按当前视口 + 真实类名评估） */
            if w.do_cr {
                for (vk2, seg) in w.seg_bounds.iter().enumerate() {
                    /* 基线规则（spec -1）：该段级联的静态 display 值 */
                    let dcl = cascade(&el_ref, chain, el.style_attr.as_deref(), &w.rules_v[vk2]);
                    let disp = match dcl.iter().rev().find(|(k, _)| k == "display") {
                        Some((_, dv)) => !matches!(dv.trim(), "none"),
                        None => true,
                    };
                    let _ = writeln!(w.cr,
                        "    nw_reg_display_rule({}, \"\", -1, {}, {}, {}, {}, {}, {});",
                        v, disp as i32, fb(seg.0), fb(seg.1), fb(seg.2), fb(seg.3), seg.4);
                }
                for rule in w.all_rules {
                    let Some((_, dv)) = rule.decls.iter().find(|(k, _)| k == "display") else { continue };
                    let dflex = !matches!(dv.trim(), "none") as i32;
                    let (mnw, mxw, mnh, mxh, morb) = media_bounds(&rule.media);
                    for sel in &rule.sels {
                        let Some(subj) = sel.parts.last() else { continue };
                        if subj.classes.is_empty() { continue; }
                        let spec = match match_selector(sel, &el_ref, chain) {
                            Some(s) => s,
                            None => {
                                /* 动态类容错：JS classList 运行时可能补齐缺失类。
                                 * 主选择器类与元素有交集即注册（如 .slide.active 注册到
                                 * 静态只有 .slide 的元素），运行时 reapply 按真实类名评估 */
                                match match_selector_display(sel, &el_ref, chain) {
                                    Some(s) => s,
                                    None => continue,
                                }
                            }
                        };
                        let cls_s = subj.classes.join(" ");
                        let _ = writeln!(w.cr,
                            "    nw_reg_display_rule({}, \"{}\", {}, {}, {}, {}, {}, {}, {});",
                            v, escape_c(&cls_s), spec, dflex, fb(mnw), fb(mxw), fb(mnh), fb(mxh), morb);
                    }
                }
            }
            /* grid 子集方向与等宽策略（声明收集完毕后统一决定；行分组结构由初始段决定） */
            if is_grid {
                match grid_cols {
                    Some(n) => {
                        let _ = writeln!(sst, "    nw_st_flex_dir({}, 1);", v);
                        let _ = writeln!(sst, "    nw_st_grid_gap({}, 0.0f, {});", v, grid_gap_c);
                        let pct = 100.0 / n as f32;
                        my_grid_child_pct = Some(pct);
                    }
                    None => {
                        /* 无模板 → 单列堆叠 */
                        let _ = writeln!(sst, "    nw_st_flex_dir({}, 1);", v);
                    }
                }
            }
            if let Some(m) = margin {
                let _ = writeln!(sst, "    nw_st_margin({}, {}, {}, {}, {});", v, m[0], m[1], m[2], m[3]);
                /* 垂直 auto 在块盒流 = 0（已发 0 值），只保留水平位 */
                let mask: u32 = margin_auto.iter().enumerate()
                    .fold(0u32, |acc, (i, &a)| if a { acc | (1u32 << i) } else { acc })
                    & 0b1010;
                if mask != 0 {
                    let _ = writeln!(sst, "    nw_st_margin_auto({}, {});", v, mask);
                    /* 浏览器块盒 vs flexbox 语义差：交叉轴 auto margin 会取消 stretch
                     *（taffy 按 flex 规范 fit-content 居中），块盒是「撑满可用宽
                     *（max-width 钳制）再居中」。块级标签且无静态 width 时补
                     * width:100%（taffy 百分比按边框盒解析，padding 不溢出）；
                     * 行内类（button/img 等收缩盒）保持 fit-content 居中不动 */
                    let block_tag = matches!(el.tag.as_str(),
                        "div" | "main" | "header" | "footer" | "section" | "article"
                        | "aside" | "nav" | "p" | "h1" | "h2" | "h3" | "h4" | "h5"
                        | "h6" | "ul" | "ol" | "li" | "form" | "table" | "figure"
                        | "figcaption" | "blockquote");
                    if block_tag && !sdecls.iter().any(|(k, _)| k == "width") {
                        let _ = writeln!(sst, "    nw_st_width_pct({}, 100.0f);", v);
                    }
                }
            }
            if has_padding {
                let p = padding.unwrap_or(["0.0f".into(), "0.0f".into(), "0.0f".into(), "0.0f".into()]);
                let _ = writeln!(sst, "    nw_st_padding({}, {}, {}, {}, {});", v, p[0], p[1], p[2], p[3]);
            }
            /* 本节点样式重算函数（先于子节点 → 表序 = 创建序） */
            let sst = wrap_guards(sst, &resolved_g);
            w.emit_style_fn(ord, &v, &sst);

            if el.tag == "textarea" {
                /* M2：textarea 的直接文本子节点 = 初始值（不生成子节点） */
                let mut init = String::new();
                for c in &el.children {
                    if let Child::Text(t) = c { init.push_str(t); }
                }
                if !init.is_empty() {
                    w.create(format!("nw_ta_set_text({}, {});", v, wide_lit(init.trim_start())));
                }
                return Ok(());
            }
            let mut ch2: Vec<ElemRef> = chain.to_vec();
            ch2.push(el_ref);
            /* 行内混排（浏览器 line box 近似）：容器内连续 inline 子节点
               包进匿名行盒（flex row + wrap + baseline 对齐），实现同行排列。
               单成员组走原路径（纯文本折行零回归）。
               通用化（J13）：不再限于 p/li/h 等文本标签——任何非 flex/grid 容器
               （nav/div/footer…）的 inline 子元素同样行盒化（浏览器块容器的
               匿名行盒语义）；linebox/gridrow 自身已是 flex row，排除。
               浮动子项所在容器例外：float 分组在 else 分支，行盒分支会把
               浮动子项当普通块直发（Blog nav 竖排回归）——浏览器里浮动盒
               脱离行内流，含浮动子项的容器整体交给 float 分组路径。 */
            let child_floated = |c: &Child| -> bool {
                match c {
                    Child::El(e) => {
                        let cref = ElemRef { tag: &e.tag, id: &e.id, classes: &e.classes, attrs: &e.attrs };
                        /* rev：取级联序最后的 float 声明（与 else 分组判定一致） */
                        cascade(&cref, &ch2, e.style_attr.as_deref(), w.rules).iter().rev()
                            .find(|(k, _)| k == "float")
                            .map(|(_, v)| matches!(v.trim(), "left" | "right"))
                            .unwrap_or(false)
                    }
                    _ => false,
                }
            };
            let has_floated = el.children.iter().any(&child_floated);
            let linebox_ok = !disp_is_flexgrid_s && el.tag != "linebox" && el.tag != "gridrow" && !has_floated;
            if linebox_ok {
                /* 行内级判定：inline 标签白名单 + 作者 CSS 未显式块级化
                   （display:block/flex/grid/none 的 a/span 等仍是块级盒，浏览器语义） */
                fn child_is_inline(c: &Child, chain: &[ElemRef], srules: &[Rule]) -> bool {
                    let e = match c {
                        Child::Text(_) => return true,
                        Child::El(e) => e,
                    };
                    if !matches!(e.tag.as_str(),
                        "b" | "strong" | "em" | "i" | "u" | "s" | "span" | "a"
                        | "code" | "kbd" | "small" | "sup" | "sub" | "label" | "br"
                        /* inline-block 替换/控件类：button/img 浏览器默认同行排列 */
                        | "button" | "img")
                    {
                        return false;
                    }
                    let cref = ElemRef { tag: &e.tag, id: &e.id, classes: &e.classes, attrs: &e.attrs };
                    let mut disp: Option<String> = None;
                    let mut floated = false;
                    /* rev：同属性后者覆盖（级联序） */
                    for (k, v) in cascade(&cref, chain, e.style_attr.as_deref(), srules).iter().rev() {
                        if k == "display" && disp.is_none() { disp = Some(v.clone()); }
                        else if k == "float" && !floated {
                            floated = matches!(v.trim(), "left" | "right");
                        }
                    }
                    /* 浮动盒脱离行内流（浏览器语义）——交给 float 分组路径 */
                    if floated { return false; }
                    !matches!(disp.as_deref(),
                        Some("block" | "flex" | "grid" | "none" | "table" | "table-row"
                            | "table-cell" | "list-item"))
                }
                let n_ch = el.children.len();
                let mut idx = 0;
                while idx < n_ch {
                    if !child_is_inline(&el.children[idx], &ch2, w.srules) {
                        emit_child(w, &v, &el.children[idx], &inh2, _ids, &ch2, &my_vars, my_grid_child_pct, false, handlers, sm)?;
                        idx += 1;
                        continue;
                    }
                    let mut j = idx;
                    while j < n_ch && child_is_inline(&el.children[j], &ch2, w.srules) { j += 1; }
                    if j - idx == 1 {
                        emit_child(w, &v, &el.children[idx], &inh2, _ids, &ch2, &my_vars, my_grid_child_pct, false, handlers, sm)?;
                    } else {
                        w.counter += 1;
                        let lord = w.counter;
                        let lv = format!("{}nwLH{}", w.pfx, lord);
                        w.create(format!("{} = nw_dom_create(\"linebox\", 0, 0);", lv));
                        w.create(format!("nw_dom_append({}, {});", v, lv));
                        w.globals.push(lv.clone());
                        let mut lsst = String::new();
                        let _ = writeln!(lsst, "    nw_st_display({}, 1);", lv);
                        let _ = writeln!(lsst, "    nw_st_flex_dir({}, 0);", lv);
                        let _ = writeln!(lsst, "    nw_st_flex_wrap({}, 1);", lv);
                        let _ = writeln!(lsst, "    nw_st_align({}, 4); /* baseline */", lv);
                        let _ = writeln!(lsst, "    nw_st_width_pct({}, 100.0);", lv);
                        /* li 标记：混排时标记需在最前，作为独立文本节点前置 */
                        let is_li = ch2.len() >= 2 && ch2[ch2.len() - 2].tag == "ul" && ch2[ch2.len() - 1].tag == "li";
                        let mut mord = 0usize;
                        let mut mv = String::new();
                        let mut msst = String::new();
                        if is_li {
                            let ul = &ch2[ch2.len() - 2];
                            let has = |c: &str| ul.classes.iter().any(|x| x == c);
                            let m = if has("arrow") { "▸ " }
                                else if has("check") { "✓ " }
                                else if has("no") { "✕ " }
                                else { "• " };
                            w.counter += 1;
                            mord = w.counter;
                            mv = format!("nwT{}", mord);
                            w.create(format!("{} = nw_dom_text({});", mv, wide_lit(m)));
                            w.create(format!("nw_dom_append({}, {});", lv, mv));
                            w.globals.push(mv.clone());
                            let _ = writeln!(msst, "    nw_st_color({}, {});", mv, color_args(inh2.color));
                            if let Some(tmpl) = &inh2.color_tmpl {
                                if tmpl.contains("var(") && tmpl_theme_ok(tmpl, w.theme_names) {
                                    let _ = writeln!(msst, "    nw_reg_var_decl({}, \"color\", \"{}\");", mv, escape_c(tmpl));
                                }
                            }
                            let fam2 = match &inh2.family { Some(f) => wide_lit(f), None => "0".to_string() };
                            let _ = writeln!(msst, "    nw_st_font({}, {:.1}f, {}, {});", mv, inh2.font_size, inh2.weight, fam2);
                        }
                        /* 行盒先于其子标记发射（表序 = 创建序；单节点重算按下标分发） */
                        w.emit_style_fn(lord, &lv, &lsst);
                        if is_li {
                            w.emit_style_fn(mord, &mv, &msst);
                        }
                        let lb_ref = ElemRef { tag: "linebox", id: &None, classes: &[], attrs: &[] };
                        let mut lb_chain = ch2.clone();
                        lb_chain.push(lb_ref);
                        for cc in &el.children[idx..j] {
                            emit_child(w, &lv, cc, &inh2, _ids, &lb_chain, &my_vars, None, true, handlers, sm)?;
                        }
                    }
                    idx = j;
                }
            } else if is_grid_s && ncols_s.is_some() {
                /* grid 行分组：每 ncols 个元素子项包进匿名行组（flex row nowrap）
                   （行分组结构恒用初始段列数；子项宽度百分比按本段级联） */
                let ncols = ncols_s.unwrap();
                let mut row_var: Option<String> = None;
                let mut row_ord = 0usize;
                let mut in_row = 0usize;
                for c in &el.children {
                    let is_el = matches!(c, Child::El(_));
                    if !is_el {
                        emit_child(w, &v, c, &inh2, _ids, &ch2, &my_vars, None, false, handlers, sm)?;
                        continue;
                    }
                    if in_row == 0 {
                        w.counter += 1;
                        row_ord = w.counter;
                        let rv = format!("{}nwGR{}", w.pfx, row_ord);
                        w.create(format!("{} = nw_dom_create(\"gridrow\", 0, 0);", rv));
                        w.create(format!("nw_dom_append({}, {});", v, rv));
                        w.globals.push(rv.clone());
                        let mut rsst = String::new();
                        let _ = writeln!(rsst, "    nw_st_display({}, 1);", rv);
                        let _ = writeln!(rsst, "    nw_st_flex_dir({}, 0);", rv);
                        let _ = writeln!(rsst, "    nw_st_flex_wrap({}, 0);", rv);
                        let _ = writeln!(rsst, "    nw_st_width_pct({}, 100.0);", rv);
                        let _ = writeln!(rsst, "    nw_st_grid_gap({}, {}, 0.0f);", rv, grid_gap_c);
                        row_var = Some(rv);
                        /* 行组样式函数在子项发射后补发（保持表序 = 创建序）：
                           先占位再填充不可行，这里先记序号，子项递归前不能发——
                           但子项序号必须大于行组序号，故行组函数立即发射，
                           grid_gap 用本段值已在 rsst 中，无需子项信息 */
                        w.emit_style_fn(row_ord, row_var.as_ref().unwrap(), &rsst);
                    }
                    let rv = row_var.as_ref().unwrap().clone();
                    emit_child(w, &rv, c, &inh2, _ids, &ch2, &my_vars, my_grid_child_pct, false, handlers, sm)?;
                    in_row += 1;
                    if in_row == ncols {
                        row_var = None;
                        in_row = 0;
                    }
                }
            } else if el.tag == "tr" && inh2.tbl_colw.is_some() {
                /* 表格行：按列序注入首行列宽 */
                let cols = inh2.tbl_colw.clone().unwrap();
                let mut ei = 0usize;
                for c in &el.children {
                    let mut ci = inh2.clone();
                    if matches!(c, Child::El(_)) {
                        ci.cell_w = cols.get(ei).cloned().flatten();
                        ei += 1;
                    }
                    emit_child(w, &v, c, &ci, _ids, &ch2, &my_vars, my_grid_child_pct, false, handlers, sm)?;
                }
            } else {
                /* float 分组（浏览器浮动语义近似）：块流容器内连续同向浮动子项包进
                   匿名行盒——right = row-reverse（DOM 序自右缘向左堆叠），left = row。
                   文本子项打断浮动行（M 近似：不实现环绕）。 */
                let n_ch = el.children.len();
                let mut idx = 0usize;
                while idx < n_ch {
                    let fl = match &el.children[idx] {
                        Child::El(e) => {
                            let cref = ElemRef { tag: &e.tag, id: &e.id, classes: &e.classes, attrs: &e.attrs };
                            cascade(&cref, &ch2, e.style_attr.as_deref(), w.rules).iter().rev()
                                .find(|(k, _)| k == "float").and_then(|(_, v)| match v.trim() {
                                    "right" => Some(2i32), "left" => Some(1i32), _ => None,
                                })
                        }
                        _ => None,
                    };
                    if fl.is_none() {
                        emit_child(w, &v, &el.children[idx], &inh2, _ids, &ch2, &my_vars, my_grid_child_pct, false, handlers, sm)?;
                        idx += 1;
                        continue;
                    }
                    let dir = fl.unwrap();
                    let mut j = idx;
                    while j < n_ch {
                        match &el.children[j] {
                            Child::El(e) => {
                                let cref = ElemRef { tag: &e.tag, id: &e.id, classes: &e.classes, attrs: &e.attrs };
                                let f2 = cascade(&cref, &ch2, e.style_attr.as_deref(), w.rules).iter().rev()
                                    .find(|(k, _)| k == "float").and_then(|(_, v)| match v.trim() {
                                        "right" => Some(2i32), "left" => Some(1i32), _ => None,
                                    });
                                if f2 != Some(dir) { break; }
                            }
                            _ => break,
                        }
                        j += 1;
                    }
                    w.counter += 1;
                    let ford = w.counter;
                    let fv = format!("{}nwFR{}", w.pfx, ford);
                    w.create(format!("{} = nw_dom_create(\"floatrow\", 0, 0);", fv));
                    w.create(format!("nw_dom_append({}, {});", v, fv));
                    w.globals.push(fv.clone());
                    let mut fsst = String::new();
                    let _ = writeln!(fsst, "    nw_st_display({}, 1);", fv);
                    /* row(1)/row-reverse(2)；右浮动从右缘起排，左浮动从左缘起排 */
                    let _ = writeln!(fsst, "    nw_st_flex_dir({}, {});", fv, if dir == 2 { 2 } else { 0 });
                    let _ = writeln!(fsst, "    nw_st_width_pct({}, 100.0); /* 行盒占满，浮动才贴父缘 */", fv);
                    let _ = writeln!(fsst, "    nw_st_align({}, 3); /* stretch */", fv);
                    w.emit_style_fn(ford, &fv, &fsst);
                    for c in &el.children[idx..j] {
                        emit_child(w, &fv, c, &inh2, _ids, &ch2, &my_vars, my_grid_child_pct, false, handlers, sm)?;
                    }
                    idx = j;
                }
            }
            Ok(())
        }
    }
}

fn emit_size(out: &mut String, var: &str, val: &str, font: f32, width: bool) {    let setter_px = if width { "nw_st_width_px" } else { "nw_st_height_px" };
    let setter_pct = if width { "nw_st_width_pct" } else { "nw_st_height_pct" };
    match css::parse_sz(val) {
        Some(css::Sz::Len(Len::Px(v))) => { let _ = writeln!(out, "    {}({}, {:.1}f);", setter_px, var, v); }
        Some(css::Sz::Len(Len::Pct(v))) => { let _ = writeln!(out, "    {}({}, {:.1}f);", setter_pct, var, v); }
        Some(other) => { let _ = writeln!(out, "    {}({}, {});", setter_px, var, css::sz_to_c(&other, font)); }
        None => {}
    }
}

fn emit_max_w(out: &mut String, var: &str, val: &str, font: f32) {
    match css::parse_sz(val) {
        Some(css::Sz::Len(Len::Px(v))) => { let _ = writeln!(out, "    nw_st_max_w_px({}, {:.1}f);", var, v); }
        Some(css::Sz::Len(Len::Pct(v))) => { let _ = writeln!(out, "    nw_st_max_w_pct({}, {:.1}f);", var, v); }
        Some(other) => { let _ = writeln!(out, "    nw_st_max_w_px({}, {});", var, css::sz_to_c(&other, font)); }
        None => {}
    }
}

/// CSS url(...) 提取：url("path") / url('path') / url(path) → 归一化项目相对路径
/// （CSS 文件内的相对路径已在 main 预处理阶段按所在目录改写为项目相对）
fn css_url(val: &str) -> Option<String> {
    let i = val.find("url(")? + 4;
    let rest = &val[i..];
    let end = rest.rfind(')')?;
    let inner = rest[..end].trim();
    let inner = inner.strip_prefix('"').and_then(|s| s.strip_suffix('"'))
        .or_else(|| inner.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
        .unwrap_or(inner);
    if inner.is_empty() { return None; }
    Some(norm_rel(inner))
}

/// background-size → 绘制模式：0=fill 1=cover 2=contain 3=固有尺寸
fn parse_bg_fit(val: &str) -> i32 {
    match val.trim() {
        "cover" => 1,
        "contain" => 2,
        "100% 100%" => 0,
        _ => 3,
    }
}

/// 相对路径归一：解析 ./ 与 ../ 段，统一正斜杠（不触盘，纯字符串运算）
pub fn norm_rel(p: &str) -> String {
    let p = p.replace('\\', "/");
    let mut out: Vec<&str> = Vec::new();
    for seg in p.split('/') {
        match seg {
            "." | "" => {}
            ".." => { out.pop(); }
            s => out.push(s),
        }
    }
    out.join("/")
}

/// font-size 的运行时 C 表达式：% / em 相对父字号运行时值（font_expr 链式传播），
/// vw/vh 随窗口实算（浏览器动态视口单位语义）
fn font_len_c(l: &Len, parent: &str) -> String {
    match l {
        Len::Px(v) => format!("{:.2}f", v),
        Len::Pct(p) => format!("({:.2}f * {} / 100.0f)", p, parent),
        Len::Em(e) => format!("({:.2}f * {})", e, parent),
        Len::Vw(v) => format!("nw_vw({:.2})", v),
        Len::Vh(v) => format!("nw_vh({:.2})", v),
    }
}

fn font_sz_c(sz: &css::Sz, parent: &str) -> String {
    match sz {
        css::Sz::Len(l) => font_len_c(l, parent),
        css::Sz::Clamp(a, b, c) => format!("fminf(fmaxf({}, {}), {})", font_sz_c(a, parent), font_sz_c(b, parent), font_sz_c(c, parent)),
        css::Sz::Min(a, b) => format!("fminf({}, {})", font_sz_c(a, parent), font_sz_c(b, parent)),
        css::Sz::Max(a, b) => format!("fmaxf({}, {})", font_sz_c(a, parent), font_sz_c(b, parent)),
    }
}

/// 长度 → (值, 单位)：0=px 1=vw 2=vh 3=em（运行时按窗口/字号实算）
fn parse_len_unit(s: &str) -> Option<(f32, i32)> {
    let t = s.trim();
    if let Some(v) = t.strip_suffix("vw") { return v.trim().parse().ok().map(|x| (x, 1)); }
    if let Some(v) = t.strip_suffix("vh") { return v.trim().parse().ok().map(|x| (x, 2)); }
    if let Some(v) = t.strip_suffix("em") { return v.trim().parse().ok().map(|x| (x, 3)); }
    if let Some(v) = t.strip_suffix("px") { return v.trim().parse().ok().map(|x| (x, 0)); }
    t.parse().ok().map(|x| (x, 0)) // 无单位按 px
}

/// transform: translate(x[,y]) / translateX / translateY → (dx, kx, dy, ky)
/// kind：0=无 1=px/vw/vh/em（值随单位运行时实算，编码见 parse_len_unit 语义+10 偏移）
/// 简化：px → kind1 存 px 值；vw/vh/em 在编译期已知窗口则烘焙……否——统一编码：
/// kind 1=px 2=%（相对自身盒）11=vw 12=vh 13=em（相对本节点字号）
fn parse_translate(val: &str) -> Option<(f32, i32, f32, i32)> {
    let t = val.trim();
    let inner = t.strip_prefix("translate(").and_then(|s| s.strip_suffix(')'))
        .or_else(|| t.strip_prefix("translatex(").and_then(|s| s.strip_suffix(')')))
        .or_else(|| t.strip_prefix("translateX(").and_then(|s| s.strip_suffix(')')))
        .or_else(|| t.strip_prefix("translatey(").and_then(|s| s.strip_suffix(')')))
        .or_else(|| t.strip_prefix("translateY(").and_then(|s| s.strip_suffix(')')))?;
    let low = t.to_ascii_lowercase();
    let single = low.starts_with("translatex(") || low.starts_with("translatey(");
    let parts: Vec<&str> = inner.split(',').map(|s| s.trim()).collect();
    if !single && parts.len() > 2 { return None; }
    let one = |tok: &str| -> Option<(f32, i32)> {
        let tt = tok.trim();
        if tt == "0" { return Some((0.0, 1)); }
        if let Some(p) = tt.strip_suffix('%') { return p.trim().parse().ok().map(|x| (x, 2)); }
        parse_len_unit(tt).map(|(v, u)| (v, match u { 0 => 1, 1 => 11, 2 => 12, _ => 13 }))
    };
    let (dx, kx) = one(parts[0])?;
    let (dy, ky) = if single || parts.len() == 1 { (0.0, 1) } else { one(parts[1])? };
    Some((dx, kx, dy, ky))
}

/// text-shadow "<dx> <dy> [<blur>] <color>" → (dx, ux, dy, uy, [r,g,b,a])
/// 单位编码同 parse_len_unit（0=px 1=vw 2=vh 3=em）
fn parse_text_shadow(val: &str) -> Option<(f32, i32, f32, i32, [f32; 4])> {
    let mut parts: Vec<String> = Vec::new();
    let mut depth = 0i32;
    let mut cur = String::new();
    for ch in val.chars() {
        match ch {
            '(' => { depth += 1; cur.push(ch); }
            ')' => { depth -= 1; cur.push(ch); }
            c if c.is_whitespace() && depth == 0 => {
                if !cur.is_empty() { parts.push(std::mem::take(&mut cur)); }
            }
            c => cur.push(ch),
        }
    }
    if !cur.trim().is_empty() { parts.push(cur); }
    let ci = parts.iter().rposition(|p| p.contains("var(") || css::parse_color(p).is_some())?;
    let c = css::parse_color(&parts[ci])?;
    if ci < 2 || ci > 3 { return None; }
    let (dx, ux) = parse_len_unit(&parts[0])?;
    let (dy, uy) = parse_len_unit(&parts[1])?;
    Some((dx, ux, dy, uy, c))
}

fn resolve_len(l: &Len, font: f32) -> f32 {
    match l {
        Len::Px(v) => *v,
        Len::Pct(v) => *v, // margin/padding 的百分比 M1 按视口处理（简单化）
        Len::Em(v) => v * font,
        Len::Vw(v) => v * 9.0, /* 编译期近似：默认窗口 900x640 */
        Len::Vh(v) => v * 6.4,
    }
}

fn parse_weight(s: &str) -> i32 {
    match s.trim() {
        "normal" => 400,
        "bold" => 700,
        "lighter" | "light" => 300,
        "bolder" => 900,
        n => n.trim().parse().unwrap_or(400),
    }
}

fn parse_align(s: &str) -> i32 {
    match s.trim() {
        "center" => 1,
        "right" | "end" => 2,
        _ => 0,
    }
}

fn parse_family(s: &str) -> Option<String> {
    let first = s.split(',').next()?.trim();
    let name = first.trim_matches(|c| c == '"' || c == '\'');
    if name.is_empty() { None } else { Some(name.to_string()) }
}

/// var() 编译期替换：var(--x) / var(--x, fallback)；无法解析且无 fallback → None（声明无效）
fn substitute_vars(val: &str, vars: &HashMap<String, String>) -> Option<String> {
    if !val.contains("var(") { return Some(val.to_string()); }
    let mut out = String::new();
    let mut rest = val;
    while let Some(p) = rest.find("var(") {
        out.push_str(&rest[..p]);
        let tail = &rest[p + 4..];
        /* 找配对右括号（fallback 里可能嵌套 var()） */
        let mut depth = 1usize;
        let mut end = None;
        for (k, ch) in tail.char_indices() {
            match ch {
                '(' => depth += 1,
                ')' => { depth -= 1; if depth == 0 { end = Some(k); break; } }
                _ => {}
            }
        }
        let end = end?;
        let inner = &tail[..end];
        rest = &tail[end + 1..];
        /* 第一个顶层逗号分隔名字与 fallback */
        let (name, fallback) = match inner.find(',') {
            Some(c) => (inner[..c].trim(), Some(inner[c + 1..].trim())),
            None => (inner.trim(), None),
        };
        if let Some(v) = vars.get(name) {
            out.push_str(v);
        } else if let Some(fb) = fallback {
            out.push_str(&substitute_vars(fb, vars)?);
        } else {
            return None;
        }
    }
    out.push_str(rest);
    Some(out)
}

fn escape_c(s: &str) -> String {
    let mut o = String::new();
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            c if (c as u32) < 0x20 => { let _ = write!(o, "\\{:03o}", c as u32); }
            c => o.push(c),
        }
    }
    o
}

/// 非 ASCII / 引号字符用独立 L"\xXXXX" 字面量块，避免十六进制贪吃
fn wide_lit(s: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut cur = String::from("L\"");
    for c in s.chars() {
        let u = c as u32;
        if u < 128 && c != '"' && c != '\\' {
            cur.push(c);
        } else {
            cur.push('"');
            parts.push(std::mem::take(&mut cur));
            parts.push(format!("L\"\\x{:04X}\"", u));
            cur = String::from("L\"");
        }
    }
    cur.push('"');
    parts.push(cur);
    parts.join("")
}

fn sanitize(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).collect()
}
