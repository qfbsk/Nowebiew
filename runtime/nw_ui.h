/* Nowebiew M1 UI 运行时 —— 窗口 + Direct2D/DirectWrite + taffy 布局 */
#ifndef NW_UI_H
#define NW_UI_H

#include "nw_runtime.h"

#ifdef __cplusplus
extern "C" {
#endif

typedef struct NWNode NWNode;

/* ===== 布局桥（与 layout/src/lib.rs 的 NWLayoutStyle 严格对应，repr(C)） ===== */
typedef struct {
    int   display;        /* 0=flex 1=none */
    int   flex_direction; /* 0=row 1=column */
    int   justify;        /* 0=start 1=center 2=end 3=space-between 4=space-around 5=space-evenly */
    int   align;          /* 0=start 1=center 2=end 3=stretch 4=baseline */
    float gap;            /* px */
    float padding[4];     /* top right bottom left (px) */
    float margin[4];      /* top right bottom left (px) */
    int   width_kind;     /* 0=auto 1=px 2=percent */
    float width;
    int   height_kind;    /* 0=auto 1=px 2=percent */
    float height;
    int   flex_wrap;      /* 0=nowrap 1=wrap */
    float col_gap;        /* -1=用 gap；否则列向独立间隙（grid 子集用） */
    float row_gap;        /* -1=用 gap */
    int   pos_kind;       /* 0=流内 1=absolute 2=fixed（fixed 绘制不随滚动） */
    float inset[4];       /* top right bottom left */
    int   inset_kind[4];  /* 0=auto 1=px 2=percent */
    float flex_grow;      /* 0=不增长（默认） */
    int   min_w_kind;     /* 0=auto(min-content) 1=px 2=percent */
    float min_w;
    int   min_h_kind;
    float min_h;
    int   max_w_kind;     /* max-width：0=无 1=px 2=percent */
    float max_w;
    float aspect;         /* aspect-ratio（宽/高）；0=无。img 固有比例由布局默认注入 */
    int   margin_auto;    /* 位掩码：0=top 1=right 2=bottom 3=left 为 auto（水平 auto=居中） */
} NWLayoutStyle;

/* ===== DOM 构建（编译期生成的 C 代码调用） ===== */
NWNode* nw_dom_create(const char* tag, const char* id, const char* cls);
NWNode* nw_dom_text(const wchar_t* text);
void    nw_dom_append(NWNode* parent, NWNode* child);
void    nw_dom_set_text(NWNode* node, const wchar_t* text);

/* ===== 样式 setter（CSS 编译产物调用） ===== */
void nw_st_display(NWNode* n, int flex);          /* 1=flex 0=none */
void nw_st_flex_dir(NWNode* n, int column);       /* 1=column 0=row */
void nw_st_flex_wrap(NWNode* n, int wrap);
void nw_st_overflow(NWNode* n, int hidden); /* overflow:hidden 内容裁剪（0=visible 1=hidden） */        /* 1=wrap 0=nowrap */
void nw_st_justify(NWNode* n, int v);
void nw_st_align(NWNode* n, int v);
void nw_st_gap(NWNode* n, float px);
void nw_st_grid_gap(NWNode* n, float col, float row); /* grid 子集：列/行独立间隙 */
void nw_st_padding(NWNode* n, float t, float r, float b, float l);
void nw_st_margin(NWNode* n, float t, float r, float b, float l);
void nw_st_margin_auto(NWNode* n, int mask);      /* margin:auto 侧（浏览器块盒水平居中） */
void nw_st_width_px(NWNode* n, float v);
void nw_st_width_pct(NWNode* n, float v);
void nw_st_height_px(NWNode* n, float v);
void nw_st_height_pct(NWNode* n, float v);
void nw_st_width_auto(NWNode* n);   /* 清除显式宽度（kind=0），供守卫级联覆盖静态尺寸 */
void nw_st_height_auto(NWNode* n);  /* 同上（.open{height:auto} 类展开语义） */
void nw_st_bg(NWNode* n, float r, float g, float b, float a);
void nw_set_page_bg(float r, float g, float b, float a);  /* 整窗清屏底色（body 背景同步于此） */
void nw_st_radius(NWNode* n, float px);
void nw_st_color(NWNode* n, float r, float g, float b, float a);
void nw_st_font(NWNode* n, float size_px, int weight, const wchar_t* family);
void nw_st_text_align(NWNode* n, int v);          /* 0=left 1=center 2=right */
void nw_st_position(NWNode* n, int kind,          /* 1=absolute 2=fixed */
                    float t, int tk, float r, int rk, float b, int bk, float l, int lk);
void nw_st_z(NWNode* n, int z);
void nw_st_grow(NWNode* n, float g);
void nw_st_border_side(NWNode* n, int side, float w, /* side 0=top 1=right 2=bottom 3=left */
                       float r, float g, float b, float a);
void nw_st_border_color(NWNode* n, int side, float r, float g, float b, float a);
void nw_st_shadow(NWNode* n, float ox, float oy, float blur, float spread,
                  float r, float g, float b, float a);
void nw_st_min_w(NWNode* n, float px);            /* 覆盖 min-content 最小宽（flex 收缩解锁） */
void nw_st_max_w_px(NWNode* n, float v);          /* CSS max-width */
void nw_st_max_w_pct(NWNode* n, float v);
void nw_st_translate(NWNode* n, float dx, int kx, float dy, int ky); /* transform: translate（k:0无 1px 2% 11vw 12vh 13em） */
void nw_st_tshadow(NWNode* n, float dx, int ux, float dy, int uy, float r, float g, float b, float a); /* text-shadow */
void nw_st_lspace(NWNode* n, float v, int unit);  /* letter-spacing（unit:0px 1vw 2vh 3em） */
void nw_st_lineh(NWNode* n, float v, int unit);   /* line-height（unit:-1默认 0倍数 1px 2vw 3vh 4em） */
/* 真线性渐变（CSS linear-gradient 编译产物） */
void nw_st_gradient_begin(NWNode* n, float deg);  /* CSS 角度：0=向上 90=向右 */
void nw_st_gradient_stop(NWNode* n, float pos, float r, float g, float b, float a); /* pos 0..1 */
void nw_st_gradient_end(NWNode* n);

/* ===== 图片子系统（WIC 解码 + D2D 位图；资源在编译期嵌入 exe） ===== */
typedef struct {
    const char* path;             /* 项目相对路径（UTF-8，正斜杠，编译期归一） */
    const unsigned char* data;    /* 文件字节（.incbin 嵌入） */
    unsigned int len;
} NWRes;
void nw_img_register(const NWRes* tab, int n);    /* 生成代码在启动时注册资源表（constructor） */
void nw_el_img_src(NWNode* n, const wchar_t* path); /* <img src>（构建期）与 el.src 赋值（运行期） */
/* CSS background-image（url 已编译期解析为资源路径）；fit: 0=fill 1=cover 2=contain 3=固有尺寸 */
void nw_st_bgimg(NWNode* n, const wchar_t* path, int fit);

/* ===== 运行时主题（CSS 变量表 + 注册过的颜色声明重解析） ===== */
void nw_var_add(int theme, const char* name, const char* value); /* name 不带 -- 前缀也可 */
void nw_reg_var_decl(NWNode* n, const char* prop, const char* tmpl); /* tmpl 含 var() 的颜色声明 */
void nw_theme_set(int theme);  /* 切换主题并重解析全部注册声明 + relayout/重绘 */
int  nw_theme_count(void);     /* 已注册主题数（uigen 用于校验） */

/* ===== 运行时 class/style（JS DOM 增强；类名/值走宽字符） ===== */
void nw_cls_add(NWNode* n, const wchar_t* cls);
void nw_cls_remove(NWNode* n, const wchar_t* cls);
int  nw_cls_toggle(NWNode* n, const wchar_t* cls);   /* 返回切换后是否拥有 */
int  nw_cls_toggle_force(NWNode* n, const wchar_t* cls, int force); /* DOMTokenList.toggle(cls, force) */
int  nw_cls_contains(NWNode* n, const wchar_t* cls);
/* display 动态规则（classList 变更后重评估；媒体条件 0/1e9 = 无界；orient 0=无 1=landscape 2=portrait） */
void nw_reg_display_rule(NWNode* n, const char* classes, int spec, int display_flex,
                         float minw, float maxw, float minh, float maxh, int orient);
void nw_st_style_set(NWNode* n, const char* prop, const wchar_t* value); /* width/height/background/color */
void nw_st_reset(NWNode* n);                       /* 样式复位到初始态（级联重算第一步） */
void nw_st_color_inh(NWNode* n);                   /* 颜色运行时继承：取祖先链最近显式色 */
int  nw_cls_has_c(NWNode* n, const char* cls);     /* 类名存在（窄串） */
int  nw_node_attr_is(NWNode* n, const char* name, const char* val); /* 属性测试；val=NULL 存在性 */
int  nw_sel_match(NWNode* n, const char* spec);    /* 选择器规格串匹配（守卫块用） */
void nw_restyle_now(void);                         /* classList/:hover 变更后全量重算 */
NWNode* nw_dom_prev_elem(NWNode* n);               /* previousElementSibling */
NWNode* nw_dom_body(void);                         /* document.body：当前页根节点 */
void nw_state_reapply(void);                       /* 全量状态重放：行内声明 + display 规则 */
void nw_theme_replay(void);                        /* 当前主题 var() 重解析（不切主题） */
void nw_touch(NWNode* n);                          /* 样式变更后 relayout + 重绘 */
/* HTML 内联事件属性 onload="..."（生成代码在 DOM 构建后注册，首帧显示前触发） */
void nw_dom_on_load(void (*fn)(void));

/* ===== 事件 ===== */
void nw_dom_on_click(NWNode* n, void (*fn)(NWNode*));
/* 全局键盘回调（textarea 未持有焦点时）：vk = Win32 虚拟键码 */
void nw_dom_on_key(void (*fn)(int vk));
/* document 级点击（浏览器 click 冒泡语义）：每次点击都触发，
 * 参数为客户区坐标 + 命中节点（未命中任何节点时 target 为 NULL）。
 * 运行时不做任何过滤策略——点击按钮时元素 onclick 与本回调都会触发，
 * 是否忽略由页面脚本用 target 自行判断（等价浏览器 e.target.closest(...)） */
void nw_dom_on_doc_click(void (*fn)(double x, double y, NWNode* target));
/* 标准键盘事件：addEventListener("keydown", fn) 的 fn 编译为 (const wchar_t* key) 回调，
 * key 为浏览器 KeyboardEvent.key 命名（"ArrowRight"/" "/"Enter"/字母小写单字符...） */
void nw_dom_on_key_event(void (*fn)(const wchar_t* key));
/* window.innerWidth/innerHeight：当前客户区尺寸（运行时求值，随 resize 变化） */
double nw_win_w(void);
double nw_win_h(void);

/* ===== 类名查询与显隐切换（页面级切换：演示文稿/标签页等） ===== */
int  nw_query_count(const char* cls);              /* 拥有该 class 的元素节点数（文档序） */
void nw_query_show_only(const char* cls, int idx); /* 只显示第 idx 个，其余 display:none；负数=全部隐藏 */

/* ===== textarea 可编辑域（M2） ===== */
const wchar_t* nw_ta_get_text(NWNode* n);
void           nw_ta_set_text(NWNode* n, const wchar_t* text);

/* ===== 应用 ===== */
void nw_app_run(const wchar_t* title, int client_w, int client_h);
/* 视口尺寸预告知（DOM 构建前调用，供 vw/vh 求值） */
void nw_app_set_viewport(int client_w, int client_h);
/* 视口单位求值（生成的 clamp()/vw/vh 表达式调用） */
double nw_vw(double x); /* x 视口宽百分比 */
double nw_vh(double x); /* x 视口高百分比 */

/* ===== 运行时媒体查询（@media 响应式：视口/断点变化 → 级联重算，DOM 永生） ===== */
/* 视口段：语义 wlo < 宽 <= whi 且 hlo < 高 <= hhi（下界 -1e9 = 无下限，上界 1e9 = 无上限） */
typedef struct { float wlo, whi, hlo, hhi; int orient; } NWMediaSeg;

/* @font-face 字体注册（constructor 启动时调用；别名 → 嵌入 TTF/OTF 数据） */
void nw_font_register(const wchar_t* alias, const unsigned char* data, unsigned int len);
/* 绑定段表与重算回调；initial 为初始窗口所在段（生成的 main 在建树前调用） */
void nw_media_bind(const NWMediaSeg* segs, int n, int initial, void (*restyle)(int variant));

/* ===== 多页面导航（编译器全站打包；<a href> 的 UA 默认行为） =====
 * 浏览器语义：站点内多文档；点击 <a href> 切换页面，外链交系统浏览器。
 * AOT 模型：每页 DOM/样式/脚本编译期生成，运行时惰性构建 + 整树切换（DOM 永生——
 * 页面树驻留内存，切回即恢复，脚本状态与事件绑定保留）。 */
typedef struct NWPageGen {
    const char* name;               /* 页面名 = 文件名去 .html（小写；<a href> 按名匹配） */
    void (*build)(void);            /* DOM 构建（首次导航到该页时调用一次） */
    void (*restyle)(int variant);   /* 本页媒体段样式重算（页自带断点段表） */
    void (*init)(void);             /* 本页脚本顶层（首次激活执行一次；无脚本为 NULL） */
    NWNode* (*root)(void);          /* 本页根节点访问器（切页恢复 g_root 用） */
    const wchar_t* (*title)(void);  /* 本页 <title>（导航时更新窗口标题） */
    void (*nws_one)(int variant, int ordinal); /* 本页单节点样式重算（JS style.* 写入） */
    void* segs;                     /* NWMediaSeg 表（本页 CSS 的 @media 断点） */
    int   nsegs;
} NWPageGen;
/* 注册页面表（生成的 constructor 在 main 前调用）；页面 0 = 入口页（main 已构建） */
void nw_nav_bind(const NWPageGen* tab, int n);
void nw_nav_go(int idx);  /* 切页：隐藏旧树 → 构建/恢复新树 → 重算样式 → 重排重绘 */
int  nw_nav_cur(void);    /* 当前页序号（-1 = 未注册） */

/* ===== J2 标准库：列表 / 属性 / 存储 / 媒体 / 正则 / 触摸 / 锚点 ===== */
/* 元素列表（querySelectorAll 结果；原型阶段堆对象不回收） */
typedef struct { NWNode** items; int len; } NWList;
/* 字符串列表（正则 exec 的捕获组；items[0] = 整体匹配） */
typedef struct { NWStr* items; int len; } NWStrList;

NWList*  nw_query_all(const char* sel);          /* 选择器子集：#id / .class / tag，逗号任一 */
NWNode*  nw_list_at(NWList* l, int i);           /* 越界 → NULL */
void     nw_list_foreach(NWList* l, void (*fn)(NWNode* el, double i));
NWStr    nw_str_or(NWStr a, NWStr b);            /* JS a || b（字符串回退） */

/* 构建期属性登记（生成的 DOM 构建代码调用；value 为窄 UTF-8 字面量） */
void nw_el_attr(NWNode* n, const char* name, const char* value);
const wchar_t* nw_el_get_attr(NWNode* n, const wchar_t* name);
void nw_el_set_attr(NWNode* n, const wchar_t* name, const wchar_t* value);
NWStr nw_el_dataset_get(NWNode* n, const char* key); /* dataset.key → data-key 属性 */
int  nw_el_closest(NWNode* n, const char* sel);      /* self+祖先链选择器匹配 */
void nw_el_scroll_top(NWNode* n, float v);           /* 滚动系统未实现：登记的空实现 */
NWNode* nw_dom_body(void);

/* document.documentElement 属性（<html> 不建节点，文档级表）。
 * set("data-theme", name) 自动桥接到页面主题切换（主题名注册于生成代码） */
const wchar_t* nw_doc_get_attr(const wchar_t* name);
void nw_doc_set_attr(const wchar_t* name, const wchar_t* value);
void nw_dom_on_theme_name(void (*fn)(const wchar_t*)); /* 生成代码注册主题名切换 */

/* localStorage（进程内存键值；键为窄字面量，值宽字符） */
NWStr nw_ls_get(const char* key);
void  nw_ls_set(const char* key, const wchar_t* value);

/* matchMedia 子集：(prefers-color-scheme: light|dark)（读系统设置）、
 * (min|max-width|height: Npx)；多条件 " and " 连接 */
int nw_media_matches(const char* query);

/* location.hash（页面内存取，无 URL 语义） */
NWStr nw_location_hash(void);
void  nw_location_set_hash(const wchar_t* h);

double nw_parse_int(const wchar_t* s); /* 十进制取前导整数 */

/* setTimeout：一次性定时器（返回定时器 id，负数 = 失败） */
double nw_timer_after(double ms, void (*fn)(void));

/* Date：new Date() = epoch 毫秒（本地时区取分量） */
double nw_now_ms(void);
double nw_date_hours(double ms);
double nw_date_minutes(double ms);
double nw_date_seconds(double ms);

/* 正则子集：^ $ \d \D \w \W \s \S . [类] 捕获组 () | * + ?（回溯 VM） */
NWStrList* nw_regex_exec(const char* pat, const wchar_t* text); /* 不匹配 → NULL */
int   nw_regex_test(const char* pat, const wchar_t* text);
NWStr nw_sl_group(NWStrList* m, int i);              /* 越界/未匹配 → 空串 */

/* 触摸事件：主触点 DOWN/UP → touchstart/touchend（坐标为客户区） */
void nw_dom_on_touch_start(void (*fn)(double x, double y, NWNode* target));
void nw_dom_on_touch_end(void (*fn)(double x, double y, NWNode* target));

#ifdef __cplusplus
}
#endif
#endif
