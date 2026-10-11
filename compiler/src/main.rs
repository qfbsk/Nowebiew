// Nowebiew M1 - 编译器入口
// 用法 1（纯逻辑）: nowebiewc <input.ts> [-o output.exe] [--runtime <dir>]
// 用法 2（UI 项目）: nowebiewc <project-dir 或 index.html> [-o output.exe]
//   项目目录约定: index.html + style.css（可选） + main.ts（可选）
// 流程: HTML/CSS/TS -> 类型检查/单态化 -> 生成 C -> gcc -> 原生 exe

mod ast;
mod check;
mod codegen;
mod css;
mod html;
mod lexer;
mod parser;
mod uigen;

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Nowebiew M1 编译器");
        eprintln!("用法: nowebiewc <input.ts | project-dir | index.html> [-o output.exe]");
        eprintln!("  [--runtime <dir>] [--layout <libnw_layout.a>] [--script <file>]  # 可选：缺省自动相对编译器 exe 探测（../runtime、../layout/...）");
        std::process::exit(2);
    }
    let mut input: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut runtime: Option<PathBuf> = None;
    let mut layout_lib: Option<PathBuf> = None;
    let mut script_override: Option<PathBuf> = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-o" => { i += 1; output = Some(PathBuf::from(&args[i])); }
            "--runtime" => { i += 1; runtime = Some(PathBuf::from(&args[i])); }
            "--layout" => { i += 1; layout_lib = Some(PathBuf::from(&args[i])); }
            "--script" => { i += 1; script_override = Some(PathBuf::from(&args[i])); }
            other => input = Some(PathBuf::from(other)),
        }
        i += 1;
    }
    let input = input.expect("缺少输入文件");

    // 判定模式
    let is_html = input.extension().map(|e| e == "html").unwrap_or(false);
    let is_dir = input.is_dir();
    let ui_mode = is_html || is_dir;

    if ui_mode {
        compile_ui(input, output, runtime, layout_lib, script_override);
    } else {
        compile_ts(input, output, runtime);
    }
}

fn compile_ui(input: PathBuf, output: Option<PathBuf>, runtime: Option<PathBuf>, layout_lib: Option<PathBuf>, script_override: Option<PathBuf>) {
    let dir: PathBuf = if input.is_dir() {
        input.clone()
    } else {
        input.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| PathBuf::from("."))
    };
    let html_path = if input.is_dir() { dir.join("index.html") } else { input.clone() };
    let css_path = dir.join("style.css");
    let ts_path = dir.join("main.ts");

    /* ===== 多页面收集：入口页 + 递归 a[href] 指向的本地 .html（去重、防环） =====
     * 浏览器语义：整个站点可用；<a href> 导航在运行时由页面注册表驱动（通用机制）。 */
    let mut page_paths: Vec<PathBuf> = vec![html_path.clone()];
    let mut page_docs: Vec<html::Doc> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let norm_key = |p: &PathBuf| p.canonicalize().unwrap_or_else(|_| p.clone()).to_string_lossy().to_lowercase();
    seen.insert(norm_key(&html_path));
    let mut pi = 0usize;
    while pi < page_paths.len() {
        let p = page_paths[pi].clone();
        let src = match std::fs::read_to_string(&p) {
            Ok(s) => s,
            Err(e) => { eprintln!("错误: 无法读取 {}: {}", p.display(), e); std::process::exit(1); }
        };
        let doc = match html::parse_html(&src) {
            Ok(d) => d,
            Err(e) => { eprintln!("HTML 错误 ({}): {}", p.display(), e); std::process::exit(1); }
        };
        /* 本页 a[href] 的本地页面链接（相对本页目录解析为可读路径） */
        let page_dir = p.parent().map(|d| d.to_path_buf()).unwrap_or_else(|| dir.clone());
        let mut links: Vec<PathBuf> = Vec::new();
        collect_page_links(&doc.body, &page_dir, &mut links);
        for l in links {
            let key = norm_key(&l);
            if seen.insert(key) { page_paths.push(l); }
        }
        page_docs.push(doc);
        pi += 1;
    }
    let multi = page_paths.len() > 1;
    let n_pages = page_paths.len();

    /* ===== 逐页编译：CSS/资源/脚本/UI 代码片段（页前缀隔离符号） ===== */
    struct PageOut {
        name: String,        /* 导航名：文件名去 .html（小写） */
        ui: uigen::UiPlan,
        js_c: String,        /* 该页 JS 程序片段（含 ui.code；空 = 无脚本且编译失败） */
        has_init: bool,      /* 是否发射了 {pfx}nw_page_init */
        handlers_defined: bool, /* 内联事件包装器是否已随页面程序定义 */
        pfx: String,
    }
    let mut pages: Vec<PageOut> = Vec::new();
    let mut res_paths: Vec<String> = Vec::new(); // 全站嵌入资源（项目相对路径）
    let mut fonts: Vec<(String, String)> = Vec::new(); // @font-face：(别名, 项目相对路径)

    for (k, doc) in page_docs.iter().enumerate() {
        let hp = &page_paths[k];
        let page_dir = hp.parent().map(|d| d.to_path_buf()).unwrap_or_else(|| dir.clone());
        let pfx = format!("p{}_", k);
        let name = page_name(hp, &page_paths, k);
        eprintln!("页面 [{}] {} {}(脚本前缀 {})", k, name, hp.display(), pfx);

        /* --- 样式来源统一按文档顺序级联（相对本页目录解析） --- */
        let mut user_rules: Vec<css::Rule> = Vec::new();
        for src in &doc.styles {
            match src {
                html::CssSource::Link(href) => {
                    let cp = page_dir.join(href);
                    let css_src = match std::fs::read_to_string(&cp) {
                        Ok(s) => s,
                        Err(e) => { eprintln!("错误: 无法读取样式表 {}: {}", cp.display(), e); std::process::exit(1); }
                    };
                    /* 浏览器语义：CSS 内 url() 相对样式表所在目录 → 改写为项目相对路径 */
                    let base_rel = href.replace('\\', "/")
                        .rsplit_once('/').map(|(d, _)| d.to_string()).unwrap_or_default();
                    let css_src = extract_font_faces(&css_src, &base_rel, &dir, &mut res_paths, &mut fonts);
                    let css_src = rewrite_css_urls(&css_src, &base_rel, &dir, &mut res_paths);
                    match css::parse_css(&css_src) {
                        Ok((r, warns)) => {
                            for w in &warns { eprintln!("警告: 忽略不支持的选择器规则: {}", w); }
                            user_rules.extend(r);
                        }
                        Err(e) => { eprintln!("CSS 错误 ({}): {}", cp.display(), e); std::process::exit(1); }
                    }
                }
                html::CssSource::Inline(css_src) => {
                    let css_src = extract_font_faces(css_src, "", &dir, &mut res_paths, &mut fonts);
                    let css_src = rewrite_css_urls(&css_src, "", &dir, &mut res_paths);
                    match css::parse_css(&css_src) {
                        Ok((r, warns)) => {
                            for w in &warns { eprintln!("警告: 忽略不支持的选择器规则: {}", w); }
                            user_rules.extend(r);
                        }
                        Err(e) => { eprintln!("CSS 错误 (<style> 块): {}", e); std::process::exit(1); }
                    }
                }
            }
        }
        if doc.styles.is_empty() && k == 0 && css_path.exists() {
            let css_src = match std::fs::read_to_string(&css_path) {
                Ok(s) => s,
                Err(e) => { eprintln!("错误: 无法读取样式表 {}: {}", css_path.display(), e); std::process::exit(1); }
            };
            let css_src = extract_font_faces(&css_src, "", &dir, &mut res_paths, &mut fonts);
            let css_src = rewrite_css_urls(&css_src, "", &dir, &mut res_paths);
            match css::parse_css(&css_src) {
                Ok((r, warns)) => {
                    for w in &warns { eprintln!("警告: 忽略不支持的选择器规则: {}", w); }
                    user_rules.extend(r);
                }
                Err(e) => { eprintln!("CSS 错误 ({}): {}", css_path.display(), e); std::process::exit(1); }
            }
        }
        // 收集本页所有 id（供 checker 校验 getElementById）
        let mut elem_ids = std::collections::HashSet::new();
        collect_element_ids(&doc.body, &mut elem_ids);

        // uigen：本页 DOM 构建 + 样式重算表（页前缀符号隔离）
        let mut ui = match uigen::generate(doc, &user_rules, &pfx, multi) {
            Ok(u) => u,
            Err(e) => { eprintln!("UI 生成错误 ({}): {}", name, e); std::process::exit(1); }
        };

        // <img src>：静态资源引用，存在才嵌入
        collect_img_srcs(&doc.body, &dir, &mut res_paths);

        // 本页脚本来源（远程 http(s) 跳过 + 警告——AOT 无法嵌入执行任意网页脚本）
        let ts_src = if let Some(sp) = &script_override {
            if k != 0 { String::new() } else {
                let full = if sp.is_absolute() { sp.clone() } else { dir.join(sp) };
                match std::fs::read_to_string(&full) {
                    Ok(s) => s,
                    Err(e) => { eprintln!("错误: 无法读取脚本 {}: {}", full.display(), e); std::process::exit(1); }
                }
            }
        } else if !doc.scripts.is_empty() {
            let mut parts: Vec<String> = Vec::new();
            for sc in &doc.scripts {
                match sc {
                    html::ScriptCode::File(rel) => {
                        if rel.contains("://") || rel.starts_with("//") {
                            eprintln!("警告: 页面 {} 的远程脚本 {} 无法 AOT 打包，已跳过（浏览器脚本加载失败语义：页面照常渲染）", name, rel);
                            continue;
                        }
                        let sp = page_dir.join(rel);
                        match std::fs::read_to_string(&sp) {
                            Ok(s) => parts.push(s),
                            Err(e) => { eprintln!("错误: 无法读取脚本 {}: {}", sp.display(), e); std::process::exit(1); }
                        }
                    }
                    html::ScriptCode::Inline(s) => parts.push(s.clone()),
                }
            }
            parts.join("\n")
        } else if k == 0 && ts_path.exists() {
            std::fs::read_to_string(&ts_path).unwrap_or_default()
        } else { String::new() };
        /* 内联事件属性处理函数并入本页同一编译管线（同一 JS 子集、同一类型检查） */
        let ts_src = if ui.extra_js.is_empty() { ts_src } else { format!("{}\n{}", ts_src, ui.extra_js) };
        let mut dropped_scripts = false; /* 页面脚本已丢弃（仅剩内联处理重试） */
        let mut full_skip = false;       /* 整页脚本放弃（含内联处理） */

        /* --- 本页 JS 程序编译（浏览器语义：脚本错误该脚本不执行，页面照常渲染） --- */
        let mut js_c = String::new();
        let mut has_init = false;
        let mut handlers_defined = false; /* 内联事件包装器是否已随页面程序定义 */
        let mut program_src = ts_src.clone();
        loop {
            let scan_src = program_src.clone();
            scan_js_assets(&scan_src, &dir, &mut res_paths);
            match parser::parse(&program_src) {
                Ok(prog) => match check::check(&prog, &elem_ids) {
                    Ok(checked) => {
                        codegen::build_ret_table(&checked);
                        match codegen::generate(&prog, &checked, true, &ui.code, ui.n_variants, ui.initial_variant, &pfx, k == 0) {
                            Ok(c) => {
                                js_c = c; has_init = true;
                                handlers_defined = !full_skip;
                            }
                            Err(e) => { if degrade(&name, &format!("codegen: {}", e.msg), &mut program_src, &ui.extra_js) { dropped_scripts = true; } else { full_skip = true; } }
                        }
                    }
                    Err(e) => { if degrade(&name, &format!("类型检查 [{}:{}]: {}", e.line, e.col, e.msg), &mut program_src, &ui.extra_js) { dropped_scripts = true; } else { full_skip = true; } }
                },
                Err(e) => { if degrade(&name, &format!("语法 [{}:{}]: {}", e.line, e.col, e.msg), &mut program_src, &ui.extra_js) { dropped_scripts = true; } else { full_skip = true; } }
            }
            if !js_c.is_empty() || program_src.is_empty() { break; }
        }
        if js_c.is_empty() {
            /* 无脚本/脚本全弃：仍发射 ui.code（构建/重算/标题必须存在）——
             * 用 codegen 空程序走一遍 UI 模式非入口路径，得到 hdr + ui.code + 空 init */
            /* <noscript> 生效（浏览器语义：页脚本全部未执行时其内容生效）：
             * 把 noscript 内 <style> 追加到级联最末，重跑 uigen 得到带降级规则的
             * 规则表/重算表（如 experience 页 loaderbg loading 遮罩 display:none）。
             * dropped_scripts（仅内联事件存活）不算——脚本引擎仍在跑，不生效。 */
            if !doc.noscript_styles.is_empty() {
                let mut rules2 = user_rules.clone();
                let mut ns_ok = true;
                for ns in &doc.noscript_styles {
                    let css_src = rewrite_css_urls(ns, "", &dir, &mut res_paths);
                    match css::parse_css(&css_src) {
                        Ok((r, warns)) => {
                            for w in &warns { eprintln!("警告: 忽略不支持的选择器规则: {}", w); }
                            rules2.extend(r);
                        }
                        Err(e) => { eprintln!("警告: <noscript> 内 CSS 解析失败: {}", e); ns_ok = false; break; }
                    }
                }
                if ns_ok {
                    match uigen::generate(doc, &rules2, &pfx, multi) {
                        Ok(u2) => ui = u2,
                        Err(e) => { eprintln!("警告: noscript 重生成 UI 失败 ({}): {}（按无 noscript 渲染）", name, e); }
                    }
                }
            }
            let prog = match parser::parse("") { Ok(p) => p, Err(_) => continue };
            let empty_ids = std::collections::HashSet::new();
            if let Ok(checked) = check::check(&prog, &empty_ids) {
                codegen::build_ret_table(&checked);
                if let Ok(c) = codegen::generate(&prog, &checked, true, &ui.code, ui.n_variants, ui.initial_variant, &pfx, k == 0) {
                    js_c = c;
                    has_init = k != 0; /* 空脚本的 init 无意义，但发射了也无害——保持一致 */
                }
            }
        }
        pages.push(PageOut { name, ui, js_c, has_init, handlers_defined, pfx: pfx.clone() });
    }

    /* 页面 0 的空程序路径里 has_init 恒 false（入口顶层在 main 内执行） */

    let c_src_prefix: String = pages.iter().map(|p| p.js_c.clone() + "\n").collect();

    /* ===== 页面注册表（多页）：a[href] 导航 + 惰性构建/脚本初始化 + 每页媒体段 ===== */
    let mut nav_c = String::new();
    /* 页面脚本被整体跳过的页：内联事件包装器无定义，发射空壳（浏览器语义：
     * 脚本错误时 on* 属性处理失效，点击无效果，页面照常渲染） */
    for p in pages.iter().filter(|p| !p.handlers_defined && !p.ui.handler_names.is_empty()) {
        nav_c.push_str(&format!("/* 页面 {} 脚本降级：内联事件处理空壳 */\n", p.name));
        for h in &p.ui.handler_names {
            nav_c.push_str(&format!(
                "static void {}nww_{}(NWNode* self) {{ (void)self; }}\n", p.pfx, h));
        }
    }
    if multi {
        nav_c.push_str("/* ===== 多页面注册表（编译器生成；<a href> 导航语义） ===== */\n");
        nav_c.push_str("static const NWPageGen nw_pages_tab[] = {\n");
        for (k, p) in pages.iter().enumerate() {
            let init_fn = if p.has_init && k != 0 { format!("p{}_nw_page_init", k) } else { "0".to_string() };
            nav_c.push_str(&format!(
                "    {{ \"{}\", p{}_nw_app_build_dom, p{}_nw_app_restyle, {}, p{}_nw_page_root, p{}_nw_title, p{}_nw_nws_one, p{}_nw_segs_tab, {} }},\n",
                p.name, k, k, init_fn, k, k, k, k, p.ui.n_variants));
        }
        nav_c.push_str("};\n");
        nav_c.push_str(&format!(
            "__attribute__((constructor)) static void nw_pages_ctor(void) {{ nw_nav_bind(nw_pages_tab, {}); }}\n",
            n_pages));
    }

    let c_src = format!("{}{}", c_src_prefix, nav_c);

    let out = output.unwrap_or_else(|| dir.join("app.exe"));
    let build_dir = out.parent().unwrap_or(Path::new(".")).join("build");
    if let Err(e) = std::fs::create_dir_all(&build_dir) {
        eprintln!("错误: 无法创建 {}: {}", build_dir.display(), e);
        std::process::exit(1);
    }
    let c_path = build_dir.join("app.nw.c");

    /* ===== 资源嵌入：.incbin 汇编 + 资源表（constructor 启动注册） ===== */
    res_paths.sort_unstable(); /* msvc+shim 构建 Vec::sort（driftsort）栈溢出；unstable 路径正常 */
    res_paths.dedup();
    let mut res_c = String::new();
    let mut res_s: Option<PathBuf> = None;
    if !res_paths.is_empty() {
        let mut s = String::from(".section .rodata\n");
        let mut tab = String::new();
        let mut decls = String::new();
        let mut total = 0u64;
        for (i, p) in res_paths.iter().enumerate() {
            let fp = dir.join(p);
            let len = std::fs::metadata(&fp).map(|m| m.len()).unwrap_or(0);
            total += len;
            /* .incbin 用绝对路径：gcc 的工作目录随调用方变化，相对路径不可靠 */
            s += &format!(".global nwres_{}\nnwres_{}:\n.incbin \"{}\"\n",
                i, i, fp.canonicalize().unwrap_or(fp.clone()).to_string_lossy().replace('\\', "/"));
            decls += &format!("extern const unsigned char nwres_{}[];\n", i);
            tab += &format!("    {{ \"{}\", nwres_{}, {}u }},\n", escape_lit(p), i, len);
        }
        let sp = build_dir.join("app.nw.res.s");
        if let Err(e) = std::fs::write(&sp, &s) {
            eprintln!("错误: 无法写入 {}: {}", sp.display(), e);
            std::process::exit(1);
        }
        res_s = Some(sp);
        /* @font-face：字体注册构造器（别名 → 嵌入数据；运行时映射真实家族名） */
        let mut font_ctor = String::new();
        for (name, fp) in &fonts {
            if let Some(i) = res_paths.iter().position(|p| p == fp) {
                let len = std::fs::metadata(dir.join(fp)).map(|m| m.len()).unwrap_or(0);
                font_ctor += &format!("    nw_font_register(L\"{}\", nwres_{}, {}u);\n", name, i, len);
            }
        }
        let font_c = if font_ctor.is_empty() { String::new() } else {
            format!("__attribute__((constructor)) static void nw_font_ctor(void) {{\n{}}}\n", font_ctor)
        };
        res_c = format!(
            "/* ===== 嵌入资源表（编译期扫描 HTML/CSS/JS 静态引用，共 {} 项 / {:.1} MB） ===== */\n{}static const NWRes nw_res_tab[] = {{\n{}}};\n__attribute__((constructor)) static void nw_res_ctor(void) {{\n    nw_img_register(nw_res_tab, (int)(sizeof(nw_res_tab)/sizeof(nw_res_tab[0])));\n}}\n{}",
            res_paths.len(), total as f64 / 1048576.0, decls, tab, font_c);
    }

    if let Err(e) = std::fs::write(&c_path, &(c_src + &res_c)) {
        eprintln!("错误: 无法写入 {}: {}", c_path.display(), e);
        std::process::exit(1);
    }

    let rt_dir = find_runtime(runtime);
    let layout = layout_lib.unwrap_or_else(|| find_layout());

    /* runtime .o 缓存：runtime 源很少改，每次全量 -O2 重编 3500+ 行是编译慢的大头之一。
     * 缓存目录 rt_dir/cache/，源 mtime 新于 .o 才重编 */
    let cache_dir = rt_dir.join("cache");
    let _ = std::fs::create_dir_all(&cache_dir);
    let mut rt_objs: Vec<std::path::PathBuf> = Vec::new();
    for src in ["runtime.c", "nw_ui.c", "nw_platform.c"] {
        let src_path = rt_dir.join(src);
        let obj_path = cache_dir.join(format!("{}.o", src));
        let need = match (std::fs::metadata(&obj_path), std::fs::metadata(&src_path)) {
            (Ok(o), Ok(s)) => {
                let ot = o.modified().unwrap_or(std::time::UNIX_EPOCH);
                let st = s.modified().unwrap_or(std::time::UNIX_EPOCH);
                st > ot
            }
            _ => true,
        };
        if need {
            let st = Command::new("gcc")
                .arg("-O2")
                .arg("-std=c11")
                .arg(format!("-I{}", rt_dir.display()))
                .arg("-c")
                .arg(&src_path)
                .arg("-o")
                .arg(&obj_path)
                .status();
            match st {
                Ok(s) if s.success() => {}
                Ok(s) => {
                    eprintln!("错误: gcc -c {} 退出码 {}", src, s.code().unwrap_or(-1));
                    std::process::exit(1);
                }
                Err(e) => {
                    eprintln!("错误: 无法启动 gcc（{}）", e);
                    std::process::exit(1);
                }
            }
        }
        rt_objs.push(obj_path);
    }

    let mut command = Command::new("gcc");
    let mut command = command
        .arg("-O1") /* 生成码是调用序列/switch，O1 编译快一档，运行无感 */
        .arg("-std=c11")
        .arg("-mwindows") /* GUI 子系统：不附命令行黑窗口；mingw CRT 兼容 main 入口 */
        .arg(format!("-I{}", rt_dir.display()))
        .arg(&c_path);
    for o in &rt_objs {
        command = command.arg(o);
    }
    let mut command = command
        .arg(&layout)
        .arg("-ld2d1").arg("-ldwrite")
        .arg("-lwindowscodecs") /* WIC 图片解码（系统组件） */
        .arg("-lgdi32").arg("-luser32").arg("-lole32").arg("-lcomdlg32")
        .arg("-lshell32") /* <a href="http..."> 外链 ShellExecute（系统浏览器） */
        .arg("-lntdll").arg("-lws2_32").arg("-ladvapi32").arg("-lbcrypt").arg("-luserenv")
        .arg("-lm");
    let status = if let Some(sp) = &res_s {
        command.arg(sp)
    } else {
        &mut command
    };
    let status = status.arg("-o").arg(&out).status();
    match status {
        Ok(s) if s.success() => {
            let size = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
            println!("编译成功: {}", out.display());
            println!("exe 大小: {:.1} KB | 中间 C: {}", size as f64 / 1024.0, c_path.display());
        }
        Ok(s) => {
            eprintln!("gcc 退出码: {}", s.code().unwrap_or(-1));
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("错误: 无法启动 gcc（{}）", e);
            std::process::exit(1);
        }
    }
}

fn collect_element_ids(el: &html::El, ids: &mut std::collections::HashSet<String>) {
    if let Some(id) = &el.id { ids.insert(id.clone()); }
    for c in &el.children {
        if let html::Child::El(e) = c { collect_element_ids(e, ids); }
    }
}

/* ===== 多页面支持（通用机制：a[href] 本地页面链接 → 编译期全站打包） ===== */

/// 收集页面内 <a href> 指向的本地 .html/.htm（相对本页目录解析为可读路径）。
/// 纯文件存在性判定，无任何页面特定逻辑（与 collect_img_srcs 同性质）。
fn collect_page_links(el: &html::El, page_dir: &Path, out: &mut Vec<PathBuf>) {
    if el.tag == "a" {
        if let Some((_, hv)) = el.attrs.iter().find(|(k, _)| k == "href") {
            let h = hv.trim();
            if !h.is_empty() && !h.contains("://") && !h.starts_with("//")
                && !h.starts_with('#') && !h.starts_with("data:") {
                let cand = page_dir.join(h);
                let is_html = cand.extension()
                    .map(|e| e == "html" || e == "htm").unwrap_or(false);
                if is_html && cand.is_file() {
                    out.push(cand);
                }
            }
        }
    }
    for c in &el.children {
        if let html::Child::El(e) = c { collect_page_links(e, page_dir, out); }
    }
}

/// 页面导航名 = 文件名去扩展名（小写）；重名（子目录同名页）回退带路径的相对名。
fn page_name(p: &Path, all: &[PathBuf], idx: usize) -> String {
    let stem = p.file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
    let dup = all.iter().enumerate().any(|(i, q)| i != idx &&
        q.file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default() == stem);
    if dup {
        p.with_extension("").to_string_lossy().replace('\\', "/").to_lowercase()
    } else {
        stem
    }
}

/// 脚本编译失败降级（浏览器语义：脚本错误该脚本不执行，页面照常渲染）。
/// 首次失败：丢弃页面脚本，仅保留内联事件属性处理函数（extra_js）重试，返回 true；
/// 再失败：整页脚本放弃（纯静态渲染），返回 false。
fn degrade(page: &str, why: &str, program_src: &mut String, extra_js: &str) -> bool {
    if *program_src != extra_js {
        eprintln!("警告: 页面 {} 脚本编译失败（{}）——丢弃页面脚本，仅保留内联事件处理重试", page, why);
        *program_src = extra_js.to_string();
        true
    } else {
        eprintln!("警告: 页面 {} 脚本编译失败（{}）——该页脚本整体跳过，页面纯静态渲染", page, why);
        *program_src = String::new();
        false
    }
}

/* ===== 资源扫描（编译期静态引用收集；无任何页面特定逻辑，纯文件存在性判定） ===== */

const IMG_EXTS: &[&str] = &["png", "jpg", "jpeg", "gif", "bmp", "dib", "ico", "tif", "tiff", "webp"];

fn is_img_ext(p: &str) -> bool {
    p.rsplit('.').next().map(|e| IMG_EXTS.contains(&e.to_ascii_lowercase().as_str())).unwrap_or(false)
}

fn escape_lit(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn is_font_ext(p: &str) -> bool {
    let l = p.to_ascii_lowercase();
    l.ends_with(".ttf") || l.ends_with(".otf")
}

/// @font-face 剥离 + 字体收集：把 `@font-face { font-family:"x"; src:url(...) }` 块从
/// CSS 中摘除（parse_css 不再告警），字体文件进嵌入资源清单，记录 (别名, 路径)。
/// 浏览器语义：@font-face 只是把字体文件与 CSS 内 font-family 别名绑定，非图片资源。
fn extract_font_faces(css: &str, base_rel: &str, dir: &Path, res_paths: &mut Vec<String>,
                      fonts: &mut Vec<(String, String)>) -> String {
    if !css.to_ascii_lowercase().contains("@font-face") { return css.to_string(); }
    let css = &css::strip_comments(css);
    let low = css.to_ascii_lowercase();
    let mut out = String::with_capacity(css.len());
    let mut i = 0usize;
    while let Some(p) = low[i..].find("@font-face") {
        let start = i + p;
        // 摘除范围：从 @font-face 到配对 '{...}' 结束
        let Some(brace) = low[start..].find('{') else { i = start; break };
        let body_start = start + brace + 1;
        let mut depth = 1usize;
        let mut j = body_start;
        let b = css.as_bytes();
        while j < b.len() && depth > 0 {
            match b[j] { b'{' => depth += 1, b'}' => depth -= 1, _ => {} }
            j += 1;
        }
        let block = &css[body_start..j.saturating_sub(1)];
        out.push_str(&css[i..start]);
        i = j;
        // 解析 font-family 别名
        let mut fam = String::new();
        for line in block.split(';') {
            let line = line.trim();
            if let Some(v) = line.strip_prefix("font-family") {
                let v = v.trim_start_matches(|c| c == ':');
                let v = v.trim().trim_matches('"').trim_matches('\'').trim();
                fam = v.to_string();
            }
        }
        if fam.is_empty() { continue; }
        // 解析 src: url(...)；取 url( 后第一个 ')'（src 行还可能跟 format("truetype")）；
        // 相对样式表目录；仅嵌 TTF/OTF（woff 系为压缩 sfnt，DWrite 内存流不解析）
        let low_b = block.to_ascii_lowercase();
        let Some(uoff) = low_b.find("url(") else { continue };
        let urest = &block[uoff + 4..];
        let Some(uend) = urest.find(')') else { continue };
        let u = urest[..uend].trim().trim_matches('"').trim_matches('\'').trim();
        if u.starts_with("data:") || u.contains("://") || u.starts_with('/') {
            eprintln!("警告: @font-face {} 的字体 URL 无法打包，已跳过", fam);
            continue;
        }
        let full = uigen::norm_rel(&format!("{}/{}", base_rel, u));
        if !dir.join(&full).is_file() {
            eprintln!("警告: @font-face {} 字体文件不存在: {}", fam, full);
            continue;
        }
        if !is_font_ext(&full) {
            eprintln!("警告: @font-face {} 仅支持 TTF/OTF（{} 为压缩格式），已跳过", fam, full);
            continue;
        }
        if !fonts.iter().any(|(n, _)| *n == fam) {
            fonts.push((fam, full.clone()));
            res_paths.push(full);
        }
    }
    out.push_str(&css[i.min(css.len())..]);
    out
}

/// CSS 内 url() 路径改写：相对样式表所在目录（base_rel）→ 项目相对路径。
fn rewrite_css_urls(css: &str, base_rel: &str, dir: &Path, out: &mut Vec<String>) -> String {
    let mut res = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(p) = rest.find("url(") {
        let head = &rest[..p + 4];
        res.push_str(head);
        let tail = &rest[p + 4..];
        let end = match tail.find(')') { Some(e) => e, None => { res.push_str(tail); break; } };
        let inner = tail[..end].trim();
        let inner = inner.strip_prefix('"').and_then(|s| s.strip_suffix('"'))
            .or_else(|| inner.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
            .unwrap_or(inner);
        if inner.starts_with("data:") || inner.contains("://") || inner.starts_with('/') {
            res.push_str(&tail[..=end]);
        } else {
            let full = uigen::norm_rel(&format!("{}/{}", base_rel, inner));
            if is_img_ext(&full) && dir.join(&full).is_file() {
                out.push(full.clone());
                res.push_str(&format!("\"{}\")", escape_lit(&full)));
            } else {
                res.push_str(&tail[..=end]); /* 非图片（如字体）或不存在：原样保留 */
            }
        }
        rest = &tail[end + 1..];
    }
    res.push_str(rest);
    res
}

/// JS 字符串字面量扫描（迷你词法：跳过 // 与 /* */ 注释，捕获三种引号字面量）。
/// 字面量整体是路径、或内含 url('...') 的，指向项目内存在的图片才收集。
fn scan_js_assets(src: &str, dir: &Path, out: &mut Vec<String>) {
    let b = src.as_bytes();
    let mut i = 0usize;
    while i < b.len() {
        match b[i] {
            b'/' if i + 1 < b.len() && b[i + 1] == b'/' => {
                while i < b.len() && b[i] != b'\n' { i += 1; }
            }
            b'/' if i + 1 < b.len() && b[i + 1] == b'*' => {
                i += 2;
                while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') { i += 1; }
                i += 2;
            }
            q @ (b'"' | b'\'' | b'`') => {
                i += 1;
                let start = i;
                while i < b.len() && b[i] != q {
                    if b[i] == b'\\' { i += 1; }
                    i += 1;
                }
                if i < b.len() {
                    try_collect_asset(&src[start..i], dir, out);
                    i += 1;
                }
            }
            _ => i += 1,
        }
    }
}

fn try_collect_asset(s: &str, dir: &Path, out: &mut Vec<String>) {
    let mut cands: Vec<&str> = Vec::new();
    if let Some(u) = extract_url_inner(s) { cands.push(u); }
    cands.push(s.trim());
    for c in cands {
        if c.is_empty() || c.contains("://") || c.starts_with('/') || c.contains("data:") { continue; }
        let p = uigen::norm_rel(c);
        if p.contains("..") || !is_img_ext(&p) { continue; }
        if dir.join(&p).is_file() { out.push(p); }
    }
}

fn extract_url_inner(s: &str) -> Option<&str> {
    let i = s.find("url(")? + 4;
    let rest = &s[i..];
    let e = rest.rfind(')')?;
    let inner = rest[..e].trim();
    let inner = inner.strip_prefix('"').and_then(|x| x.strip_suffix('"'))
        .or_else(|| inner.strip_prefix('\'').and_then(|x| x.strip_suffix('\'')))
        .unwrap_or(inner);
    Some(inner.trim())
}

fn collect_img_srcs(el: &html::El, dir: &Path, out: &mut Vec<String>) {
    if el.tag == "img" {
        if let Some((_, sv)) = el.attrs.iter().find(|(k, _)| k == "src") {
            if !sv.contains("://") && !sv.starts_with("data:") {
                let p = uigen::norm_rel(sv);
                if is_img_ext(&p) && dir.join(&p).is_file() { out.push(p); }
            }
        }
    }
    for c in &el.children {
        if let html::Child::El(e) = c { collect_img_srcs(e, dir, out); }
    }
}

fn compile_ts(input: PathBuf, output: Option<PathBuf>, runtime: Option<PathBuf>) {
    let src = match std::fs::read_to_string(&input) {
        Ok(s) => s,
        Err(e) => { eprintln!("错误: 无法读取 {}: {}", input.display(), e); std::process::exit(1); }
    };
    let prog = match parser::parse(&src) {
        Ok(p) => p,
        Err(e) => { report(&e.msg, e.line, e.col); exit1(); }
    };
    let empty = std::collections::HashSet::new();
    let checked = match check::check(&prog, &empty) {
        Ok(c) => c,
        Err(e) => { report(&e.msg, e.line, e.col); exit1(); }
    };
    codegen::build_ret_table(&checked);
    let c_src = match codegen::generate(&prog, &checked, false, "", 0, 0, "", true) {
        Ok(c) => c,
        Err(e) => { eprintln!("错误 [codegen]: {}", e.msg); std::process::exit(1); }
    };
    let out = output.unwrap_or_else(|| {
        let stem = input.file_stem().unwrap().to_string_lossy().to_string();
        input.with_file_name(format!("{}.exe", stem))
    });
    let build_dir = out.parent().unwrap_or(Path::new(".")).join("build");
    if let Err(e) = std::fs::create_dir_all(&build_dir) {
        eprintln!("错误: 无法创建 {}: {}", build_dir.display(), e);
        std::process::exit(1);
    }
    let stem = input.file_stem().unwrap().to_string_lossy().to_string();
    let c_path = build_dir.join(format!("{}.nw.c", stem));
    if let Err(e) = std::fs::write(&c_path, &c_src) {
        eprintln!("错误: 无法写入 {}: {}", c_path.display(), e);
        std::process::exit(1);
    }
    let rt_dir = find_runtime(runtime);
    let status = Command::new("gcc")
        .arg("-O2")
        .arg("-std=c11")
        .arg(format!("-I{}", rt_dir.display()))
        .arg(&c_path)
        .arg(rt_dir.join("runtime.c"))
        .arg("-lm")
        .arg("-o")
        .arg(&out)
        .status();
    match status {
        Ok(s) if s.success() => {
            let size = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
            println!("编译成功: {}", out.display());
            println!("exe 大小: {:.1} KB | 中间 C: {}", size as f64 / 1024.0, c_path.display());
        }
        Ok(s) => {
            eprintln!("gcc 退出码: {}", s.code().unwrap_or(-1));
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("错误: 无法启动 gcc（{}）", e);
            std::process::exit(1);
        }
    }
}

fn find_runtime(runtime: Option<PathBuf>) -> PathBuf {
    runtime.or_else(|| {
        /* 沿 exe 祖先逐层探测（兼容 target/{release,x86_64-pc-windows-gnu/release} 等任意深度） */
        let exe = std::env::current_exe().ok()?;
        exe.parent()?.ancestors()
            .map(|p| p.join("runtime"))
            .find(|c| c.exists())
    }).unwrap_or_else(|| {
        eprintln!("错误: 找不到 runtime 目录（用 --runtime 指定）");
        std::process::exit(1);
    })
}

fn find_layout() -> PathBuf {
    if let Some(exe) = std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.to_path_buf())) {
        let cand = exe.ancestors()
            .map(|p| p.join("layout/target/x86_64-pc-windows-gnu/release/libnw_layout.a"))
            .find(|c| c.exists());
        if let Some(c) = cand { return c; }
    }
    eprintln!("错误: 找不到 libnw_layout.a（用 --layout 指定）");
    std::process::exit(1);
}

fn report(msg: &str, line: u32, col: u32) {
    eprintln!("错误 [{}:{}]: {}", line, col, msg);
}

fn exit1() -> ! {
    std::process::exit(1);
}
