// Nowebiew M0 - 类型检查 + 惰性单态化
//
// TS 类型可选的直接体现：参数未标类型的函数，按调用点的实参类型
// 特化生成多个版本（类似 C++ 模板 / 泛型实例化）。
// 实例化从顶层语句出发惰性进行，从未被调用的函数不生成代码。
// 递归函数若无显式返回类型标注，将报错要求标注（M0 限制，报错信息明确）。

use std::collections::HashMap;
use crate::ast::*;

#[derive(Debug, Clone)]
pub struct Instance {
    pub decl_name: String,
    pub key: Vec<Ty>,
    pub params: Vec<(String, Ty)>,
    pub ret: Ty,
    pub body: Vec<Stmt>,
    pub line: u32,
    pub col: u32,
}

pub struct Checked {
    pub instances: Vec<Instance>,
    pub top: Vec<Stmt>,
    /// 顶层 let/const（将成为 C 全局变量）：(名字, 类型, 可变)
    pub top_vars: Vec<(String, Ty, bool)>,
    /// 被赋给 .onclick 的函数（需生成 C 包装器）
    pub onclick_fns: Vec<String>,
    /// 被赋给 document.onkeydown 的函数（签名 (number)->void，需生成 C 包装器）
    pub key_fns: Vec<String>,
    /// 被赋给 document.onclick 的函数（签名 (e: Event)->void，需生成 C 包装器）
    pub doc_click_fns: Vec<String>,
    /// 注册到 addEventListener('keydown', fn) 的函数（签名 (e: Event)->void，e.key 可读）
    pub key_event_fns: Vec<String>,
    /// addEventListener('touchstart'/'touchend', fn)（签名 (e: Event)->void，e.touches[0].clientX 可读）
    pub touch_start_fns: Vec<String>,
    pub touch_end_fns: Vec<String>,
    /// 元素级 addEventListener('click', fn) 且 fn 带 Event 参数（nwe_ 包装器：绑定节点 + 零坐标事件）
    pub el_click_evt_fns: Vec<String>,
    /// 注册到 setTimeout(fn, ms) 的回调（签名 ()->void，需生成 C 包装器）
    pub timer_fns: Vec<String>,
    /// 调用点实例 key 标注：(line, col) -> key（codegen 按此查实例，避免双端推导分歧）
    pub call_keys: HashMap<(u32, u32), Vec<Ty>>,
    /// 函数体顶层变量的终态类型：(ctx, name) -> Ty（Str 吸收 Num 后 C 存储类型，
    /// codegen 据此发射局部声明与隐式转换；ctx = "name#key"）
    pub local_types: HashMap<(String, String), Ty>,
}

pub struct CheckError {
    pub msg: String,
    pub line: u32,
    pub col: u32,
}

type CResult<T> = Result<T, CheckError>;

fn cerr(msg: impl Into<String>, line: u32, col: u32) -> CheckError {
    CheckError { msg: msg.into(), line, col }
}

/// JS 真值语义：string 非空、数组/元素非 NULL 视为真
fn truthy_ok(t: Ty) -> bool {
    matches!(t, Ty::Str | Ty::ElemList | Ty::StrList | Ty::Elem)
}

/// JS style 属性名 → CSS 属性名：minHeight → min-height
fn camel_to_kebab(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
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

/// null 定型回写：把 Let 的 Null 初始化替换为带类型的零值表达式
fn fixup_null_lets(stmts: &mut [Stmt], ctx: &str, resolved: &HashMap<(String, String), Ty>) {
    for s in stmts.iter_mut() {
        match s {
            Stmt::Let { name, init, .. } => {
                if matches!(init, Expr::Null(..)) {
                    if let Some(ty) = resolved.get(&(ctx.to_string(), name.clone())) {
                        *init = Expr::Zero { ty: *ty };
                    }
                }
            }
            Stmt::Block(b) => fixup_null_lets(b, ctx, resolved),
            Stmt::Try { body, .. } => fixup_null_lets(body, ctx, resolved),
            Stmt::If { then, els, .. } => {
                fixup_null_lets(std::slice::from_mut(&mut **then), ctx, resolved);
                if let Some(e) = els {
                    fixup_null_lets(std::slice::from_mut(&mut **e), ctx, resolved);
                }
            }
            Stmt::While { body, .. } => fixup_null_lets(std::slice::from_mut(&mut **body), ctx, resolved),
            Stmt::For { init, body, .. } => {
                if let Some(i) = init {
                    fixup_null_lets(std::slice::from_mut(&mut **i), ctx, resolved);
                }
                fixup_null_lets(std::slice::from_mut(&mut **body), ctx, resolved);
            }
            _ => {}
        }
    }
}

#[derive(Clone)]
struct Scope {
    vars: HashMap<String, (Ty, bool)>, // (类型, 可变)
}

struct Mono<'a> {
    decls: HashMap<String, &'a FnDecl>,
    instances: Vec<Instance>,
    index: HashMap<(String, Vec<Ty>), usize>,
    in_progress: Vec<(String, Vec<Ty>)>,
    scopes: Vec<Scope>,
    loop_depth: usize,
    fn_ret: Option<Ty>, // 当前正在检查的函数的返回类型约束
    saw_value_return: bool,
    ret_ty: Option<Ty>,
    top_vars: Vec<(String, Ty, bool)>,
    onclick_fns: Vec<String>,
    key_fns: Vec<String>,
    doc_click_fns: Vec<String>,
    key_event_fns: Vec<String>,
    touch_start_fns: Vec<String>,
    touch_end_fns: Vec<String>,
    el_click_evt_fns: Vec<String>,
    timer_fns: Vec<String>,
    /// 当前正在检查的事件处理器种类：Some(0)=click（clientX/Y/target）、Some(1)=keydown（key）、
    /// Some(2)=touchstart / Some(3)=touchend（clientX/Y/target）
    cur_evt: Option<u8>,
    elem_ids: std::collections::HashSet<String>,
    /// Null 流定型：当前实例上下文标识（顶层 ""，函数 "name#key"）
    cur_ctx: String,
    /// (ctx, name) -> 定型类型（首次真实赋值时记录）
    null_resolved: HashMap<(String, String), Ty>,
    /// 声明为 null 的变量：(ctx, name, line, col)
    null_lets: Vec<(String, String, u32, u32)>,
    /// 声明源为 null 的变量名（定型后仍允许赋回 null —— 复位语义）
    null_origin: std::collections::HashSet<String>,
    /// 调用点实例 key 标注：(line, col) -> key（codegen 按 C 存储类型推导参数会与
    /// 流类型推导的实例 key 分歧，这里由 check 侧权威标注，codegen 直接查表）
    call_keys: HashMap<(u32, u32), Vec<Ty>>,
    /// 函数体顶层变量终态类型：(ctx, name) -> Ty
    local_types: HashMap<(String, String), Ty>,
    /// 延迟回调工作队列：(fn, key, 事件种类)。回调体不属于任何同步调用图，
    /// 在顶层走查完毕后统一实例化（浏览器语义：事件/定时器触发时才执行）
    pending_cbs: Vec<(String, Vec<Ty>, Option<u8>)>,
}

pub fn check(prog: &Program, elem_ids: &std::collections::HashSet<String>) -> CResult<Checked> {
    let mut decls: HashMap<String, &FnDecl> = HashMap::new();
    for f in &prog.fns {
        if decls.insert(f.name.clone(), f).is_some() {
            return Err(cerr(format!("函数 '{}' 重复定义", f.name), f.line, f.col));
        }
    }
    let mut m = Mono {
        decls,
        instances: Vec::new(),
        index: HashMap::new(),
        in_progress: Vec::new(),
        scopes: vec![Scope { vars: HashMap::new() }],
        loop_depth: 0,
        fn_ret: None,
        saw_value_return: false,
        ret_ty: None,
        top_vars: Vec::new(),
        onclick_fns: Vec::new(),
        key_fns: Vec::new(),
        doc_click_fns: Vec::new(),
        key_event_fns: Vec::new(),
        touch_start_fns: Vec::new(),
        timer_fns: Vec::new(),
        touch_end_fns: Vec::new(),
        el_click_evt_fns: Vec::new(),
        cur_evt: None,
        elem_ids: elem_ids.clone(),
        cur_ctx: String::new(),
        null_resolved: HashMap::new(),
        null_lets: Vec::new(),
        null_origin: std::collections::HashSet::new(),
        call_keys: HashMap::new(),
        local_types: HashMap::new(),
        pending_cbs: Vec::new(),
    };
    // 顶层语句是唯一的实例化入口；scopes[0] 恒为全局作用域。
    // 克隆后按可变遍历检查（Null 流定型需要回写 let 的推导类型）
    let mut top_owned = prog.top.clone();
    for s in &mut top_owned {
        m.check_stmt(s)?;
    }
    // nwh_ 前缀 = 编译器内部保留名（HTML 内联事件属性编译出的处理函数），
    // 属于入口点：从不被用户代码调用，须无条件实例化才能生成 C 定义
    for f in &prog.fns {
        if f.name.starts_with("nwh_") {
            m.ret_of(&f.name, Vec::new(), f.line, f.col)?;
            /* 内联 onclick 属性处理器也是事件处理器：注册进 onclick_fns，
             * codegen 据此发射 nww_ 签名适配包装器（与 .onclick=fn 通道同一路径） */
            if !m.onclick_fns.iter().any(|n| n == &f.name) {
                m.onclick_fns.push(f.name.clone());
            }
        }
    }
    // 排空延迟回调队列：回调体（事件处理器/定时器回调）统一在此实例化。
    // 排空过程中注册的新回调（回调里再注册回调）继续入队，直至队空。
    while let Some((fname, key, evt)) = m.pending_cbs.pop() {
        let saved = m.cur_evt;
        m.cur_evt = evt;
        let ret = m.ret_of(&fname, key, 0, 0)?;
        m.cur_evt = saved;
        if ret != Ty::Void {
            return Err(cerr(format!("回调 '{}' 必须返回 void", fname), 0, 0));
        }
    }
    // 终态仍为 Null 的全局变量 = 无法推断类型
    for (name, t, _) in &m.top_vars {
        if *t == Ty::Null {
            return Err(cerr(
                format!("变量 '{}' 初始化为 null 且从未赋过真实值，无法推断类型（null 变量须经首次赋值定型）", name),
                0, 0,
            ));
        }
    }
    for (ctx, name, line, col) in &m.null_lets {
        if !m.null_resolved.contains_key(&(ctx.clone(), name.clone())) {
            return Err(cerr(
                format!("变量 '{}' 初始化为 null 且从未赋过真实值，无法推断类型（null 变量须经首次赋值定型）", name),
                *line, *col,
            ));
        }
    }
    // 定型回写：null 初始化替换为带类型的零值（codegen 需要 C 类型）
    fixup_null_lets(&mut top_owned, "", &m.null_resolved);
    for inst in &mut m.instances {
        let ctx = format!("{}#{}", inst.decl_name, inst.key.iter().map(|t| t.mangle()).collect::<String>());
        fixup_null_lets(&mut inst.body, &ctx, &m.null_resolved);
    }
    Ok(Checked {
        instances: m.instances,
        top: top_owned,
        top_vars: m.top_vars,
        onclick_fns: m.onclick_fns,
        key_fns: m.key_fns,
        doc_click_fns: m.doc_click_fns,
        key_event_fns: m.key_event_fns,
        touch_start_fns: m.touch_start_fns,
        touch_end_fns: m.touch_end_fns,
            el_click_evt_fns: m.el_click_evt_fns,
        timer_fns: m.timer_fns,
        call_keys: m.call_keys,
        local_types: m.local_types,
    })
}

impl<'a> Mono<'a> {
    fn lookup(&self, name: &str) -> Option<(Ty, bool)> {
        for sc in self.scopes.iter().rev() {
            if let Some(v) = sc.vars.get(name) {
                return Some(*v);
            }
        }
        None
    }

    fn declare(&mut self, name: &str, ty: Ty, mutable: bool, line: u32, col: u32) -> CResult<()> {
        let cur = self.scopes.last_mut().unwrap();
        if let Some(&(old_ty, old_mut)) = cur.vars.get(name) {
            /* var 语义：同作用域重声明合法。同类型则覆盖（等价一次赋值）；
             * 类型不同仍报错（子集要求可推断的稳定类型）。 */
            if old_ty == ty && (old_mut || !mutable) {
                cur.vars.insert(name.to_string(), (ty, mutable));
                return Ok(());
            }
            return Err(cerr(format!("变量 '{}' 在同一作用域重复声明且类型不同（{} vs {}）", name, old_ty.name(), ty.name()), line, col));
        }
        cur.vars.insert(name.to_string(), (ty, mutable));
        Ok(())
    }

    /// Str 吸收 Num：把变量的流类型粘性改为 Str。
    /// 支持：顶层变量（联动 top_vars，C 存储类型随 codegen 全局声明提升为 NWStr）
    /// 与函数参数（参数层类型漂移，ret_of 收尾时回写实例形参）。
    /// 块局部变量不支持（C 局部声明类型无法回头改），给出明确诊断。
    fn retype_to_str(&mut self, name: &str, line: u32, col: u32) -> CResult<()> {
        let mut layer = None;
        for (i, sc) in self.scopes.iter().enumerate().rev() {
            if let Some(v) = sc.vars.get(name) {
                if v.0 == Ty::Num { layer = Some(i); }
                break;
            }
        }
        let layer = match layer {
            Some(i) => i,
            None => return Ok(()), /* 变量不在（或已不是）Num：交给后续通用检查 */
        };
        let in_fn = !self.cur_ctx.is_empty();
        let is_global = layer == 0;
        let is_param = in_fn && layer == 1;
        if !is_global && !is_param {
            return Err(cerr(
                format!(
                    "局部变量 '{}' 不支持 number→string 重赋值（Str 吸收仅支持顶层变量与函数参数，请拆成两个变量）",
                    name
                ),
                line, col,
            ));
        }
        if let Some(v) = self.scopes[layer].vars.get_mut(name) {
            v.0 = Ty::Str;
        }
        if is_global {
            for (n, t, _) in self.top_vars.iter_mut() {
                if n == name && *t == Ty::Num { *t = Ty::Str; break; }
            }
        }
        Ok(())
    }

    /// null 变量首次真实赋值：在所有作用域层与 top_vars 中定型
    fn promote(&mut self, name: &str, ty: Ty) {
        for sc in self.scopes.iter_mut().rev() {
            if let Some(v) = sc.vars.get_mut(name) {
                if v.0 == Ty::Null { v.0 = ty; break; }
            }
        }
        let mut top_level = false;
        for (n, t, _) in self.top_vars.iter_mut() {
            if n == name && *t == Ty::Null { *t = ty; top_level = true; break; }
        }
        if top_level {
            /* 顶层声明位的回写键（fixup_null_lets 用 "" 查顶层 let） */
            self.null_resolved.insert((String::new(), name.to_string()), ty);
        }
        self.null_origin.insert(name.to_string());
        self.null_resolved.insert((self.cur_ctx.clone(), name.to_string()), ty);
    }

    // 实例化（或复用）函数特化，返回其返回类型
    fn ret_of(&mut self, fname: &str, key: Vec<Ty>, line: u32, col: u32) -> CResult<Ty> {
        if let Some(&i) = self.index.get(&(fname.to_string(), key.clone())) {
            return Ok(self.instances[i].ret);
        }
        // 相互递归检测
        if let Some(d) = self.decls.get(fname) {
            if self.in_progress.iter().any(|(n, k)| n == fname && *k == key) {
                if let Some(r) = d.ret {
                    return Ok(r); // 显式返回类型可打破循环
                }
                return Err(cerr(
                    format!(
                        "函数 '{}' 存在递归且未标注返回类型。M0 要求递归函数显式标注返回类型，如 `function {}(...): number`",
                        fname, fname
                    ),
                    line, col,
                ));
            }
            let mut decl: FnDecl = (**d).clone();
            // 占位检查：参数个数
            if decl.params.len() != key.len() {
                return Err(cerr(
                    format!("函数 '{}' 期望 {} 个参数，传入了 {}", fname, decl.params.len(), key.len()),
                    line, col,
                ));
            }
            self.in_progress.push((fname.to_string(), key.clone()));
            let mut params: Vec<(String, Ty)> = decl.params.iter().zip(key.iter())
                .map(|((n, _), t)| (n.clone(), *t)).collect();
            // 严格检查函数体（保留全局作用域 scopes[0]，函数可读顶层变量）
            // 全局快照叠加 top_vars 最新类型：顶层 null 变量可能在先前实例的
            // 函数体内被定型（promote 只改了那份快照），这里取终态保证可见。
            let mut global_scope = self.scopes[0].clone();
            for (n, t, m) in &self.top_vars {
                global_scope.vars.insert(n.clone(), (*t, *m));
            }
            let saved_scopes = std::mem::replace(&mut self.scopes,
                vec![global_scope, Scope { vars: HashMap::new() }]);
            let saved_loop = self.loop_depth;
            let saved_fn_ret = self.fn_ret;
            let saved_saw = self.saw_value_return;
            let saved_ret = self.ret_ty;
            let saved_ctx = std::mem::replace(&mut self.cur_ctx,
                format!("{}#{}", fname, key.iter().map(|t| t.mangle()).collect::<String>()));
            self.loop_depth = 0;
            self.fn_ret = decl.ret;
            self.saw_value_return = false;
            self.ret_ty = None;
            for (n, t) in &params {
                /* JS 参数可变（浏览器语义：形参就是局部变量） */
                self.scopes.last_mut().unwrap().vars.insert(n.clone(), (*t, true));
            }
            for s in decl.body.iter_mut() {
                self.check_stmt(s)?;
            }
            // 归纳返回类型
            let ret = match decl.ret {
                Some(t) => {
                    if self.saw_value_return && t == Ty::Void {
                        return Err(cerr(
                            format!("函数 '{}' 标注返回 void，但函数体返回了值", fname),
                            decl.line, decl.col,
                        ));
                    }
                    if self.ret_ty.is_some() && self.ret_ty != Some(t) {
                        return Err(cerr(
                            format!("函数 '{}' 的 return 值类型与标注的返回类型 {} 不符", fname, t.name()),
                            decl.line, decl.col,
                        ));
                    }
                    t
                }
                None => {
                    if let Some(t) = self.ret_ty {
                        t
                    } else {
                        Ty::Void
                    }
                }
            };
            if decl.ret == Some(Ty::Void) && self.saw_value_return {
                unreachable!();
            }
            /* 参数类型漂移回写 + 函数体顶层变量终态类型记录：
             * Str 吸收 Num 会把参数/局部变量的流类型粘性改为 Str，实例的 C 存储
             * 类型随之提升（codegen 据此发射 NWStr 声明与隐式转换） */
            let ctx = self.cur_ctx.clone();
            for (n, (t, _)) in self.scopes.last().unwrap().vars.iter() {
                self.local_types.insert((ctx.clone(), n.clone()), *t);
            }
            for (n, t) in params.iter_mut() {
                if let Some((ct, _)) = self.scopes.last().unwrap().vars.get(n) {
                    if *ct != *t { *t = *ct; }
                }
            }
            self.scopes = saved_scopes;
            self.loop_depth = saved_loop;
            self.fn_ret = saved_fn_ret;
            self.saw_value_return = saved_saw;
            self.ret_ty = saved_ret;
            self.cur_ctx = saved_ctx;
            self.in_progress.pop();
            let inst = Instance {
                decl_name: fname.to_string(),
                key: key.clone(),
                params,
                ret,
                body: decl.body.clone(),
                line: decl.line,
                col: decl.col,
            };
            // 检查参数名重复
            let mut seen = std::collections::HashSet::new();
            for (n, _) in &inst.params {
                if !seen.insert(n.clone()) {
                    return Err(cerr(format!("函数 '{}' 参数 '{}' 重复", fname, n), inst.line, inst.col));
                }
            }
            self.instances.push(inst);
            self.index.insert((fname.to_string(), key), self.instances.len() - 1);
            Ok(ret)
        } else {
            Err(cerr(format!("未定义的函数 '{}'", fname), line, col))
        }
    }

    fn check_stmt(&mut self, s: &mut Stmt) -> CResult<()> {
        match s {
            Stmt::Empty => Ok(()),
            Stmt::Let { name, explicit, init, is_const, line, col } => {
                let t = self.check_expr(init)?;
                if t == Ty::Void {
                    return Err(cerr("不能用 void 值初始化变量", *line, *col));
                }
                if t == Ty::Event {
                    return Err(cerr("Event 不能作为值存储：它只在 document.onclick 处理器参数上存在，读成员用 e.clientX / e.clientY / e.target", *line, *col));
                }
                /* null 初始化：合法（流类型）——类型由首次真实赋值定型；
                 * 终态仍未定型的变量在 check 结束时统一报错 */
                if t == Ty::Null {
                    self.null_lets.push((self.cur_ctx.clone(), name.clone(), *line, *col));
                }
                if let Some(ex) = explicit {
                    if *ex != t {
                        return Err(cerr(
                            format!("变量 '{}' 标注为 {}，但初始值是 {}", name, ex.name(), t.name()),
                            *line, *col,
                        ));
                    }
                }
                self.declare(name, t, !*is_const, *line, *col)?;
                // 顶层（scopes 只有全局一层）let/const 记录为 C 全局变量
                if self.scopes.len() == 1 {
                    self.top_vars.push((name.clone(), t, !*is_const));
                }
                Ok(())
            }
            Stmt::Assign { name, op, value, line, col } => {
                /* 浏览器语义：给未声明的标识符赋值 = 隐式创建顶层全局变量 */
                if self.lookup(name).is_none() && !self.decls.contains_key(name) {
                    let vt2 = self.check_expr(value)?;
                    if vt2 == Ty::Void || vt2 == Ty::Event {
                        return Err(cerr(format!("隐式全局变量 '{}' 的初始值类型非法（{}）", name, vt2.name()), *line, *col));
                    }
                    self.scopes[0].vars.insert(name.clone(), (vt2, true));
                    if !self.top_vars.iter().any(|(n, _, _)| n == name) {
                        self.top_vars.push((name.clone(), vt2, true));
                    }
                    return Ok(());
                }
                let (vt, mutable) = self.lookup(name)
                    .ok_or_else(|| cerr(format!("未定义的变量 '{}'", name), *line, *col))?;
                if !mutable {
                    return Err(cerr(format!("不能给 const 变量 '{}' 赋值", name), *line, *col));
                }
                if vt == Ty::Event {
                    return Err(cerr("Event 不能被赋值或存储（只在 document.onclick 处理器参数上存在）", *line, *col));
                }
                let vt2 = self.check_expr(value)?;
                if vt2 == Ty::Event {
                    return Err(cerr("Event 不能作为值赋给变量（读成员用 e.clientX / e.clientY / e.target）", *line, *col));
                }
                match op {
                    AssignOp::Eq => {
                        if vt == Ty::Null && vt2 != Ty::Null {
                            /* null 变量首次真实赋值：定型（联动 scopes 与 top_vars） */
                            self.promote(name, vt2);
                        } else if vt2 == Ty::Null && self.null_origin.contains(name) {
                            /* 声明源为 null 的变量复位为 null：合法（codegen 发射零值） */
                        } else if vt == Ty::Num && vt2 == Ty::Str {
                            /* JS Str 吸收 Num：字符串赋值使变量粘性变为 string
                             * （浏览器动态类型的静态近似；数字赋值侧由 codegen
                             * 隐式 nw_num_str 转换，字符串读侧按 ToNumber 语义） */
                            self.retype_to_str(name, *line, *col)?;
                        } else if vt != vt2 {
                            return Err(cerr(
                                format!("赋值类型不匹配：'{}' 是 {}，右侧是 {}", name, vt.name(), vt2.name()),
                                *line, *col,
                            ));
                        }
                    }
                    AssignOp::AddEq => {
                        match (vt, vt2) {
                            (Ty::Num, Ty::Num) | (Ty::Str, Ty::Str) => {}
                            _ => return Err(cerr(
                                format!("'+=' 要求两侧同为 number 或同为 string，这里是 {} += {}", vt.name(), vt2.name()),
                                *line, *col,
                            )),
                        }
                    }
                    _ => {
                        if vt != Ty::Num || vt2 != Ty::Num {
                            return Err(cerr("'-=' '*=' '/=' 仅支持 number", *line, *col));
                        }
                    }
                }
                Ok(())
            }
            Stmt::MemberAssign { obj, prop, value, line, col } => {
                /* document.onkeydown = fn：全局键盘（document 不是变量，先于变量查找特判） */
                if obj == "document" && prop == "onkeydown" {
                    let fname = match value {
                        Expr::Ident(n, _, _) => n,
                        _ => return Err(cerr(
                            "onkeydown 只能赋函数名（不支持函数表达式）".to_string(),
                            *line, *col,
                        )),
                    };
                    let d = match self.decls.get(fname) {
                        Some(d) => d,
                        None => return Err(cerr(format!("未定义的函数 '{}'", fname), *line, *col)),
                    };
                    let n_params = d.params.len();
                    let ret = self.ret_of(fname, vec![Ty::Num], *line, *col)?;
                    if n_params != 1 || ret != Ty::Void {
                        return Err(cerr(
                            format!("onkeydown 处理器 '{}' 必须是 (number vk) -> void（vk 为虚拟键码）", fname),
                            *line, *col,
                        ));
                    }
                    if !self.key_fns.iter().any(|f| f == fname) {
                        self.key_fns.push(fname.clone());
                    }
                    return Ok(());
                }
                /* document.onclick = fn：document 级点击（浏览器
                 * document.addEventListener('click', fn) 等价物。
                 * 处理器收到标准事件参数 (e: Event)，可读 e.clientX / e.clientY / e.target；
                 * 判断逻辑（过滤按钮、半屏翻页等）全部留在页面脚本，运行时不做任何策略） */
                if obj == "document" && prop == "onclick" {
                    let fname = match value {
                        Expr::Ident(n, _, _) => n,
                        _ => return Err(cerr(
                            "document.onclick 只能赋函数名（不支持函数表达式）".to_string(),
                            *line, *col,
                        )),
                    };
                    let d = match self.decls.get(fname) {
                        Some(d) => d,
                        None => return Err(cerr(format!("未定义的函数 '{}'", fname), *line, *col)),
                    };
                    if d.params.len() != 1 || d.params[0].1 != Some(Ty::Event) {
                        return Err(cerr(
                            format!("document.onclick 处理器 '{}' 必须是 (e: Event) -> void（如 function {}(e: Event)）", fname, fname),
                            *line, *col,
                        ));
                    }
                    let ret = self.ret_of(fname, vec![Ty::Event], *line, *col)?;
                    if ret != Ty::Void {
                        return Err(cerr(
                            format!("document.onclick 处理器 '{}' 必须返回 void", fname),
                            *line, *col,
                        ));
                    }
                    if !self.doc_click_fns.iter().any(|f| f == fname) {
                        self.doc_click_fns.push(fname.clone());
                    }
                    return Ok(());
                }
                /* el.style.<prop> = v：运行时样式（obj 编码为 "<elem>@style"；
                 * document.body 特例 → "document.body@style"） */
                if let Some(elem) = obj.strip_suffix("@style") {
                    if elem != "document.body" {
                        let et = match self.lookup(elem) {
                            Some((t, _)) => t,
                            None => return Err(cerr(format!("未定义的变量 '{}'", elem), *line, *col)),
                        };
                        if et != Ty::Elem {
                            return Err(cerr(format!(".style 只能用于 Element，'{}' 是 {}", elem, et.name()), *line, *col));
                        }
                    }
                    let kebab = camel_to_kebab(prop);
                    let supported = ["width", "height", "background", "background-color", "background-image", "color",
                                     "min-height", "min-width", "position", "inset", "overflow",
                                     "top", "right", "bottom", "left"];
                    if !supported.contains(&kebab.as_str()) {
                        if kebab == "transform" {
                            eprintln!("nowebiewc: 警告: style.transform 赋值被接受但 transform 渲染未实现（运行时忽略）");
                        } else {
                            return Err(cerr(
                                format!("运行时 style 不支持 '{}'（支持：width/height/background/color/min-height/min-width/position/inset/overflow/top/right/bottom/left）", prop),
                                *line, *col,
                            ));
                        }
                    }
                    let vt = self.check_expr(value)?;
                    if vt != Ty::Str {
                        return Err(cerr(format!("style.{} 只能赋 string（如 \"50%\"、\"#ff0000\"）", prop), *line, *col));
                    }
                    return Ok(());
                }
                /* location.hash = "p3"：页面锚点（运行时存储，location.hash 读回） */
                if obj == "location" && prop == "hash" {
                    let vt = self.check_expr(value)?;
                    if vt != Ty::Str {
                        return Err(cerr("location.hash 只能赋 string", *line, *col));
                    }
                    return Ok(());
                }
                let ot = match self.lookup(obj) {
                    Some((t, _)) => t,
                    None => return Err(cerr(format!("未定义的变量 '{}'", obj), *line, *col)),
                };
                if ot != Ty::Elem {
                    return Err(cerr(
                        format!(".{} 只能用于 Element，'{}' 是 {}", prop, obj, ot.name()),
                        *line, *col,
                    ));
                }
                match prop.as_str() {
                    "textContent" => {
                        let vt = self.check_expr(value)?;
                        if vt != Ty::Str {
                            return Err(cerr(
                                format!("textContent 只能赋 string，右侧是 {}", vt.name()),
                                *line, *col,
                            ));
                        }
                    }
                    "value" => {
                        /* M2：textarea.value 赋值 */
                        let vt = self.check_expr(value)?;
                        if vt != Ty::Str {
                            return Err(cerr(
                                format!("value 只能赋 string，右侧是 {}", vt.name()),
                                *line, *col,
                            ));
                        }
                    }
                    "onclick" => {
                        let fname = match value {
                            Expr::Ident(n, _, _) => n,
                            _ => return Err(cerr(
                                "onclick 只能赋函数名（M1 不支持函数表达式）".to_string(),
                                *line, *col,
                            )),
                        };
                        match self.decls.get(fname) {
                            Some(d) => {
                                if !d.params.is_empty() {
                                    return Err(cerr(
                                        format!("onclick 处理器 '{}' 不能有参数", fname),
                                        *line, *col,
                                    ));
                                }
                            }
                            None => return Err(cerr(format!("未定义的函数 '{}'", fname), *line, *col)),
                        }
                        /* 回调延迟检查：回调体不入当前同步调用图（浏览器语义，
                         * 事件触发时才执行），避免 A→回调→A 被误判为同步递归 */
                        self.pending_cbs.push((fname.clone(), vec![], Some(0)));
                        if !self.onclick_fns.iter().any(|f| f == fname) {
                            self.onclick_fns.push(fname.clone());
                        }
                    }
                    "src" => {
                        /* el.src = "images/x.png"：img 元素换图（路径指向编译期嵌入的资源） */
                        let vt = self.check_expr(value)?;
                        if vt != Ty::Str {
                            return Err(cerr(
                                format!("src 只能赋 string，右侧是 {}", vt.name()),
                                *line, *col,
                            ));
                        }
                    }
                    _ => return Err(cerr(
                        format!("仅支持 .textContent / .onclick / .value / .src，收到 '.{}'", prop),
                        *line, *col,
                    )),
                }
                Ok(())
            }
            Stmt::ElemMemberAssign { base, prop, value, line, col } => {
                // 任意 Elem 表达式（getElementById(...) / list[i]）的成员赋值
                let bt = self.check_expr(base)?;
                if bt != Ty::Elem {
                    return Err(cerr(format!(".{} 赋值目标必须是 Element，这里是 {}", prop, bt.name()), *line, *col));
                }
                if let Some(sub) = prop.strip_prefix("style.") {
                    let kebab = camel_to_kebab(sub);
                    let supported = ["width", "height", "background", "background-color", "background-image", "color",
                                     "min-height", "min-width", "position", "inset", "overflow",
                                     "top", "right", "bottom", "left"];
                    if !supported.contains(&kebab.as_str()) {
                        if kebab == "transform" {
                            eprintln!("nowebiewc: 警告: style.transform 赋值被接受但 transform 渲染未实现（运行时忽略）");
                        } else {
                            return Err(cerr(
                                format!("运行时 style 不支持 '{}'", sub),
                                *line, *col,
                            ));
                        }
                    }
                    let vt = self.check_expr(value)?;
                    if vt != Ty::Str {
                        return Err(cerr(format!("style.{} 只能赋 string", sub), *line, *col));
                    }
                    return Ok(());
                }
                match prop.as_str() {
                    "textContent" => {
                        let vt = self.check_expr(value)?;
                        if vt != Ty::Str {
                            return Err(cerr(format!("textContent 只能赋 string，右侧是 {}", vt.name()), *line, *col));
                        }
                    }
                    "scrollTop" => {
                        let vt = self.check_expr(value)?;
                        if vt != Ty::Num {
                            return Err(cerr("scrollTop 只能赋 number", *line, *col));
                        }
                        eprintln!("nowebiewc: 警告: scrollTop 赋值被接受但滚动系统未实现（运行时忽略）");
                    }
                    "onclick" => {
                        /* el.onclick = fn：标准 DOM 事件属性，等价 addEventListener("click") */
                        let fname = match value {
                            Expr::Ident(n, _, _) => n,
                            _ => return Err(cerr(
                                "onclick 只能赋函数名（不支持函数表达式）".to_string(),
                                *line, *col,
                            )),
                        };
                        match self.decls.get(fname) {
                            Some(d) => {
                                if !d.params.is_empty() {
                                    return Err(cerr(
                                        format!("onclick 处理器 '{}' 不能有参数", fname),
                                        *line, *col,
                                    ));
                                }
                            }
                            None => return Err(cerr(format!("未定义的函数 '{}'", fname), *line, *col)),
                        }
                        /* 回调延迟检查：回调体不入当前同步调用图（浏览器语义，
                         * 事件触发时才执行），避免 A→回调→A 被误判为同步递归 */
                        self.pending_cbs.push((fname.clone(), vec![], Some(0)));
                        if !self.onclick_fns.iter().any(|f| f == fname) {
                            self.onclick_fns.push(fname.clone());
                        }
                    }
                    "src" => {
                        /* el.src = "images/x.png"：img 元素换图（getElementById 等表达式对象） */
                        let vt = self.check_expr(value)?;
                        if vt != Ty::Str {
                            return Err(cerr(format!("src 只能赋 string，右侧是 {}", vt.name()), *line, *col));
                        }
                    }
                    _ => return Err(cerr(
                        format!("Element 成员赋值仅支持 textContent/scrollTop/onclick/src/style.*，不支持 '.{}'", prop),
                        *line, *col,
                    )),
                }
                Ok(())
            }
            Stmt::Try { body, line, col } => {
                // 子集内无抛出来源（fs/短存取返回值而非异常），catch 分支在 parse 期剥离
                eprintln!("nowebiewc: 警告: try/catch 的 catch 分支不会执行（子集内无抛出来源），仅编译 try 体 ({}:{})", line, col);
                for s in body.iter_mut() {
                    self.check_stmt(s)?;
                }
                Ok(())
            }
            Stmt::PostAssign { name, op: _, line, col } => {
                let (vt, mutable) = self.lookup(name)
                    .ok_or_else(|| cerr(format!("未定义的变量 '{}'", name), *line, *col))?;
                if !mutable {
                    return Err(cerr(format!("不能修改 const 变量 '{}'", name), *line, *col));
                }
                if vt != Ty::Num {
                    return Err(cerr("'++' / '--' 仅支持 number", *line, *col));
                }
                Ok(())
            }
            Stmt::ExprStmt(e) => {
                self.check_expr(e)?;
                Ok(())
            }
            Stmt::If { cond, then, els } => {
                let ct = self.check_expr(cond)?;
                if ct != Ty::Bool && !truthy_ok(ct) {
                    return Err(cerr(format!("if 条件必须是 boolean，这里是 {}", ct.name()), 0, 0));
                }
                self.scopes.push(Scope { vars: HashMap::new() });
                self.check_stmt(then)?;
                self.scopes.pop();
                if let Some(e) = els {
                    self.scopes.push(Scope { vars: HashMap::new() });
                    self.check_stmt(e)?;
                    self.scopes.pop();
                }
                Ok(())
            }
            Stmt::While { cond, body } => {
                let ct = self.check_expr(cond)?;
                if ct != Ty::Bool && !truthy_ok(ct) {
                    return Err(cerr("while 条件必须是 boolean", 0, 0));
                }
                self.scopes.push(Scope { vars: HashMap::new() });
                self.loop_depth += 1;
                self.check_stmt(body)?;
                self.loop_depth -= 1;
                self.scopes.pop();
                Ok(())
            }
            Stmt::For { init, cond, step, body } => {
                self.scopes.push(Scope { vars: HashMap::new() });
                if let Some(i) = init {
                    self.check_stmt(i)?;
                }
                if let Some(c) = cond {
                    let ct = self.check_expr(c)?;
                    if ct != Ty::Bool && !truthy_ok(ct) {
                        return Err(cerr("for 条件必须是 boolean", 0, 0));
                    }
                }
                if let Some(st) = step {
                    match &**st {
                        Stmt::Assign { .. } | Stmt::PostAssign { .. } | Stmt::ExprStmt(_) | Stmt::Empty => {}
                        _ => return Err(cerr("for 的步进部分只能是赋值或表达式", 0, 0)),
                    }
                    self.check_stmt(st)?;
                }
                self.loop_depth += 1;
                self.check_stmt(body)?;
                self.loop_depth -= 1;
                self.scopes.pop();
                Ok(())
            }
            Stmt::Return(e, line, col) => {
                if self.fn_ret.is_none() && self.ret_ty.is_none() && self.in_progress.is_empty() {
                    return Err(cerr("顶层不能使用 return", *line, *col));
                }
                match e {
                    Some(ex) => {
                        let t = self.check_expr(ex)?;
                        self.saw_value_return = true;
                        if let Some(exp) = self.fn_ret {
                            if exp == Ty::Void {
                                return Err(cerr("函数返回类型是 void，不能 return 值", *line, *col));
                            }
                            if exp != t {
                                return Err(cerr(
                                    format!("return 值类型 {} 与函数返回类型 {} 不符", t.name(), exp.name()),
                                    *line, *col,
                                ));
                            }
                        }
                        if let Some(prev) = self.ret_ty {
                            if prev != t {
                                return Err(cerr(
                                    format!("函数内有多种 return 类型（{} 与 {}）", prev.name(), t.name()),
                                    *line, *col,
                                ));
                            }
                        } else {
                            self.ret_ty = Some(t);
                        }
                    }
                    None => {
                        if let Some(exp) = self.fn_ret {
                            if exp != Ty::Void {
                                return Err(cerr(
                                    format!("函数返回类型是 {}，必须 return 值", exp.name()),
                                    *line, *col,
                                ));
                            }
                        }
                    }
                }
                Ok(())
            }
            Stmt::Break(line, col) => {
                if self.loop_depth == 0 {
                    return Err(cerr("break 只能在循环内使用", *line, *col));
                }
                Ok(())
            }
            Stmt::Continue(line, col) => {
                if self.loop_depth == 0 {
                    return Err(cerr("continue 只能在循环内使用", *line, *col));
                }
                Ok(())
            }
            Stmt::Block(stmts) => {
                self.scopes.push(Scope { vars: HashMap::new() });
                for s in stmts {
                    self.check_stmt(s)?;
                }
                self.scopes.pop();
                Ok(())
            }
        }
    }

    fn builtin_arity(&self, name: &str, n: usize, line: u32, col: u32) -> CResult<()> {
        let ok = match name {
            "Math.floor" | "Math.abs" | "Math.sqrt" | "String" => n == 1,
            "Math.max" | "Math.min" => n == 2,
            "console.log" => true,
            "fs.readText" => n == 1,
            "fs.writeText" => n == 2,
            "clipboard.setText" => n == 1,
            "clipboard.getText" => n == 0,
            "document.queryCount" => n == 1,
            "document.showOnly" => n == 2,
            "document.setTheme" => n == 1,
            "document.querySelectorAll" => n == 1,
            "window.innerWidth" | "window.innerHeight" => n == 0,
            "window.print" => n == 0,
            "window.matchMedia.matches" => n == 1,
            "window.matchMedia" => n == 0,
            "localStorage.getItem" => n == 1,
            "localStorage.setItem" => n == 2,
            "document.documentElement.setAttribute" => n == 2,
            "document.documentElement.getAttribute" => n == 1,
            "location.hash" => n == 0,
            "parseInt" => n == 1 || n == 2,
            "Array.from" => n == 1,
            "Date" => n == 0,
            "Date.getHours" | "Date.getMinutes" | "Date.getSeconds" | "Date.getTime" => n == 1,
            "setTimeout" => n == 2,
            "classList.add" | "classList.remove" | "classList.contains" => n == 2,
            "classList.toggle" => n == 2 || n == 3,
            "elem.prevSibling" => n == 1,
            "openFileDialog" | "saveFileDialog" => n <= 1,
            _ => false,
        };
        if ok { Ok(()) } else {
            Err(cerr(format!("内置函数 '{}' 参数个数错误（收到 {} 个）", name, n), line, col))
        }
    }

    fn check_expr(&mut self, e: &Expr) -> CResult<Ty> {
        match e {
            Expr::Num(_) => Ok(Ty::Num),
            Expr::Bool(_) => Ok(Ty::Bool),
            Expr::Str(_) => Ok(Ty::Str),
            Expr::Tpl(_) => Ok(Ty::Str),
            Expr::Ident(name, line, col) => {
                /* document.body 伪对象：Element（运行时 = 当前页根节点） */
                if name == "document.body" {
                    return Ok(Ty::Elem);
                }
                self.lookup(name)
                    .map(|(t, _)| t)
                    .ok_or_else(|| cerr(format!("未定义的变量 '{}'", name), *line, *col))
            }
            Expr::Unary(op, inner, ) => {
                let t = self.check_expr(inner)?;
                match op {
                    UnOp::Neg => {
                        if t != Ty::Num { return Err(cerr("取负只能用于 number", 0, 0)); }
                        Ok(Ty::Num)
                    }
                    UnOp::Not => {
                        if t != Ty::Bool && !truthy_ok(t) { return Err(cerr("'!' 只能用于 boolean（或可判定真值的 string/数组/Element）", 0, 0)); }
                        Ok(Ty::Bool)
                    }
                }
            }
            Expr::PostOp(_, inner, line, col) => {
                match &**inner {
                    Expr::Ident(name, l, c) => {
                        let (t, mutable) = self.lookup(name)
                            .ok_or_else(|| cerr(format!("未定义的变量 '{}'", name), *l, *c))?;
                        if !mutable {
                            return Err(cerr(format!("不能修改 const 变量 '{}'", name), *line, *col));
                        }
                        if t != Ty::Num {
                            return Err(cerr("'++' / '--' 仅支持 number", *line, *col));
                        }
                        Ok(Ty::Num)
                    }
                    _ => Err(cerr("'++' / '--' 的操作数必须是变量", *line, *col)),
                }
            }
            Expr::Binary(op, l, r, line, col) => {
                let lt = self.check_expr(l)?;
                let rt = self.check_expr(r)?;
                self.check_binop(*op, lt, rt, *line, *col)
            }
            Expr::Ternary(c, a, b) => {
                let ct = self.check_expr(c)?;
                if ct != Ty::Bool && !truthy_ok(ct) { return Err(cerr("三元条件必须是 boolean", 0, 0)); }
                let at = self.check_expr(a)?;
                let bt = self.check_expr(b)?;
                if at != bt {
                    return Err(cerr(
                        format!("三元表达式两个分支类型不同（{} 与 {}）", at.name(), bt.name()),
                        0, 0,
                    ));
                }
                Ok(at)
            }
            Expr::Call { callee, args, line, col } => {
                // 内置
                if callee == "console.log" {
                    for a in args {
                        let t = self.check_expr(a)?;
                        if t == Ty::Void { return Err(cerr("console.log 不能打印 void", *line, *col)); }
                    }
                    return Ok(Ty::Void);
                }
                // e.preventDefault()：事件对象方法（运行时默认不产生默认行为，编译为空）
                if let Some(obj) = callee.strip_suffix(".preventDefault") {
                    let t = self.lookup(obj)
                        .map(|(t, _)| t)
                        .ok_or_else(|| cerr(format!("未定义的变量 '{}'", obj), *line, *col))?;
                    if t != Ty::Event {
                        return Err(cerr("preventDefault 只能用于事件对象参数", *line, *col));
                    }
                    if !args.is_empty() {
                        return Err(cerr("preventDefault 不接受参数", *line, *col));
                    }
                    return Ok(Ty::Void);
                }
                // element.addEventListener('click', fn)：元素级点击绑定。
                // 处理器 0 参（走 onclick 通道）或 1 个 Event 参数（nwe_ 包装器）。
                if let Some(obj) = callee.strip_suffix(".addEventListener") {
                    if !obj.is_empty() && obj != "document" && obj != "window" {
                        let t = self.lookup(obj).map(|(t, _)| t)
                            .ok_or_else(|| cerr(format!("未定义的变量 '{}'", obj), *line, *col))?;
                        if t != Ty::Elem {
                            return Err(cerr("addEventListener 只能用于 Element（document.addEventListener 除外）", *line, *col));
                        }
                        if args.len() != 2 {
                            return Err(cerr("addEventListener 的参数必须是 (事件名, 处理函数)", *line, *col));
                        }
                        let ev = match &args[0] {
                            Expr::Str(s) => s.clone(),
                            _ => return Err(cerr("addEventListener 事件名必须是字符串字面量", *line, *col)),
                        };
                        if ev != "click" {
                            return Err(cerr(format!("元素级暂仅支持 addEventListener('click', ...)（收到 '{}'）", ev), *line, *col));
                        }
                        let fname = match &args[1] {
                            Expr::Ident(n, _, _) => n.clone(),
                            _ => return Err(cerr("addEventListener 第二参必须是函数名（箭头函数会自动成为函数名）", *line, *col)),
                        };
                        let d = self.decls.get(&fname)
                            .ok_or_else(|| cerr(format!("未定义的函数 '{}'", fname), *line, *col))?;
                        match d.params.len() {
                            0 => {
                                self.pending_cbs.push((fname.clone(), vec![], Some(0)));
                                if !self.onclick_fns.iter().any(|f| f == &fname) {
                                    self.onclick_fns.push(fname.clone());
                                }
                            }
                            1 => {
                                if let Some(pt) = d.params[0].1 {
                                    if pt != Ty::Event {
                                        return Err(cerr(format!("事件处理器 '{}' 的参数类型必须是 Event（或不写参数）", fname), *line, *col));
                                    }
                                }
                                self.pending_cbs.push((fname.clone(), vec![Ty::Event], Some(0)));
                                if !self.el_click_evt_fns.iter().any(|f| f == &fname) {
                                    self.el_click_evt_fns.push(fname.clone());
                                }
                            }
                            _ => return Err(cerr(format!("事件处理器 '{}' 至多 1 个参数", fname), *line, *col)),
                        }
                        return Ok(Ty::Void);
                    }
                }
                // document.addEventListener(kind, fn[, options])：标准事件注册
                if callee == "document.addEventListener" {                    if args.len() != 2 && args.len() != 3 {
                        return Err(cerr("addEventListener 的参数必须是 (事件名, 处理函数[, 选项对象])", *line, *col));
                    }
                    let ev = match &args[0] {
                        Expr::Str(s) => s.clone(),
                        _ => return Err(cerr("addEventListener 事件名必须是字符串字面量（\"keydown\" / \"click\" / \"touchstart\" / \"touchend\"）", *line, *col)),
                    };
                    let fname = match &args[1] {
                        Expr::Ident(n, _, _) => n.clone(),
                        _ => return Err(cerr("addEventListener 第二参必须是函数名（箭头函数会自动成为函数名）", *line, *col)),
                    };
                    let kind: u8 = match ev.as_str() {
                        "keydown" => 1,
                        "click" => 0,
                        "touchstart" => 2,
                        "touchend" => 3,
                        other => return Err(cerr(
                            format!("暂不支持事件 '{}'（当前支持 keydown / click / touchstart / touchend）", other),
                            *line, *col,
                        )),
                    };
                    let d = self.decls.get(&fname)
                        .ok_or_else(|| cerr(format!("未定义的函数 '{}'", fname), *line, *col))?;
                    if d.params.len() != 1 {
                        return Err(cerr(format!("事件处理器 '{}' 必须恰好有 1 个参数（事件对象）", fname), *line, *col));
                    }
                    if let Some(t) = d.params[0].1 {
                        if t != Ty::Event {
                            return Err(cerr(format!("事件处理器 '{}' 的参数类型必须是 Event（或不写类型）", fname), *line, *col));
                        }
                    }
                    let saved = self.cur_evt;
                    self.cur_evt = Some(kind);
                    /* 回调延迟检查：事件处理器体不属于当前同步调用图 */
                    self.pending_cbs.push((fname.clone(), vec![Ty::Event], Some(kind)));
                    self.cur_evt = saved;
                    let list = match kind {
                        1 => &mut self.key_event_fns,
                        2 => &mut self.touch_start_fns,
                        3 => &mut self.touch_end_fns,
                        _ => &mut self.doc_click_fns,
                    };
                    if !list.iter().any(|f| f == &fname) { list.push(fname); }
                    return Ok(Ty::Void);
                }
                // setTimeout(fn, ms)：一次性定时器。fn 为函数名（含匿名函数表达式提升后的名字）
                if callee == "setTimeout" {
                    self.builtin_arity(callee, args.len(), *line, *col)?;
                    let fname = match &args[0] {
                        Expr::Ident(n, _, _) => n.clone(),
                        _ => return Err(cerr("setTimeout 的第一个参数必须是函数名或 function(){} 表达式", *line, *col)),
                    };
                    let d = match self.decls.get(&fname) {
                        Some(d) => d,
                        None => return Err(cerr(format!("setTimeout: 未定义的函数 '{}'", fname), *line, *col)),
                    };
                    if !d.params.is_empty() {
                        return Err(cerr(format!("setTimeout 回调 '{}' 必须是无参函数", fname), *line, *col));
                    }
                    let ms = self.check_expr(&args[1])?;
                    if ms != Ty::Num {
                        return Err(cerr("setTimeout 的第二个参数必须是 number 毫秒", *line, *col));
                    }
                    /* 回调延迟检查：setTimeout 回调体不属于当前同步调用图 */
                    self.pending_cbs.push((fname.clone(), vec![], None));
                    if !self.timer_fns.iter().any(|f| f == &fname) { self.timer_fns.push(fname); }
                    return Ok(Ty::Num); /* 返回定时器 id */
                }
                // M2 平台内置：fs / clipboard / 对话框 / DOM 扩展 / J2 标准库
                if callee == "fs.readText" || callee == "fs.writeText"
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
                    || callee == "setTimeout"
                    || callee.starts_with("classList.")
                    || callee == "elem.prevSibling"
                    || callee == "openFileDialog" || callee == "saveFileDialog" {
                    self.builtin_arity(callee, args.len(), *line, *col)?;
                    let mut argt = Vec::new();
                    for a in args { argt.push(self.check_expr(a)?); }
                    return match callee.as_str() {
                        "fs.readText" => {
                            if argt[0] != Ty::Str { return Err(cerr("fs.readText 的路径必须是 string", *line, *col)); }
                            Ok(Ty::Str)
                        }
                        "fs.writeText" => {
                            if argt[0] != Ty::Str || argt[1] != Ty::Str {
                                return Err(cerr("fs.writeText 的参数必须是 (string 路径, string 内容)", *line, *col));
                            }
                            Ok(Ty::Bool)
                        }
                        "clipboard.setText" => {
                            if argt[0] != Ty::Str {
                                return Err(cerr("clipboard.setText 的参数必须是 string", *line, *col));
                            }
                            Ok(Ty::Void)
                        }
                        "clipboard.getText" => Ok(Ty::Str),
                        "document.queryCount" => {
                            if argt[0] != Ty::Str {
                                return Err(cerr("document.queryCount 的参数必须是 string 类选择器（如 \".slide\"）", *line, *col));
                            }
                            Ok(Ty::Num)
                        }
                        "document.showOnly" => {
                            if argt[0] != Ty::Str || argt[1] != Ty::Num {
                                return Err(cerr("document.showOnly 的参数必须是 (string 类选择器, number 序号)", *line, *col));
                            }
                            Ok(Ty::Void)
                        }
                        "window.innerWidth" | "window.innerHeight" => Ok(Ty::Num),
                        "document.setTheme" => {
                            if argt[0] != Ty::Str {
                                return Err(cerr("document.setTheme 的参数必须是 string 主题名（如 \"light\"）", *line, *col));
                            }
                            Ok(Ty::Void)
                        }
                        "document.querySelectorAll" => {
                            if argt[0] != Ty::Str {
                                return Err(cerr("document.querySelectorAll 的参数必须是 string 选择器（如 \".slide\"、\"button\"）", *line, *col));
                            }
                            Ok(Ty::ElemList)
                        }
                        "Array.from" => {
                            if argt[0] != Ty::ElemList {
                                return Err(cerr("Array.from 当前仅支持 Element[]（querySelectorAll 结果）", *line, *col));
                            }
                            Ok(Ty::ElemList)
                        }
                        "Date" => Ok(Ty::Num), /* epoch 毫秒 */
                        "Date.getHours" | "Date.getMinutes" | "Date.getSeconds" | "Date.getTime" => {
                            if argt[0] != Ty::Num {
                                return Err(cerr(format!("{} 的参数必须是 number（new Date() 的返回值）", callee), *line, *col));
                            }
                            Ok(Ty::Num)
                        }
                        "window.print" => Ok(Ty::Void),
                        "window.matchMedia.matches" => {
                            if argt[0] != Ty::Str {
                                return Err(cerr("matchMedia 的参数必须是 string 查询（如 \"(prefers-color-scheme: light)\"）", *line, *col));
                            }
                            Ok(Ty::Bool)
                        }
                        "window.matchMedia" => Ok(Ty::Bool), /* 存在性判断：恒真 */
                        "localStorage.getItem" => {
                            if argt[0] != Ty::Str {
                                return Err(cerr("localStorage.getItem 的参数必须是 string 键名", *line, *col));
                            }
                            Ok(Ty::Str) /* 缺失键返回空串（null 的等价落地） */
                        }
                        "localStorage.setItem" => {
                            if argt[0] != Ty::Str || argt[1] != Ty::Str {
                                return Err(cerr("localStorage.setItem 的参数必须是 (string 键, string 值)", *line, *col));
                            }
                            Ok(Ty::Void)
                        }
                        "document.documentElement.setAttribute" => {
                            if argt[0] != Ty::Str || argt[1] != Ty::Str {
                                return Err(cerr("setAttribute 的参数必须是 (string 属性名, string 值)", *line, *col));
                            }
                            Ok(Ty::Void)
                        }
                        "document.documentElement.getAttribute" => {
                            if argt[0] != Ty::Str {
                                return Err(cerr("getAttribute 的参数必须是 string 属性名", *line, *col));
                            }
                            Ok(Ty::Str) /* 未设置返回空串 */
                        }
                        "location.hash" => Ok(Ty::Str), /* 读（页面内存取） */
                        "parseInt" => {
                            if argt[0] != Ty::Str || (argt.len() == 2 && argt[1] != Ty::Num) {
                                return Err(cerr("parseInt 的参数必须是 (string, number 进制?)", *line, *col));
                            }
                            Ok(Ty::Num)
                        }
                        "classList.add" | "classList.remove" => {
                            if argt[0] != Ty::Elem || argt[1] != Ty::Str {
                                return Err(cerr("classList.add/remove 的参数必须是 (Element, string 类名)", *line, *col));
                            }
                            Ok(Ty::Void)
                        }
                        "classList.toggle" => {
                            if argt[0] != Ty::Elem || argt[1] != Ty::Str
                                || (argt.len() == 3 && argt[2] != Ty::Bool) {
                                return Err(cerr("classList.toggle 的参数必须是 (Element, string 类名[, boolean 强制])", *line, *col));
                            }
                            Ok(Ty::Num)
                        }
                        "elem.prevSibling" => {
                            if argt[0] != Ty::Elem {
                                return Err(cerr("previousElementSibling 只能用于 Element", *line, *col));
                            }
                            Ok(Ty::Elem)
                        }
                        "classList.contains" => {
                            if argt[0] != Ty::Elem || argt[1] != Ty::Str {
                                return Err(cerr("classList.toggle/contains 的参数必须是 (Element, string 类名)", *line, *col));
                            }
                            Ok(Ty::Bool) /* 浏览器语义：boolean，可直接作 if 条件 */
                        }
                        "openFileDialog" | "saveFileDialog" => {
                            if !argt.is_empty() && argt[0] != Ty::Str {
                                return Err(cerr("对话框标题必须是 string", *line, *col));
                            }
                            Ok(Ty::Str) /* 用户取消时为空串 */
                        }
                        _ => unreachable!(),
                    };
                }
                if callee.starts_with("Math.") || callee == "String" {
                    self.builtin_arity(callee, args.len(), *line, *col)?;
                    let mut argt = Vec::new();
                    for a in args {
                        argt.push(self.check_expr(a)?);
                    }
                    return match callee.as_str() {
                        "Math.floor" | "Math.abs" | "Math.sqrt" => {
                            if argt[0] != Ty::Num {
                                return Err(cerr(format!("{} 的参数必须是 number", callee), *line, *col));
                            }
                            Ok(Ty::Num)
                        }
                        "Math.max" | "Math.min" => {
                            if argt[0] != Ty::Num || argt[1] != Ty::Num {
                                return Err(cerr(format!("{} 的参数必须是 number", callee), *line, *col));
                            }
                            Ok(Ty::Num)
                        }
                        "String" => {
                            if argt[0] == Ty::Void {
                                return Err(cerr("String() 不能转换 void", *line, *col));
                            }
                            Ok(Ty::Str)
                        }
                        _ => unreachable!(),
                    };
                }
                // 用户函数：惰性单态化
                let mut argt = Vec::new();
                for a in args {
                    argt.push(self.check_expr(a)?);
                }
                if argt.iter().any(|t| *t == Ty::Event) {
                    return Err(cerr(
                        "Event 不能作为实参传递（它只在 document.onclick 处理器参数上存在）",
                        *line, *col,
                    ));
                }
                if argt.iter().any(|t| *t == Ty::Null) {
                    return Err(cerr(
                        "null 作为实参传递：变量尚未定型（须先赋真实值再使用）",
                        *line, *col,
                    ));
                }
                /* 调用点实例 key 标注：codegen 按 C 存储类型看实参（ uplifted 变量
                 * 存储已是 Str），与这里的流类型 key 会分歧，由 check 权威标注 */
                self.call_keys.insert((*line, *col), argt.clone());
                self.ret_of(callee, argt, *line, *col)
            }
            Expr::ElemValue { obj, line, col } => {
                let t = self.lookup(obj)
                    .map(|(t, _)| t)
                    .ok_or_else(|| cerr(format!("未定义的变量 '{}'", obj), *line, *col))?;
                if t != Ty::Elem {
                    return Err(cerr(format!(".value 只能用于 Element，'{}' 是 {}", obj, t.name()), *line, *col));
                }
                Ok(Ty::Str)
            }
            Expr::PropRead { obj, prop, line, col } => {
                let t = self.lookup(obj)
                    .map(|(t, _)| t)
                    .ok_or_else(|| cerr(format!("未定义的变量 '{}'", obj), *line, *col))?;
                if t != Ty::Event {
                    return Err(cerr(
                        format!(".{} 成员读只能用于 Event（document.onclick 处理器参数），'{}' 是 {}", prop, obj, t.name()),
                        *line, *col,
                    ));
                }
                match prop.as_str() {
                    "clientX" | "clientY" => {
                        if self.cur_evt == Some(1) {
                            return Err(cerr(
                                "键盘事件没有坐标：e.clientX/e.clientY 只能在 click / touch 处理器里读取".to_string(),
                                *line, *col,
                            ));
                        }
                        Ok(Ty::Num)
                    }
                    "target" => {
                        if self.cur_evt == Some(1) {
                            return Err(cerr(
                                "键盘事件没有 target：e.target 只能在 click / touch 处理器里读取".to_string(),
                                *line, *col,
                            ));
                        }
                        Ok(Ty::Elem)
                    }
                    "key" => {
                        if self.cur_evt != Some(1) {
                            return Err(cerr(
                                "e.key 只能在 addEventListener('keydown', fn) 处理器里读取".to_string(),
                                *line, *col,
                            ));
                        }
                        Ok(Ty::Str)
                    }
                    _ => Err(cerr(
                        format!("Event 只有 clientX / clientY / target / key 成员，没有 '{}'", prop),
                        *line, *col,
                    )),
                }
            }
            Expr::GetById { id, line, col } => {                if self.elem_ids.is_empty() {
                    return Err(cerr("document 未定义（纯逻辑模式没有 DOM；请以项目模式编译 index.html）", *line, *col));
                }
                if !self.elem_ids.contains(id) {
                    return Err(cerr(
                        format!("HTML 中不存在 id=\"{}\" 的元素", id),
                        *line, *col,
                    ));
                }
                Ok(Ty::Elem)
            }
            Expr::Length(inner, line, col) => {
                let t = self.check_expr(inner)?;
                match t {
                    Ty::Str | Ty::ElemList | Ty::StrList => Ok(Ty::Num),
                    _ => Err(cerr(format!(".length 只能用于 string / 数组，这里是 {}", t.name()), *line, *col)),
                }
            }
            Expr::Null(..) => Ok(Ty::Null),
            Expr::Zero { ty } => Ok(*ty),
            Expr::Index { obj, index, line, col } => {
                let it = self.check_expr(index)?;
                if it != Ty::Num {
                    return Err(cerr(format!("下标必须是 number，这里是 {}", it.name()), *line, *col));
                }
                let t = self.lookup(obj)
                    .map(|(t, _)| t)
                    .ok_or_else(|| cerr(format!("未定义的变量 '{}'", obj), *line, *col))?;
                match t {
                    Ty::ElemList => Ok(Ty::Elem),
                    Ty::StrList => Ok(Ty::Str),
                    _ => Err(cerr(format!("'{}' 不是数组（Element[]/string[]），不能下标访问（{}）", obj, t.name()), *line, *col)),
                }
            }
            Expr::RegexExec { pat: _, arg, line, col } => {
                let t = self.check_expr(arg)?;
                if t != Ty::Str {
                    return Err(cerr(".exec 的参数必须是 string", *line, *col));
                }
                Ok(Ty::StrList)
            }
            Expr::RegexTest { pat: _, arg, line, col } => {
                let t = self.check_expr(arg)?;
                if t != Ty::Str {
                    return Err(cerr(".test 的参数必须是 string", *line, *col));
                }
                Ok(Ty::Bool)
            }
            Expr::DatasetRead { base, key, line, col } => {
                let t = self.check_expr(base)?;
                if t != Ty::Elem {
                    return Err(cerr(format!(".dataset 只能用于 Element，这里是 {}", t.name()), *line, *col));
                }
                if key.is_empty() {
                    return Err(cerr("dataset 键名不能为空", *line, *col));
                }
                Ok(Ty::Str)
            }
            Expr::Closest { base, sel, line, col } => {
                let t = self.check_expr(base)?;
                if t != Ty::Elem {
                    return Err(cerr(format!(".closest 只能用于 Element，这里是 {}", t.name()), *line, *col));
                }
                if sel.trim().is_empty() {
                    return Err(cerr("closest 选择器不能为空", *line, *col));
                }
                Ok(Ty::Bool)
            }
            Expr::ForEach { list, fncall, line, col } => {
                let t = self.lookup(list)
                    .map(|(t, _)| t)
                    .ok_or_else(|| cerr(format!("未定义的变量 '{}'", list), *line, *col))?;
                if t != Ty::ElemList {
                    return Err(cerr(format!(".forEach 只能用于 Element[]，'{}' 是 {}", list, t.name()), *line, *col));
                }
                let d = self.decls.get(fncall)
                    .ok_or_else(|| cerr(format!("未定义的函数 '{}'", fncall), *line, *col))?;
                let n = d.params.len();
                if n != 1 && n != 2 {
                    return Err(cerr(format!("forEach 回调 '{}' 必须是 (元素) 或 (元素, 序号)", fncall), *line, *col));
                }
                let ret = if n == 2 {
                    self.ret_of(fncall, vec![Ty::Elem, Ty::Num], *line, *col)?
                } else {
                    self.ret_of(fncall, vec![Ty::Elem], *line, *col)?
                };
                if ret == Ty::Void || ret == Ty::Num || ret == Ty::Bool || ret == Ty::Str {
                    // 返回值被忽略（浏览器同语义），仅要求不是 Event/数组
                    Ok(Ty::Void)
                } else {
                    Ok(Ty::Void)
                }
            }
        }
    }

    fn check_binop(&self, op: BinOp, lt: Ty, rt: Ty, line: u32, col: u32) -> CResult<Ty> {
        use Ty::*;
        let r = match op {
            BinOp::Add => match (lt, rt) {
                (Num, Num) => Num,
                (Str, Str) | (Str, Num) | (Str, Bool) | (Num, Str) | (Bool, Str) => Str,
                _ => return Err(cerr(
                    format!("'+=' 类运算不支持 {} + {}（boolean 不能参与 +）", lt.name(), rt.name()),
                    line, col,
                )),
            },
            BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod => {
                /* JS ToNumber：字符串参与算术时数值化（"6"*2=12），codegen 发 nw_str_num；
                 * null 参与算术 ToNumber(null)=0（null-救值惯用法），codegen 发 0.0 字面量 */
                if (lt == Num && rt == Num) || (lt == Num && rt == Str) || (lt == Str && rt == Num)
                    || (lt == Num && rt == Null) || (lt == Null && rt == Num)
                    || (lt == Str && rt == Null) || (lt == Null && rt == Str) { Num } else {
                    return Err(cerr(
                        format!("算术运算要求两侧都是 number（或可数值化的 string），这里是 {} 和 {}", lt.name(), rt.name()),
                        line, col,
                    ));
                }
            }
            BinOp::Eq | BinOp::Neq => {
                if lt == rt && lt != Void && lt != Null { Bool }
                else if lt == Null && rt == Null { Bool } /* null === null：JS 语义恒真 */
                else if lt == Null && rt != Null && rt != Void && rt != Event { Bool } /* x === null */
                else if rt == Null && lt != Null && lt != Void && lt != Event { Bool }
                else {
                    return Err(cerr(
                        format!("相等比较要求两侧类型相同（M0 采用 === 语义），这里是 {} 与 {}", lt.name(), rt.name()),
                        line, col,
                    ));
                }
            }
            BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge => {
                /* JS ToNumber：字符串与数字比较时字符串数值化（"09" < 10 === true），
                 * codegen 对 Str 侧发 nw_str_num 转换 */
                if (lt == Num && rt == Num) || (lt == Str && rt == Str)
                    || (lt == Num && rt == Str) || (lt == Str && rt == Num) { Bool } else {
                    return Err(cerr(
                        format!("比较运算要求两侧同为 number 或同为 string（或可数值化混用），这里是 {} 与 {}", lt.name(), rt.name()),
                        line, col,
                    ));
                }
            }
            BinOp::And | BinOp::Or => {
                if lt == Bool && rt == Bool { Bool }
                else if op == BinOp::Or && lt == Str && rt == Str { Str } /* dataset.sec || "默认" */
                else {
                    return Err(cerr("'&&' / '||' 两侧必须是 boolean（'||' 额外支持 string 回退）", line, col));
                }
            }
        };
        Ok(r)
    }
}
