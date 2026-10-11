// Nowebiew M0 - C 代码生成
// 类型映射：number -> double，boolean -> int，string -> NWStr（值语义 struct）。
// 生成可读的 C 代码，中间 .c 文件保留供审查。
// M0 内存模型：字符串拼接分配后不回收（原型阶段），M1 换引用计数/arena。

use std::collections::HashMap;
use std::fmt::Write as _;
use crate::ast::*;
use crate::check::Checked;

#[derive(Debug, Clone)]
pub struct CodegenError {
    pub msg: String,
}

pub fn generate(prog: &Program, checked: &Checked, is_ui: bool, ui_prefix: &str,
                n_variants: usize, initial_variant: usize,
                pfx: &str, emit_main: bool) -> Result<String, CodegenError> {
    let mut g = Gen {
        out: String::new(),
        scopes: vec![HashMap::new()],
        err: None,
        top_vars: checked.top_vars.clone(),
        onclick_fns: checked.onclick_fns.clone(),
        key_fns: checked.key_fns.clone(),
        doc_click_fns: checked.doc_click_fns.clone(),
        key_event_fns: checked.key_event_fns.clone(),
        touch_start_fns: checked.touch_start_fns.clone(),
        touch_end_fns: checked.touch_end_fns.clone(),
        el_click_evt_fns: checked.el_click_evt_fns.clone(),
        timer_fns: checked.timer_fns.clone(),
        adapter_map: HashMap::new(),
        call_keys: checked.call_keys.clone(),
        inst_params: checked.instances.iter()
            .map(|i| ((i.decl_name.clone(), i.key.clone()),
                      i.params.iter().map(|(n, t)| (n.clone(), *t)).collect()))
            .collect(),
        local_types: checked.local_types.clone(),
        cur_fn: None,
        is_ui,
        pfx: pfx.to_string(),
        emit_main,
    };
    g.run(prog, checked, ui_prefix, n_variants, initial_variant)?;
    if let Some(e) = g.err { Err(e) } else { Ok(g.out) }
}

struct Gen {
    out: String,
    scopes: Vec<HashMap<String, Ty>>,
    err: Option<CodegenError>,
    top_vars: Vec<(String, Ty, bool)>,
    onclick_fns: Vec<String>,
    key_fns: Vec<String>,
    doc_click_fns: Vec<String>,
    key_event_fns: Vec<String>,
    touch_start_fns: Vec<String>,
    touch_end_fns: Vec<String>,
    el_click_evt_fns: Vec<String>,
    timer_fns: Vec<String>,
    adapter_map: HashMap<String, String>, /* forEach 单参回调 -> 顶层适配器名 */
    /// check 侧标注的调用点实例 key：(line, col) -> key（权威，避免双端推导分歧）
    call_keys: HashMap<(u32, u32), Vec<Ty>>,
    /// 实例形参存储类型：(decl, key) -> [(name, Ty)]（Str 吸收后形参可能是 NWStr）
    inst_params: HashMap<(String, Vec<Ty>), Vec<(String, Ty)>>,
    /// 函数体顶层变量终态类型：(ctx, name) -> Ty（Str 吸收 Num 的 C 存储类型）
    local_types: HashMap<(String, String), Ty>,
    /// 当前正在发射的实例上下文（与 check 的 cur_ctx 同格式 "name#key"）
    cur_fn: Option<String>,
    is_ui: bool,
    /// 多页面符号前缀（"p0_"/"p1_"...；单页为空）——函数/包装器/顶层变量/id 全局按页隔离
    pfx: String,
    /// false = 多页非入口页：不发射 main()，改发射 {pfx}nw_page_init()（页面脚本顶层）
    emit_main: bool,
}

fn cty(t: Ty) -> &'static str {
    match t {
        Ty::Num => "double",
        Ty::Bool => "int",
        Ty::Str => "NWStr",
        Ty::Void => "void",
        Ty::Elem => "NWNode*",
        Ty::Event => "double", /* 不直接落地：sig_params 会把 Event 参数展开成三个 C 参数 */
        Ty::ElemList => "NWList*",
        Ty::StrList => "NWStrList*",
        Ty::Null => "void*", /* 不应出现在定型后的代码；check 保证 */
    }
}

/// 零值/null 的 C 表达式（按目标类型）：Num=null 用 NaN 表示
fn zero_of(t: Ty) -> &'static str {
    match t {
        Ty::Num => "NAN",
        Ty::Bool => "0",
        Ty::Str => "nw_slit(\"\")",
        Ty::Elem | Ty::ElemList | Ty::StrList => "NULL",
        _ => "NULL",
    }
}

/// C 形参列表：Event 参数按处理器种类展开。
/// click 类（onclick/document.onclick/addEventListener("click")）→ (clientX, clientY, target) 三参数；
/// keydown 类（addEventListener("keydown")）→ 单个 NWStr（e.key）。
/// 成员读 e.clientX/e.clientY/e.target/e.key 直接映射到形参（零开销）。
fn sig_params(params: &[(String, Ty)], key_evt: bool) -> String {
    let mut parts: Vec<String> = Vec::new();
    for (n, t) in params {
        match t {
            Ty::Event => {
                let c = c_ident(n);
                if key_evt {
                    parts.push(format!("NWStr {}", c));
                } else {
                    parts.push(format!("double {}_nw_x", c));
                    parts.push(format!("double {}_nw_y", c));
                    parts.push(format!("NWNode* {}_nw_target", c));
                }
            }
            _ => parts.push(format!("{} {}", cty(*t), c_ident(n))),
        }
    }
    if parts.is_empty() { "void".to_string() } else { parts.join(", ") }
}

fn sanitize_c(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).collect()
}

fn mangle_key(key: &[Ty]) -> String {
    key.iter().map(|t| t.mangle()).collect()
}

/// forEach 单参回调预扫描：收集所有 Expr::ForEach 引用的函数名
fn collect_foreach_exprs(e: &Expr, out: &mut Vec<String>) {
    match e {
        Expr::ForEach { fncall, .. } => out.push(fncall.clone()),
        Expr::Unary(_, a) => collect_foreach_exprs(a, out),
        Expr::PostOp(_, a, _, _) => collect_foreach_exprs(a, out),
        Expr::Binary(_, a, b, _, _) => { collect_foreach_exprs(a, out); collect_foreach_exprs(b, out); }
        Expr::Ternary(a, b, c) => { collect_foreach_exprs(a, out); collect_foreach_exprs(b, out); collect_foreach_exprs(c, out); }
        Expr::Call { args, .. } => for a in args { collect_foreach_exprs(a, out); }
        Expr::Tpl(parts) => for p in parts { if let TplNode::Expr(x) = p { collect_foreach_exprs(x, out); } }
        Expr::Index { index, .. } => collect_foreach_exprs(index, out),
        Expr::RegexExec { arg, .. } | Expr::RegexTest { arg, .. } => collect_foreach_exprs(arg, out),
        Expr::DatasetRead { base, .. } | Expr::Closest { base, .. } => collect_foreach_exprs(base, out),
        Expr::Length(a, _, _) => collect_foreach_exprs(a, out),
        _ => {}
    }
}

fn collect_foreach_stmts(stmts: &[Stmt], out: &mut Vec<String>) {
    for s in stmts {
        match s {
            Stmt::ExprStmt(e) => collect_foreach_exprs(e, out),
            Stmt::Let { init, .. } => collect_foreach_exprs(init, out),
            Stmt::Assign { value, .. } | Stmt::MemberAssign { value, .. } | Stmt::ElemMemberAssign { value, .. } => collect_foreach_exprs(value, out),
            Stmt::If { cond, then, els } => {
                collect_foreach_exprs(cond, out);
                collect_foreach_stmts(std::slice::from_ref(then), out);
                if let Some(e) = els { collect_foreach_stmts(std::slice::from_ref(e), out); }
            }
            Stmt::While { cond, body } => {
                collect_foreach_exprs(cond, out);
                collect_foreach_stmts(std::slice::from_ref(body), out);
            }
            Stmt::For { init, cond, step, body } => {
                if let Some(i) = init { collect_foreach_stmts(std::slice::from_ref(i), out); }
                if let Some(c) = cond { collect_foreach_exprs(c, out); }
                if let Some(st) = step { collect_foreach_stmts(std::slice::from_ref(st), out); }
                collect_foreach_stmts(std::slice::from_ref(body), out);
            }
            Stmt::Return(Some(e), _, _) => collect_foreach_exprs(e, out),
            Stmt::Block(b) => collect_foreach_stmts(b, out),
            Stmt::Try { body, .. } => collect_foreach_stmts(body, out),
            _ => {}
        }
    }
}

fn c_ident(name: &str) -> String {
    format!("nwl_{}", name)
}

fn fn_c_name(decl: &str, key: &[Ty]) -> String {
    /* nwh_ 前缀 = 编译器内部保留名（HTML 内联事件属性编译出的处理函数，
     * uigen 生成的绑定代码按原名引用），透传不加 nwf_ 前缀/参数 mangling */
    if let Some(rest) = decl.strip_prefix("nwh_") {
        let _ = rest;
        return decl.to_string();
    }
    format!("nwf_{}__{}", decl, mangle_key(key))
}

impl Gen {
    /// 实例函数 C 名（多页时带页前缀；nwh_* 已由 uigen 页内唯一化，透传）
    fn fnc(&self, decl: &str, key: &[Ty]) -> String {
        if decl.starts_with("nwh_") { return decl.to_string(); }
        format!("{}nwf_{}__{}", self.pfx, decl, mangle_key(key))
    }
    /// 变量 C 名：顶层变量跨页隔离加前缀；函数内局部/形参（含同名遮蔽）不加。
    /// 解析规则：由内向外找名字所属 scope——命中局部 scope → 素名（C 局部遮蔽全局）；
    /// 命中全局 scope（top var）→ 前缀名；未入任何 scope（声明点）→ 函数内 = 素名，
    /// 顶层 = 前缀名（声明与引用点结论一致，避免遮蔽名前后缀不匹配）。
    fn ci(&self, name: &str) -> String {
        for (i, sc) in self.scopes.iter().enumerate().rev() {
            if sc.contains_key(name) {
                return if i == 0 { format!("{}nwl_{}", self.pfx, name) } else { c_ident(name) };
            }
        }
        if self.scopes.len() > 1 {
            c_ident(name)
        } else {
            format!("{}nwl_{}", self.pfx, name)
        }
    }
    /// 声明点 C 名：顶层（main/page_init 体，scopes.len()==1）声明进全局表 → 前缀名；
    /// 函数内声明 → 素名（C 局部，允许遮蔽同名顶层变量）。
    /// 与 ci() 的解析结论一致：声明后引用点命中声明所在 scope，取同名。
    fn decl_name(&self, name: &str) -> String {
        if self.scopes.len() == 1 { format!("{}nwl_{}", self.pfx, name) } else { c_ident(name) }
    }

    /// 事件/定时器包装器 C 名（多页时带页前缀）
    fn wrap(&self, kind: &str, fname: &str) -> String {
        format!("{}{}_{}", self.pfx, kind, sanitize_c(fname))
    }
}

fn escape_c(s: &str) -> String {
    let mut o = String::new();
    for c in s.chars() {
        match c {
            '\"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\t' => o.push_str("\\t"),
            '\r' => o.push_str("\\r"),
            '\0' => o.push_str("\\0"),
            c if (c as u32) < 0x20 => { let _ = write!(o, "\\{:03o}", c as u32); }
            c => o.push(c),
        }
    }
    o
}

/// 窄字符串字面量（C char*）：类选择器是 ASCII ident（valid_ident 保证），安全
fn c_narrow_lit(s: &str) -> String {
    format!("\"{}\"", escape_c(s))
}

fn num_lit(v: f64) -> String {

    if v.is_finite() && v == v.trunc() && v.abs() < 1e15 {
        format!("{:.1}", v)
    } else {
        format!("{:e}", v)
    }
}

impl Gen {
    fn fail(&mut self, msg: impl Into<String>) {
        if self.err.is_none() {
            self.err = Some(CodegenError { msg: msg.into() });
        }
    }

    fn run(&mut self, prog: &Program, checked: &Checked, ui_prefix: &str,
           n_variants: usize, initial_variant: usize) -> Result<(), CodegenError> {
        let _ = prog;
        let mut hdr = String::new();
        let _ = writeln!(hdr, "/* 由 nowebiewc 生成 —— Nowebiew M1 */");
        let _ = writeln!(hdr, "#include <stdio.h>");
        let _ = writeln!(hdr, "#include <math.h>");
        let _ = writeln!(hdr, "#include \"nw_runtime.h\"");
        let _ = writeln!(hdr, "#include \"nw_ui.h\"");
        let _ = writeln!(hdr);

        // UI 模式：HTML/CSS 生成的 DOM 构建代码插在 include 之后
        self.out.push_str(&hdr);

        // 前向声明先行：实例函数与事件/定时器包装器都会被 ui_prefix
        //（nw_app_build_dom 绑定内联事件属性）和实例体引用
        let mut fwd = String::new();
        for inst in &checked.instances {
            let fname = self.fnc(&inst.decl_name, &inst.key);
            let params = sig_params(&inst.params, self.key_event_fns.contains(&inst.decl_name));
            let _ = writeln!(fwd, "static {} {}({});", cty(inst.ret), fname, params);
        }
        for fname in &self.onclick_fns {
            let _ = writeln!(fwd, "static void {}(NWNode* self);", self.wrap("nww", fname));
        }
        for fname in &self.key_fns {
            let _ = writeln!(fwd, "static void {}(int vk);", self.wrap("nwk", fname));
        }
        for fname in &self.timer_fns {
            let _ = writeln!(fwd, "static void {}(void);", self.wrap("nwtm", fname));
        }
        for fname in &self.doc_click_fns {
            let _ = writeln!(fwd, "static void {}(double x, double y, NWNode* nw_target);", self.wrap("nwd", fname));
        }
        for fname in &self.el_click_evt_fns {
            let _ = writeln!(fwd, "static void {}(NWNode* nw_self);", self.wrap("nwe", fname));
        }
        for fname in &self.key_event_fns {
            let _ = writeln!(fwd, "static void {}(const wchar_t* nw_wkey);", self.wrap("nwke", fname));
        }
        for fname in self.touch_start_fns.iter().chain(self.touch_end_fns.iter()) {
            let _ = writeln!(fwd, "static void {}(double x, double y, NWNode* nw_target);", self.wrap("nwt", fname));
        }
        let _ = writeln!(fwd);
        self.out.push_str(&fwd);

        if !ui_prefix.is_empty() {
            self.out.push_str(ui_prefix);
            self.out.push('\n');
        }
        let mut hdr2 = String::new();

        // 顶层 let/const -> C 全局变量（事件处理器可访问）
        for (name, ty, _) in &self.top_vars {
            let _ = writeln!(hdr2, "static {} {};", cty(*ty), self.ci(name));
        }
        let _ = writeln!(hdr2);

        self.out.push_str(&hdr2);

        // forEach 单参回调适配器（预扫描；C 无嵌套函数，须顶层定义）。
        // 前向声明已覆盖实例函数，适配器可安全引用。
        {
            let mut fns: Vec<String> = Vec::new();
            collect_foreach_stmts(&checked.top, &mut fns);
            for inst in &checked.instances {
                collect_foreach_stmts(&inst.body, &mut fns);
            }
            let mut seen: Vec<String> = Vec::new();
            for f in fns {
                if seen.contains(&f) { continue; }
                seen.push(f.clone());
                /* 1 参 (Element) 与 2 参 (Element, index) 回调都包一层 void 适配器：
                 * nw_list_foreach 的回调签名是 void(*)(NWNode*, double)，
                 * 直接传 2 参函数会在回调返回非 void 时类型不兼容 */
                let aname = format!("nwfe_{}", sanitize_c(&f));
                let has1 = checked.instances.iter()
                    .any(|i| i.decl_name == f && i.key == vec![Ty::Elem]);
                let call_expr = if has1 {
                    format!("{}(nw_el);", self.fnc(&f, &vec![Ty::Elem]))
                } else {
                    format!("{}(nw_el, nw_i);", self.fnc(&f, &vec![Ty::Elem, Ty::Num]))
                };
                let _ = writeln!(self.out,
                    "static void {}(NWNode* nw_el, double nw_i) {{ (void)nw_i; {} }}",
                    aname, call_expr);
                self.adapter_map.insert(f, aname);
            }
        }

        // 函数实例
        for inst in &checked.instances {
            let fname = self.fnc(&inst.decl_name, &inst.key);
            let params = sig_params(&inst.params, self.key_event_fns.contains(&inst.decl_name));
            let _ = writeln!(self.out, "static {} {}({}) {{", cty(inst.ret), fname, params);
            self.cur_fn = Some(format!("{}#{}", inst.decl_name,
                inst.key.iter().map(|t| t.mangle()).collect::<String>()));
            let mut global: HashMap<String, Ty> = HashMap::new();
            for (name, ty, _) in &self.top_vars {
                global.insert(name.clone(), *ty);
            }
            /* 两层作用域：[全局层, 函数局部层]。JS 语义：函数内 var 声明是新局部
             * （遮蔽同名全局），重声明判定只看本层；全局仍经 lookup 跨层可读。 */
            self.scopes = vec![global, HashMap::new()];
            for (n, t) in &inst.params {
                self.scopes.last_mut().unwrap().insert(n.clone(), *t);
            }
            for s in &inst.body {
                self.stmt(s);
                if self.err.is_some() { return Err(self.err.clone().unwrap()); }
            }
            // 兜底 return（不可达时无害）
            match inst.ret {
                Ty::Num => self.out.push_str("    return 0;\n"),
                Ty::Bool => self.out.push_str("    return 0;\n"),
                Ty::Str => self.out.push_str("    return nw_slit(\"\");\n"),
                Ty::Void | Ty::Elem => {} // Elem: 无法合成默认元素，check 已保证有 return
                Ty::Event => {} // check 保证 Event 只出现在参数位，不会是返回类型
                Ty::ElemList | Ty::StrList | Ty::Null => {}
            }
            let _ = writeln!(self.out, "}}");
            let _ = writeln!(self.out);
            self.cur_fn = None;
        }

        // onclick 包装器：TS 的 () -> void 函数适配 C 回调签名
        for fname in &self.onclick_fns {
            let cname = self.fnc(fname, &[]);
            let _ = writeln!(self.out,
                "static void {}(NWNode* self) {{ (void)self; {}(); }}",
                self.wrap("nww", fname), cname);
        }
        // onkeydown 包装器：vk（Win32 虚拟键码）适配 TS 的 (number) -> void
        for fname in &self.key_fns {
            let cname = self.fnc(fname, &[Ty::Num]);
            let _ = writeln!(self.out,
                "static void {}(int vk) {{ (void)vk; {}((double)vk); }}",
                self.wrap("nwk", fname), cname);
        }
        // setTimeout 回调包装器：无参定时器回调
        for fname in &self.timer_fns {
            let cname = self.fnc(fname, &[]);
            let _ = writeln!(self.out,
                "static void {}(void) {{ {}(); }}",
                self.wrap("nwtm", fname), cname);
        }
        // document.onclick 包装器：运行时点击事件 (clientX, clientY, target)
        // 适配 TS 的 (e: Event) -> void（Event 参数展开为三个 C 形参，零拷贝）
        for fname in &self.doc_click_fns {
            let cname = self.fnc(fname, &[Ty::Event]);
            let _ = writeln!(self.out,
                "static void {}(double x, double y, NWNode* nw_target) {{ {}(x, y, nw_target); }}",
                self.wrap("nwd", fname), cname);
        }
        // 元素级 addEventListener("click", fn)（fn 带 Event 参数）包装器：
        // 绑定节点自身为 target，坐标以零值占位（元素级事件无全局坐标语义）
        for fname in &self.el_click_evt_fns {
            let cname = self.fnc(fname, &[Ty::Event]);
            let _ = writeln!(self.out,
                "static void {}(NWNode* nw_self) {{ {}(0.0, 0.0, nw_self); }}",
                self.wrap("nwe", fname), cname);
        }
        // addEventListener("keydown", fn) 包装器：运行时键盘事件（e.key 字符串）
        // wchar -> NWStr 转换在包装器内完成，处理器形参即 e.key 本体（零开销读）
        for fname in &self.key_event_fns {
            let cname = self.fnc(fname, &[Ty::Event]);
            let _ = writeln!(self.out,
                "static void {}(const wchar_t* nw_wkey) {{ {}(nw_wide_to_str(nw_wkey)); }}",
                self.wrap("nwke", fname), cname);
        }
        // touchstart/touchend 包装器：运行时触摸事件 (主触点 clientX, clientY, target)
        for fname in self.touch_start_fns.iter().chain(self.touch_end_fns.iter()) {
            let cname = self.fnc(fname, &[Ty::Event]);
            let _ = writeln!(self.out,
                "static void {}(double x, double y, NWNode* nw_target) {{ {}(x, y, nw_target); }}",
                self.wrap("nwt", fname), cname);
        }

        // J3：媒体段切换不再重建 DOM（DOM 永生）——样式重算由 uigen 生成的
        // nw_app_restyle 承担（变体样式 → 主题重放 → 行内声明/display 规则叠加）。
        // 脚本状态、类名、行内声明、事件绑定全部驻留，无需任何重放。
        let _ = writeln!(self.out);

        // main：全局初始化 + 顶层语句
        // 全局作用域（函数体可访问这些变量）
        let mut global: HashMap<String, Ty> = HashMap::new();
        for (name, ty, _) in &self.top_vars {
            global.insert(name.clone(), *ty);
        }
        // 顶层 let 的初始化表达式映射
        let mut top_inits: Vec<(String, Expr)> = Vec::new();
        for s in &checked.top {
            if let Stmt::Let { name, init, .. } = s {
                if self.top_vars.iter().any(|(n, _, _)| n == name) {
                    top_inits.push((name.clone(), init.clone()));
                }
            }
        }
        // 入口页（emit_main=true）：main() 承担启动引导；多页非入口页：只发射
        // {pfx}nw_page_init()（页面脚本顶层，首次导航激活时由运行时惰性调用——
        // 与浏览器「导航到该页才执行其脚本」语义一致）
        if self.emit_main {
            self.out.push_str("int main(void) {\n");
            self.out.push_str("    nw_init();\n");
            if self.is_ui {
                self.out.push_str(&format!("    {}nw_media_init(); /* 媒体段表 + 样式重算回调绑定 */\n", self.pfx));
                self.out.push_str("    nw_app_set_viewport(900, 640); /* vw/vh 求值基准 */\n");
                self.out.push_str(&format!("    {}nw_app_build_dom(); /* DOM 只建一次（永生） */\n", self.pfx));
                let _ = writeln!(self.out, "    {}nw_app_restyle({}); /* 初始媒体段级联应用 */", self.pfx, initial_variant);
            }
        } else {
            self.out.push_str(&format!("void {}nw_page_init(void) {{\n", self.pfx));
        }
        self.scopes = vec![global];
        for (name, init) in &top_inits {
            match self.expr(init) {
                Some((code, t)) => {
                    /* 全局存储类型 = top_vars 终态；数字初始化给 NWStr 全局发隐式转换 */
                    let tt = self.lookup(name).unwrap_or(t);
                    let code = if tt == Ty::Str && (t == Ty::Num || t == Ty::Bool) {
                        self.str_conv(code, t)
                    } else {
                        code
                    };
                    let _ = writeln!(self.out, "    {} = {};", self.ci(name), code);
                }
                None => {}
            }
            if self.err.is_some() { return Err(self.err.clone().unwrap()); }
        }
        for s in &checked.top {
            // 顶层 let 已作为全局初始化，跳过声明本身
            if let Stmt::Let { name, .. } = s {
                if self.top_vars.iter().any(|(n, _, _)| n == name) { continue; }
            }
            self.stmt(s);
            if self.err.is_some() { return Err(self.err.clone().unwrap()); }
        }
        if self.emit_main && self.is_ui {
            self.out.push_str(&format!("    nw_app_run({}nw_title(), 900, 640);\n", self.pfx));
        }
        if self.emit_main {
            self.out.push_str("    return 0;\n}\n");
        } else {
            self.out.push_str("}\n");
        }
        Ok(())
    }

    fn lookup(&self, name: &str) -> Option<Ty> {
        for sc in self.scopes.iter().rev() {
            if let Some(t) = sc.get(name) { return Some(*t); }
        }
        None
    }

    // ============ 语句 ============

    fn stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Empty => {}
            Stmt::Let { name, init, is_const, .. } => {
                let ind = "    ";
                match self.expr(init) {
                    Some((code, t)) => {
                        let _ = is_const;
                        /* C 存储类型 = check 侧终态类型（Str 吸收 Num 后局部声明
                         * 须为 NWStr，数字初始化发隐式 nw_num_str 转换） */
                        let dt = self.cur_fn.as_ref()
                            .and_then(|ctx| self.local_types.get(&(ctx.clone(), name.clone())))
                            .copied()
                            .unwrap_or(t);
                        let code = if dt == Ty::Str && (t == Ty::Num || t == Ty::Bool) {
                            self.str_conv(code, t)
                        } else {
                            code
                        };
                        /* JS var 重声明 = 重新赋值（浏览器语义：同一作用域可重复 var），
                         * C 局部不能重复声明，改为赋值语句 */
                        if self.scopes.last().unwrap().contains_key(name) {
                            let _ = writeln!(self.out, "{}{} = {};", ind, self.ci(name), code);
                        } else {
                            let _ = writeln!(self.out, "{}{} {} = {};", ind, cty(dt), self.decl_name(name), code);
                            self.scopes.last_mut().unwrap().insert(name.clone(), dt);
                        }
                    }
                    None => {}
                }
            }
            Stmt::Assign { name, op, value, .. } => {
                let ind = "    ";
                let vt = match self.lookup(name) {
                    Some(t) => t,
                    None => { self.fail(format!("codegen: 未定义变量 {}", name)); return; }
                };
                let v = match self.expr(value) {
                    Some((c, Ty::Null)) => zero_of(vt).to_string(), /* x = null → 类型零值（NaN/""/NULL） */
                    Some((c, t)) => {
                        /* JS 隐式转换：目标为 NWStr 存储时数字/布尔右值发 nw_num_str/nw_bool_str
                         * （Str 吸收 Num：变量 C 存储已是字符串，赋数字即 String(n)） */
                        if vt == Ty::Str && (t == Ty::Num || t == Ty::Bool) {
                            self.str_conv(c, t)
                        } else {
                            c
                        }
                    }
                    None => return,
                };
                let n = self.ci(name);
                match op {
                    AssignOp::Eq => { let _ = writeln!(self.out, "{}{} = {};", ind, n, v); }
                    AssignOp::AddEq => {
                        match vt {
                            Ty::Num => { let _ = writeln!(self.out, "{}{} += {};", ind, n, v); }
                            Ty::Str => { let _ = writeln!(self.out, "{}{} = nw_cat2({}, {});", ind, n, n, v); }
                            _ => self.fail("codegen: += 类型错误"),
                        }
                    }
                    AssignOp::SubEq => { let _ = writeln!(self.out, "{}{} -= {};", ind, n, v); }
                    AssignOp::MulEq => { let _ = writeln!(self.out, "{}{} *= {};", ind, n, v); }
                    AssignOp::DivEq => { let _ = writeln!(self.out, "{}{} /= {};", ind, n, v); }
                }
            }
            Stmt::MemberAssign { obj, prop, value, line, col } => {
                /* document.onkeydown = fn：全局键盘回调 */
                if obj == "document" && prop == "onkeydown" {
                    if let Expr::Ident(fname, _, _) = value {
                        let _ = writeln!(self.out, "    nw_dom_on_key({});", self.wrap("nwk", fname));
                    } else {
                        self.fail(format!("codegen: onkeydown 右值非法 ({}:{})", line, col));
                    }
                    return;
                }
                /* document.onclick = fn：document 级点击回调（标准事件参数
                 * (clientX, clientY, target)；纯函数指针，不绑节点——
                 * 媒体段重建后依然有效，只需在 main 注册一次） */
                if obj == "document" && prop == "onclick" {
                    if let Expr::Ident(fname, _, _) = value {
                        let _ = writeln!(self.out, "    nw_dom_on_doc_click({});", self.wrap("nwd", fname));
                    } else {
                        self.fail(format!("codegen: document.onclick 右值非法 ({}:{})", line, col));
                    }
                    return;
                }
                /* el.style.<prop> = v：运行时样式（obj 编码 "<elem>@style"；document.body 特例） */
                if let Some(elem) = obj.strip_suffix("@style") {
                    let v = match self.expr(value) { Some((c, _)) => c, None => return };
                    let target = if elem == "document.body" {
                        "nw_dom_body()".to_string()
                    } else {
                        self.ci(elem)
                    };
                    let _ = writeln!(self.out, "    nw_st_style_set({}, \"{}\", nw_str_to_wide({}));",
                        target, Self::camel_to_kebab_c(prop), v);
                    return;
                }
                /* location.hash = "p3"：页面锚点 */
                if obj == "location" && prop == "hash" {
                    let v = match self.expr(value) { Some((c, _)) => c, None => return };
                    let _ = writeln!(self.out, "    nw_location_set_hash(nw_str_to_wide({}));", v);
                    return;
                }
                let oc = self.ci(obj);
                match prop.as_str() {
                    "textContent" => {
                        let v = match self.expr(value) { Some((c, _)) => c, None => return };
                        let _ = writeln!(self.out, "    nw_dom_set_text({}, nw_str_to_wide({}));", oc, v);
                    }
                    "src" => {
                        /* el.src = "path"：img 换图（运行时按路径查嵌入资源表） */
                        let v = match self.expr(value) { Some((c, _)) => c, None => return };
                        let _ = writeln!(self.out, "    nw_el_img_src({}, nw_str_to_wide({}));", oc, v);
                    }
                    "value" => {
                        /* M2：textarea.value 赋值 */
                        let v = match self.expr(value) { Some((c, _)) => c, None => return };
                        let _ = writeln!(self.out, "    nw_ta_set_text({}, nw_str_to_wide({}));", oc, v);
                    }
                    "onclick" => {
                        if let Expr::Ident(fname, _, _) = value {
                            let _ = writeln!(self.out, "    nw_dom_on_click({}, {});",
                                oc, self.wrap("nww", fname));
                        } else {
                            self.fail(format!("codegen: onclick 右值非法 ({}:{})", line, col));
                        }
                    }
                    _ => { self.fail(format!("codegen: 未知属性 {}", prop)); }
                }
            }
            Stmt::ElemMemberAssign { base, prop, value, .. } => {
                let (bc, _) = match self.expr(base) { Some(x) => x, None => return };
                if prop == "onclick" {
                    /* el.onclick = fn：标准 DOM 事件属性（nww_ 包装器由 onclick_fns 统一生成） */
                    if let Expr::Ident(fname, _, _) = value {
                        let _ = writeln!(self.out, "    nw_dom_on_click({}, {});",
                            bc, self.wrap("nww", fname));
                    } else {
                        self.fail("codegen: onclick 右值必须是函数名".to_string());
                    }
                    return;
                }
                let v = match self.expr(value) { Some((c, _)) => c, None => return };
                if let Some(sub) = prop.strip_prefix("style.") {
                    let _ = writeln!(self.out, "    nw_st_style_set({}, \"{}\", nw_str_to_wide({}));",
                        bc, Self::camel_to_kebab_c(sub), v);
                } else if prop == "textContent" {
                    let _ = writeln!(self.out, "    nw_dom_set_text({}, nw_str_to_wide({}));", bc, v);
                } else if prop == "src" {
                    /* getElementById(...).src = "path"：img 换图 */
                    let _ = writeln!(self.out, "    nw_el_img_src({}, nw_str_to_wide({}));", bc, v);
                } else if prop == "scrollTop" {
                    /* 滚动系统未实现：登记的空实现 */
                    let _ = writeln!(self.out, "    nw_el_scroll_top({}, (float)({}));", bc, v);
                } else {
                    self.fail(format!("codegen: ElemMemberAssign 未知属性 {}", prop));
                }
            }
            Stmt::Try { body, .. } => {
                /* catch 已在 parse 期剥离：仅编译 try 体 */
                for s in body {
                    self.stmt(s);
                    if self.err.is_some() { return; }
                }
            }
            Stmt::PostAssign { name, op, .. } => {
                let n = self.ci(name);
                let o = match op { PostOp::Inc => "++", PostOp::Dec => "--" };
                let _ = writeln!(self.out, "    {}{};", n, o);
            }
            Stmt::ExprStmt(e) => {
                if let Some((code, _)) = self.expr(e) {
                    let _ = writeln!(self.out, "    {};", code);
                }
            }
            Stmt::If { cond, then, els } => {
                let (c, ct) = match self.expr(cond) { Some((c, t)) => (c, t), None => return };
                self.out.push_str(&format!("    if ({}) {{\n", self.truthy_c(&c, ct)));
                self.stmt(then);
                if self.err.is_some() { return; }
                match els {
                    Some(e) => {
                        self.out.push_str("    } else {\n");
                        self.stmt(e);
                        if self.err.is_some() { return; }
                    }
                    None => {}
                }
                self.out.push_str("    }\n");
            }
            Stmt::While { cond, body } => {
                let (c, ct) = match self.expr(cond) { Some((c, t)) => (c, t), None => return };
                self.out.push_str(&format!("    while ({}) {{\n", self.truthy_c(&c, ct)));
                self.stmt(body);
                if self.err.is_some() { return; }
                self.out.push_str("    }\n");
            }
            Stmt::For { init, cond, step, body } => {
                // for 头部三段的代码
                let mut head = String::new();
                if let Some(i) = init {
                    match &**i {
                        Stmt::Let { name, init: ie, .. } => {
                            if let Some((code, t)) = self.expr(ie) {
                                let _ = write!(head, "{} {} = {}", cty(t), self.decl_name(name), code);
                                self.scopes.last_mut().unwrap().insert(name.clone(), t);
                            } else { return; }
                        }
                        Stmt::Assign { name, op, value, .. } => {
                            let v = match self.expr(value) { Some((c, _)) => c, None => return };
                            let n = self.ci(name);
                            let ops = match op {
                                AssignOp::Eq => "=",
                                AssignOp::AddEq => "+=",
                                AssignOp::SubEq => "-=",
                                AssignOp::MulEq => "*=",
                                AssignOp::DivEq => "/=",
                            };
                            let _ = write!(head, "{} {} {}", n, ops, v);
                        }
                        Stmt::PostAssign { name, op, .. } => {
                            let o = match op { PostOp::Inc => "++", PostOp::Dec => "--" };
                            let _ = write!(head, "{}{}", self.ci(name), o);
                        }
                        Stmt::ExprStmt(e) => {
                            if let Some((code, _)) = self.expr(e) {
                                let _ = write!(head, "{}", code);
                            } else { return; }
                        }
                        _ => { self.fail("codegen: for-init 语句类型不支持"); return; }
                    }
                }
                let c = match cond {
                    Some(c) => match self.expr(c) {
                        Some((c, t)) => self.truthy_c(&c, t),
                        None => return,
                    },
                    None => String::new(),
                };
                // step 生成为纯表达式片段
                let st = match step {
                    Some(st) => match &**st {
                        Stmt::Assign { name, op, value, .. } => {
                            let v = match self.expr(value) { Some((c, _)) => c, None => return };
                            let n = self.ci(name);
                            match op {
                                AssignOp::Eq => format!("{} = {}", n, v),
                                AssignOp::AddEq => format!("{} += {}", n, v),
                                AssignOp::SubEq => format!("{} -= {}", n, v),
                                AssignOp::MulEq => format!("{} *= {}", n, v),
                                AssignOp::DivEq => format!("{} /= {}", n, v),
                            }
                        }
                        Stmt::PostAssign { name, op, .. } => {
                            let o = match op { PostOp::Inc => "++", PostOp::Dec => "--" };
                            format!("{}{}", self.ci(name), o)
                        }
                        Stmt::ExprStmt(e) => match self.expr(e) {
                            Some((c, _)) => c,
                            None => return,
                        },
                        _ => { self.fail("codegen: for-step 语句类型不支持"); return; }
                    },
                    None => String::new(),
                };
                self.out.push_str(&format!("    for ({}; {}; {}) {{\n", head, c, st));
                self.stmt(body);
                if self.err.is_some() { return; }
                self.out.push_str("    }\n");
            }
            Stmt::Return(e, _, _) => {
                match e {
                    Some(ex) => {
                        if let Some((code, _)) = self.expr(ex) {
                            let _ = writeln!(self.out, "    return {};", code);
                        }
                    }
                    None => self.out.push_str("    return;\n"),
                }
            }
            Stmt::Break(..) => self.out.push_str("    break;\n"),
            Stmt::Continue(..) => self.out.push_str("    continue;\n"),
            Stmt::Block(stmts) => {
                self.out.push_str("    {\n");
                self.scopes.push(HashMap::new());
                for s in stmts {
                    self.stmt(s);
                    if self.err.is_some() { return; }
                }
                self.scopes.pop();
                self.out.push_str("    }\n");
            }
        }
    }

    // ============ 表达式 ============
    // 返回 (C 代码, 类型)。check 阶段已保证类型正确，此处推断失败即为编译器 bug。

    /// JS ToNumber：字符串侧数值化（比较/算术隐式转换，"09" < 10 === true）
    fn num_side(&self, code: &str, t: Ty) -> String {
        if t == Ty::Str { format!("nw_str_num({})", code) } else { code.to_string() }
    }

    fn str_conv(&mut self, code: String, t: Ty) -> String {
        match t {
            Ty::Str => code,
            Ty::Num => format!("nw_num_str({})", code),
            Ty::Bool => format!("nw_bool_str({})", code),
            Ty::Void => String::new(),
            Ty::Elem => {
                self.fail("Element 不能参与字符串拼接");
                code
            }
            Ty::Event => {
                self.fail("Event 不能参与字符串拼接");
                code
            }
            Ty::ElemList | Ty::StrList | Ty::Null => {
                self.fail("数组/null 不能参与字符串拼接");
                code
            }
        }
    }

    /// style 属性名 camelCase → kebab（C 字符串字面量用）：minHeight -> min-height
fn camel_to_kebab_c(s: &str) -> String {
    let mut out = String::new();
    for ch in s.chars() {
        if ch.is_ascii_uppercase() {
            out.push('-');
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

/// JS 真值语义的 C 表达式：Str→长度非零、数组/元素→非 NULL、其余原样
    fn truthy_c(&self, code: &str, t: Ty) -> String {
        match t {
            Ty::Str => format!("(({}).len != 0)", code),
            Ty::ElemList | Ty::StrList | Ty::Elem => format!("(({}) != NULL)", code),
            Ty::Null => "(0)".to_string(),
            _ => code.to_string(),
        }
    }

    /// x === null / x !== null 的 C 表达式（按 x 的类型）
    fn null_eq_c(&self, code: &str, t: Ty, neq: bool) -> String {
        let op = if neq { "!" } else { "" };
        match t {
            Ty::Num => format!("{}isnan({})", op, code),
            Ty::Str => format!("{}(({}).len != 0)", if neq { "" } else { "!" }, code),
            _ => format!("{}(({}) != NULL)", op, code),
        }
    }

    fn expr(&mut self, e: &Expr) -> Option<(String, Ty)> {
        if self.err.is_some() { return None; }
        let r = match e {
            Expr::Num(v) => Some((num_lit(*v), Ty::Num)),
            Expr::Bool(b) => Some((if *b { "1".into() } else { "0".into() }, Ty::Bool)),
            Expr::Str(s) => Some((format!("nw_slit(\"{}\")", escape_c(s)), Ty::Str)),
            Expr::Tpl(parts) => {
                if parts.is_empty() {
                    Some(("nw_slit(\"\")".to_string(), Ty::Str))
                } else {
                    let mut acc: Option<(String, Ty)> = None;
                    for p in parts {
                        let piece = match p {
                            TplNode::Lit(s) => (format!("nw_slit(\"{}\")", escape_c(s)), Ty::Str),
                            TplNode::Expr(ex) => self.expr(ex)?,
                        };
                        acc = Some(match acc {
                            None => (self.str_conv(piece.0, piece.1), Ty::Str),
                            Some((a, _)) => {
                                let b = self.str_conv(piece.0, piece.1);
                                (format!("nw_cat2({}, {})", a, b), Ty::Str)
                            }
                        });
                    }
                    acc
                }
            }
            Expr::Ident(name, line, col) => {
                /* document.body 伪对象：运行时返回当前页根节点（多页导航自动跟随） */
                if name == "document.body" {
                    return Some(("nw_dom_body()".to_string(), Ty::Elem));
                }
                match self.lookup(name) {
                    Some(t) => Some((self.ci(name), t)),
                    None => {
                        self.fail(format!("codegen: 未定义变量 {} ({}:{})", name, line, col));
                        None
                    }
                }
            }
            Expr::Unary(op, inner) => {
                let (c, t) = self.expr(inner)?;
                match op {
                    UnOp::Neg => Some((format!("(-{})", c), Ty::Num)),
                    UnOp::Not => Some((format!("(!{})", self.truthy_c(&c, t)), Ty::Bool)),
                }
            }
            Expr::PostOp(op, inner, _, _) => {
                if let Expr::Ident(name, _, _) = &**inner {
                    let o = match op { PostOp::Inc => "++", PostOp::Dec => "--" };
                    Some((format!("({}{})", self.ci(name), o), Ty::Num))
                } else {
                    self.fail("codegen: ++/-- 操作数必须是变量");
                    None
                }
            }
            Expr::Binary(op, l, r, _, _) => {
                let (lc, lt) = self.expr(l)?;
                let (rc, rt) = self.expr(r)?;
                let res = match op {
                    BinOp::Add => {
                        // 任一侧 Str 则拼接
                        if lt == Ty::Str || rt == Ty::Str {
                            let a = self.str_conv(lc, lt);
                            let b = self.str_conv(rc, rt);
                            (format!("nw_cat2({}, {})", a, b), Ty::Str)
                        } else {
                            (format!("({} + {})", lc, rc), Ty::Num)
                        }
                    }
                    BinOp::Sub => (format!("({} - {})", self.num_side(&lc, lt), self.num_side(&rc, rt)), Ty::Num),
                    BinOp::Mul => (format!("({} * {})", self.num_side(&lc, lt), self.num_side(&rc, rt)), Ty::Num),
                    BinOp::Div => (format!("({} / {})", self.num_side(&lc, lt), self.num_side(&rc, rt)), Ty::Num),
                    BinOp::Mod => (format!("fmod({}, {})", self.num_side(&lc, lt), self.num_side(&rc, rt)), Ty::Num),
                    BinOp::Eq => {
                        if lt == Ty::Null || rt == Ty::Null {
                            /* x === null：Num→isnan、Str→空串、指针→NULL */
                            let (oc, ot) = if lt == Ty::Null { (rc, rt) } else { (lc, lt) };
                            (format!("({})", self.null_eq_c(&oc, ot, false)), Ty::Bool)
                        } else if lt != rt {
                            /* JS === 跨类型恒 false（不隐式转换） */
                            ("0".to_string(), Ty::Bool)
                        } else if lt == Ty::Str {
                            (format!("nw_seq({}, {})", lc, rc), Ty::Bool)
                        } else {
                            (format!("({} == {})", lc, rc), Ty::Bool)
                        }
                    }
                    BinOp::Neq => {
                        if lt == Ty::Null || rt == Ty::Null {
                            let (oc, ot) = if lt == Ty::Null { (rc, rt) } else { (lc, lt) };
                            (format!("({})", self.null_eq_c(&oc, ot, true)), Ty::Bool)
                        } else if lt != rt {
                            ("1".to_string(), Ty::Bool)
                        } else if lt == Ty::Str {
                            (format!("(!nw_seq({}, {}))", lc, rc), Ty::Bool)
                        } else {
                            (format!("({} != {})", lc, rc), Ty::Bool)
                        }
                    }
                    BinOp::Lt => {
                        let cmp = if lt == Ty::Str && rt == Ty::Str {
                            format!("nw_slt({}, {})", lc, rc)
                        } else {
                            format!("({} < {})", self.num_side(&lc, lt), self.num_side(&rc, rt))
                        };
                        (cmp, Ty::Bool)
                    }
                    BinOp::Gt => {
                        let cmp = if lt == Ty::Str && rt == Ty::Str {
                            format!("nw_sgt({}, {})", lc, rc)
                        } else {
                            format!("({} > {})", self.num_side(&lc, lt), self.num_side(&rc, rt))
                        };
                        (cmp, Ty::Bool)
                    }
                    BinOp::Le => {
                        let cmp = if lt == Ty::Str && rt == Ty::Str {
                            format!("(!nw_sgt({}, {}))", lc, rc)
                        } else {
                            format!("({} <= {})", self.num_side(&lc, lt), self.num_side(&rc, rt))
                        };
                        (cmp, Ty::Bool)
                    }
                    BinOp::Ge => {
                        let cmp = if lt == Ty::Str && rt == Ty::Str {
                            format!("(!nw_slt({}, {}))", lc, rc)
                        } else {
                            format!("({} >= {})", self.num_side(&lc, lt), self.num_side(&rc, rt))
                        };
                        (cmp, Ty::Bool)
                    }
                    BinOp::And => (format!("({} && {})", self.truthy_c(&lc, lt), self.truthy_c(&rc, rt)), Ty::Bool),
                    BinOp::Or => {
                        if lt == Ty::Str && rt == Ty::Str {
                            /* dataset.sec || "默认"：左侧非空取左，否则取右 */
                            (format!("nw_str_or({}, {})", lc, rc), Ty::Str)
                        } else {
                            (format!("({} || {})", self.truthy_c(&lc, lt), self.truthy_c(&rc, rt)), Ty::Bool)
                        }
                    }
                };
                Some(res)
            }
            Expr::Ternary(c, a, b) => {
                let (cc, ct) = self.expr(c)?;
                let (ac, at) = self.expr(a)?;
                let (bc, _) = self.expr(b)?;
                Some((format!("({} ? {} : {})", self.truthy_c(&cc, ct), ac, bc), at))
            }
            Expr::Call { callee, args, line, col } => {
                // console.log：分段打印
                if callee == "console.log" {
                    let mut blk = String::from("({ int nwb_1st = 1; (void)nwb_1st;");
                    for a in args {
                        let (ac, at) = self.expr(a)?;
                        let pr = match at {
                            Ty::Num => format!("nw_pnum({});", ac),
                            Ty::Bool => format!("nw_pbool({});", ac),
                            Ty::Str => format!("nw_pstr({});", ac),
                            Ty::Void | Ty::Elem | Ty::Event | Ty::ElemList | Ty::StrList | Ty::Null => { self.fail("console.log 不能打印 void/Element/数组/null"); return None; }
                        };
                        blk.push_str(&format!(" if(!nwb_1st) putchar(' '); nwb_1st = 0; {}", pr));
                    }
                    blk.push_str(" putchar('\\n'); })");
                    Some((blk, Ty::Void))
                } else if callee.starts_with("Math.") || callee == "String" {
                    let mut cs = Vec::new();
                    for a in args {
                        let (ac, _) = self.expr(a)?;
                        cs.push(ac);
                    }
                    let r = match callee.as_str() {
                        "Math.floor" => format!("floor({})", cs[0]),
                        "Math.abs" => format!("fabs({})", cs[0]),
                        "Math.sqrt" => format!("sqrt({})", cs[0]),
                        "Math.max" => format!("fmax({}, {})", cs[0], cs[1]),
                        "Math.min" => format!("fmin({}, {})", cs[0], cs[1]),
                        "String" => {
                            let (t0, c0) = {
                                // 重新取第一个参数与其类型
                                let a = &args[0];
                                let (c, t) = self.expr(a)?;
                                (t, c)
                            };
                            match t0 {
                                Ty::Num => format!("nw_num_str({})", c0),
                                Ty::Bool => format!("nw_bool_str({})", c0),
                                Ty::Str => c0,
                                Ty::Void | Ty::Elem | Ty::Event | Ty::ElemList | Ty::StrList | Ty::Null => { self.fail("String() 不能转换 void/Element/数组/null"); return None; }
                            }
                        }
                        _ => { self.fail(format!("未知内置函数 {}", callee)); return None; }
                    };
                    Some((r, if callee == "String" { Ty::Str } else { Ty::Num }))
                } else if callee == "document.addEventListener" {
                    /* 标准事件注册：keydown -> nwke_、click -> nwd_、touch -> nwt_ 包装器
                     * （check 已验证事件名与处理器签名，此处直接登记） */
                    let fname = match &args[1] {
                        Expr::Ident(n, _, _) => n.clone(),
                        _ => { self.fail("codegen: addEventListener 第二参必须是函数名".to_string()); return None; }
                    };
                    let kind = match &args[0] {
                        Expr::Str(s) => s.as_str(),
                        _ => { self.fail("codegen: addEventListener 事件名必须是字符串字面量".to_string()); return None; }
                    };
                    let r = match kind {
                        "keydown" => format!("nw_dom_on_key_event({});", self.wrap("nwke", &fname)),
                        "touchstart" => format!("nw_dom_on_touch_start({});", self.wrap("nwt", &fname)),
                        "touchend" => format!("nw_dom_on_touch_end({});", self.wrap("nwt", &fname)),
                        _ => format!("nw_dom_on_doc_click({});", self.wrap("nwd", &fname)),
                    };
                    Some((r, Ty::Void))
                } else if let Some(_) = callee.strip_suffix(".preventDefault") {
                    /* e.preventDefault()：桌面窗口无浏览器默认行为（空格滚动/回车提交等），
                     * 语义上等价于已阻止，编译为空 */
                    Some(("((void)0)".to_string(), Ty::Void))
                } else if callee.ends_with(".addEventListener") && callee != "document.addEventListener" {
                    /* element.addEventListener('click', fn)：绑定到节点 onclick 通道。
                     * 0 参处理器走 nww_；带 Event 参数走 nwe_（自身为 target） */
                    let obj = callee.trim_end_matches(".addEventListener");
                    if obj.is_empty() || obj == "window" {
                        self.fail("该对象不支持 addEventListener".to_string()); return None;
                    }
                    let ev = match &args[0] {
                        Expr::Str(s2) => s2.clone(),
                        _ => { self.fail("addEventListener 事件名必须是字符串字面量".to_string()); return None; }
                    };
                    if ev != "click" {
                        self.fail("元素级暂仅支持 addEventListener('click', ...)".to_string()); return None;
                    }
                    let fname = match &args[1] {
                        Expr::Ident(n, _, _) => n.clone(),
                        _ => { self.fail("addEventListener 第二参必须是函数名".to_string()); return None; }
                    };
                    let wrapc = if self.el_click_evt_fns.iter().any(|f| f == &fname) {
                        self.wrap("nwe", &fname)
                    } else {
                        self.wrap("nww", &fname)
                    };
                    Some((format!("nw_dom_on_click({}, {});", self.ci(obj), wrapc), Ty::Void))
                } else if callee == "setTimeout" {
                    /* setTimeout(fn, ms)：包装器 nwtm_<fn> 已在文件头生成 */
                    if let Expr::Ident(fname, _, _) = &args[0] {
                        let (msc, _) = self.expr(&args[1])?;
                        Some((format!("nw_timer_after({}, {})", msc, self.wrap("nwtm", fname)), Ty::Num))
                    } else {
                        self.fail("codegen: setTimeout 第一参数必须是函数名".to_string());
                        None
                    }
                } else if callee == "fs.readText" || callee == "fs.writeText"
                    || callee == "clipboard.setText" || callee == "clipboard.getText"
                    || callee == "document.queryCount" || callee == "document.showOnly"
                    || callee == "document.setTheme" || callee == "document.querySelectorAll"
                    || callee == "window.innerWidth" || callee == "window.innerHeight"
                    || callee == "window.print" || callee == "window.matchMedia.matches"
                    || callee == "window.matchMedia"
                    || callee == "localStorage.getItem" || callee == "localStorage.setItem"
                    || callee == "document.documentElement.setAttribute" || callee == "document.documentElement.getAttribute"
                    || callee == "location.hash"
                    || callee == "parseInt" || callee == "Array.from"
                    || callee == "Date" || callee.starts_with("Date.")
                    || callee.starts_with("classList.")
                    || callee == "elem.prevSibling"
                    || callee == "openFileDialog" || callee == "saveFileDialog" {
                    /* M2 平台内置：全部走 wide（wchar_t*）API，NWStr 边界转换 */
                    let mut cs = Vec::new();
                    for a in args {
                        let (ac, _) = self.expr(a)?;
                        cs.push(ac);
                    }
                    match callee.as_str() {
                        "fs.readText" => {
                            let r = format!("nw_wide_to_str(nw_fs_read(nw_str_to_wide({})))", cs[0]);
                            Some((r, Ty::Str))
                        }
                        "fs.writeText" => {
                            let r = format!("(nw_fs_write(nw_str_to_wide({}), nw_str_to_wide({})) == 0)", cs[0], cs[1]);
                            Some((r, Ty::Bool))
                        }
                        "clipboard.setText" => {
                            Some((format!("nw_clip_set(nw_str_to_wide({}))", cs[0]), Ty::Void))
                        }
                        "clipboard.getText" => {
                            Some(("nw_wide_to_str(nw_clip_get())".to_string(), Ty::Str))
                        }
                        "document.queryCount" => {
                            /* 类选择器是 ASCII ident（valid_ident 保证），直接窄字面量 */
                            let cls = match &args[0] {
                                Expr::Str(s) => c_narrow_lit(s),
                                _ => { self.fail("codegen: queryCount 参数必须是字符串字面量"); return None; }
                            };
                            Some((format!("(double)nw_query_count({})", cls), Ty::Num))
                        }
                        "document.showOnly" => {
                            let cls = match &args[0] {
                                Expr::Str(s) => c_narrow_lit(s),
                                _ => { self.fail("codegen: showOnly 参数必须是字符串字面量"); return None; }
                            };
                            let (kc, _) = self.expr(&args[1])?;
                            let r = format!("nw_query_show_only({}, (int)({}))", cls, kc);
                            Some((r, Ty::Void))
                        }
                        "document.setTheme" => {
                            let name = match &args[0] {
                                Expr::Str(s) => format!("nw_slit(\"{}\")", escape_c(s)),
                                _ => { self.fail("codegen: setTheme 参数必须是字符串字面量"); return None; }
                            };
                            Some((format!("nw_theme_set(nw_theme_index(nw_str_to_wide({})))", name), Ty::Void))
                        }
                        "window.innerWidth" => Some(("nw_win_w()".to_string(), Ty::Num)),
                        "window.innerHeight" => Some(("nw_win_h()".to_string(), Ty::Num)),
                        "window.print" => Some(("((void)0)".to_string(), Ty::Void)), /* 桌面端无打印子系统：标准库登记的空实现 */
                        "window.matchMedia.matches" => {
                            let q = match &args[0] {
                                Expr::Str(s) => c_narrow_lit(s),
                                _ => { self.fail("codegen: matchMedia 参数必须是字符串字面量"); return None; }
                            };
                            Some((format!("nw_media_matches({})", q), Ty::Bool))
                        }
                        "window.matchMedia" => Some(("1".to_string(), Ty::Bool)), /* 存在性判断：恒真 */
                        "document.querySelectorAll" => {
                            let sel = match &args[0] {
                                Expr::Str(s) => c_narrow_lit(s),
                                _ => { self.fail("codegen: querySelectorAll 参数必须是字符串字面量"); return None; }
                            };
                            Some((format!("nw_query_all({})", sel), Ty::ElemList))
                        }
                        "Array.from" => {
                            let (c0, _) = self.expr(&args[0])?;
                            Some((c0, Ty::ElemList)) /* 恒等：列表本身就是数组 */
                        }
                        "Date" => Some(("nw_now_ms()".to_string(), Ty::Num)),
                        "Date.getHours" => Some((format!("nw_date_hours({})", cs[0]), Ty::Num)),
                        "Date.getMinutes" => Some((format!("nw_date_minutes({})", cs[0]), Ty::Num)),
                        "Date.getSeconds" => Some((format!("nw_date_seconds({})", cs[0]), Ty::Num)),
                        "Date.getTime" => Some((cs[0].clone(), Ty::Num)), /* 恒等 */
                        "localStorage.getItem" => {
                            let k = match &args[0] {
                                Expr::Str(s) => c_narrow_lit(s),
                                _ => { self.fail("codegen: localStorage 键必须是字符串字面量"); return None; }
                            };
                            Some((format!("nw_ls_get({})", k), Ty::Str))
                        }
                        "localStorage.setItem" => {
                            let k = match &args[0] {
                                Expr::Str(s) => c_narrow_lit(s),
                                _ => { self.fail("codegen: localStorage 键必须是字符串字面量"); return None; }
                            };
                            Some((format!("nw_ls_set({}, nw_str_to_wide({}))", k, cs[1]), Ty::Void))
                        }
                        "document.documentElement.setAttribute" => {
                            let k = match &args[0] {
                                Expr::Str(s) => format!("nw_str_to_wide(nw_slit(\"{}\"))", escape_c(s)),
                                _ => { self.fail("codegen: setAttribute 属性名必须是字符串字面量"); return None; }
                            };
                            Some((format!("nw_doc_set_attr({}, nw_str_to_wide({}))", k, cs[1]), Ty::Void))
                        }
                        "document.documentElement.getAttribute" => {
                            let k = match &args[0] {
                                Expr::Str(s) => format!("nw_str_to_wide(nw_slit(\"{}\"))", escape_c(s)),
                                _ => { self.fail("codegen: getAttribute 属性名必须是字符串字面量"); return None; }
                            };
                            Some((format!("nw_wide_to_str(nw_doc_get_attr({}))", k), Ty::Str))
                        }
                        "location.hash" => {
                            Some(("nw_location_hash()".to_string(), Ty::Str))
                        }
                        "parseInt" => {
                            Some((format!("nw_parse_int(nw_str_to_wide({}))", cs[0]), Ty::Num))
                        }
                        "elem.prevSibling" => {
                            let (ec0, et0) = self.expr(&args[0])?;
                            if et0 != Ty::Elem { self.fail("previousElementSibling 只能用于 Element"); return None; }
                            Some((format!("nw_dom_prev_elem({})", ec0), Ty::Elem))
                        }
                        "classList.add" | "classList.remove" | "classList.toggle" | "classList.contains" => {
                            let (ec, et) = self.expr(&args[0])?;
                            if et != Ty::Elem { self.fail("codegen: classList 首参必须是 Element"); return None; }
                            let cls = match &args[1] {
                                Expr::Str(s) => format!("nw_str_to_wide(nw_slit(\"{}\"))", escape_c(s)),
                                _ => { self.fail("codegen: classList 类名必须是字符串字面量"); return None; }
                            };
                            match callee.as_str() {
                                "classList.add" => Some((format!("nw_cls_add({}, {})", ec, cls), Ty::Void)),
                                "classList.remove" => Some((format!("nw_cls_remove({}, {})", ec, cls), Ty::Void)),
                                "classList.toggle" => {
                                    if cs.len() == 3 {
                                        /* toggle(cls, force)：强制加/删 */
                                        Some((format!("(double)nw_cls_toggle_force({}, {}, (int)({}))", ec, cls, cs[2]), Ty::Num))
                                    } else {
                                        Some((format!("(double)nw_cls_toggle({}, {})", ec, cls), Ty::Num))
                                    }
                                }
                                _ => Some((format!("nw_cls_contains({}, {})", ec, cls), Ty::Bool)),
                            }
                        }
                        "openFileDialog" | "saveFileDialog" => {
                            /* 默认标题：打开/保存（C 宽字面量用 \x 转义避免源码编码问题） */
                            let title = if cs.is_empty() {
                                if callee == "openFileDialog" { "L\"\\x6253\\x5f00\"" } else { "L\"\\x4fdd\\x5b58\"" }.to_string()
                            } else { format!("nw_str_to_wide({})", cs[0]) };
                            let fnname = if callee == "openFileDialog" { "nw_dialog_open" } else { "nw_dialog_save" };
                            let r = format!(
                                "nw_wide_to_str({}({}, L\"文本文件 (*.txt);*.txt;所有文件 (*.*);*.*\"))",
                                fnname, title
                            );
                            Some((r, Ty::Str))
                        }
                        _ => unreachable!(),
                    }
                } else {
                    // 用户函数：实例 key 用 check 侧标注（流类型推导的权威 key）；
                    // C 实参按存储类型对照实例形参做 JS 隐式转换
                    //（Str 吸收：数字实参→NWStr 形参发 nw_num_str；反之 ToNumber）
                    let mut cs: Vec<(String, Ty)> = Vec::new();
                    for a in args {
                        let r = self.expr(a)?;
                        cs.push(r);
                    }
                    let key = self.call_keys.get(&(*line, *col)).cloned()
                        .unwrap_or_else(|| cs.iter().map(|(_, t)| *t).collect());
                    let ptypes: Vec<Ty> = self.inst_params.get(&(callee.clone(), key.clone()))
                        .map(|ps| ps.iter().map(|(_, t)| *t).collect())
                        .unwrap_or_else(|| cs.iter().map(|(_, t)| *t).collect());
                    let mut call_args = Vec::new();
                    for (i, (ac, at)) in cs.iter().enumerate() {
                        let pt = ptypes.get(i).copied().unwrap_or(*at);
                        call_args.push(match (*at, pt) {
                            (Ty::Num, Ty::Str) => format!("nw_num_str({})", ac),
                            (Ty::Str, Ty::Num) => format!("nw_str_num({})", ac),
                            _ => ac.clone(),
                        });
                    }
                    let fname = self.fnc(callee, &key);
                    // ret 类型查实例表由 check 保证存在；这里通过声明前向已含全部实例
                    let ret = self.ret_lookup(callee, &key);
                    Some((format!("{}({})", fname, call_args.join(", ")), ret))
                }
            }
            Expr::ElemValue { obj, .. } => {
                Some((format!("nw_wide_to_str(nw_ta_get_text({}))", self.ci(&obj)), Ty::Str))
            }
            Expr::PropRead { obj, prop, line, col } => {
                let t = match self.lookup(obj) {
                    Some(t) => t,
                    None => { self.fail(format!("codegen: 未定义变量 {}", obj)); return None; }
                };
                if t != Ty::Event {
                    self.fail(format!("codegen: .{} 只能用于 Event 参数 ({}:{})", prop, line, col));
                    return None;
                }
                let c = self.ci(obj);
                match prop.as_str() {
                    /* Event 参数被展开为 C 形参，成员读即形参直读（零开销） */
                    "clientX" => Some((format!("{}_nw_x", c), Ty::Num)),
                    "clientY" => Some((format!("{}_nw_y", c), Ty::Num)),
                    "target" => Some((format!("{}_nw_target", c), Ty::Elem)),
                    "key" => Some((c, Ty::Str)), /* keydown 处理器：形参本身就是 e.key */
                    _ => { self.fail(format!("codegen: Event 无成员 '{}'", prop)); None }
                }
            }
            Expr::GetById { id, .. } => {
                Some((format!("{}nw_el_{}", self.pfx, sanitize_c(id)), Ty::Elem))
            }
            Expr::Length(inner, _, _) => {
                let (c, t) = self.expr(inner)?;
                /* Str.length = .len（值语义结构体）；ElemList.length = ->（指针） */
                match t {
                    Ty::Str => Some((format!("((double)({}).len)", c), Ty::Num)),
                    Ty::ElemList | Ty::StrList => Some((format!("((double)({})->len)", c), Ty::Num)),
                    _ => { self.fail("codegen: .length 只能用于 string 或数组".to_string()); None }
                }
            }
            Expr::Null(..) => Some(("NULL".to_string(), Ty::Null)), /* 具体零值由使用点按类型转 */
            Expr::Zero { ty } => Some((zero_of(*ty).to_string(), *ty)),
            Expr::Index { obj, index, line, col } => {
                let t = match self.lookup(obj) {
                    Some(t) => t,
                    None => { self.fail(format!("codegen: 未定义变量 {} ({}:{})", obj, line, col)); return None; }
                };
                let (ic, _) = self.expr(index)?;
                let oc = self.ci(obj);
                match t {
                    Ty::ElemList => Some((format!("nw_list_at({}, (int)({}))", oc, ic), Ty::Elem)),
                    Ty::StrList => Some((format!("nw_sl_group({}, (int)({}))", oc, ic), Ty::Str)),
                    _ => { self.fail(format!("codegen: '{}' 不是数组 ({}:{})", obj, line, col)); None }
                }
            }
            Expr::RegexExec { pat, arg, line, col } => {
                let (ac, _) = self.expr(arg)?;
                let _ = (line, col);
                Some((format!("nw_regex_exec(\"{}\", nw_str_to_wide({}))", escape_c(pat), ac), Ty::StrList))
            }
            Expr::RegexTest { pat, arg, line, col } => {
                let (ac, _) = self.expr(arg)?;
                let _ = (line, col);
                Some((format!("nw_regex_test(\"{}\", nw_str_to_wide({}))", escape_c(pat), ac), Ty::Bool))
            }
            Expr::DatasetRead { base, key, .. } => {
                let (bc, _) = self.expr(base)?;
                Some((format!("nw_el_dataset_get({}, \"{}\")", bc, escape_c(key)), Ty::Str))
            }
            Expr::Closest { base, sel, .. } => {
                let (bc, _) = self.expr(base)?;
                Some((format!("nw_el_closest({}, \"{}\")", bc, escape_c(sel)), Ty::Bool))
            }
            Expr::ForEach { list, fncall, line, col } => {
                let t = match self.lookup(list) {
                    Some(t) => t,
                    None => { self.fail(format!("codegen: 未定义变量 {} ({}:{})", list, line, col)); return None; }
                };
                if t != Ty::ElemList {
                    self.fail(format!("codegen: forEach 目标不是数组 ({}:{})", line, col));
                    return None;
                }
                /* 回调 C 签名：2 参 (NWNode*, double) 直接用；
                 * 1 参回调由 run() 预扫描生成的顶层适配器适配（C 无嵌套函数） */
                let call = match self.adapter_map.get(fncall) {
                    Some(a) => a.clone(),
                    None => self.fnc(fncall, &vec![Ty::Elem, Ty::Num]),
                };
                Some((format!("nw_list_foreach({}, {});", self.ci(list), call), Ty::Void))
            }
        };
        if self.err.is_some() { None } else { r }
    }
}

// ret_lookup 需要实例表；用线程局部简化（M0 单次运行）
impl Gen {
    fn ret_lookup(&self, callee: &str, key: &[Ty]) -> Ty {
        // Gen 没有实例表引用，用全局静态表（单线程编译器）
        RET_TABLE.with(|t| {
            t.borrow().get(&(callee.to_string(), key.to_vec())).copied().unwrap_or(Ty::Void)
        })
    }
}

use std::cell::RefCell;
thread_local! {
    static RET_TABLE: RefCell<HashMap<(String, Vec<Ty>), Ty>> = RefCell::new(HashMap::new());
}

// generate() 入口处在构造 Gen 前填充 RET_TABLE
pub fn build_ret_table(checked: &Checked) {
    RET_TABLE.with(|t| {
        let mut tb = t.borrow_mut();
        tb.clear();
        for inst in &checked.instances {
            tb.insert((inst.decl_name.clone(), inst.key.clone()), inst.ret);
        }
    });
}
