/* Nowebiew M1 UI 运行时 —— 窗口 + Direct2D/DirectWrite + taffy 布局桥
 * （2026-10-09 重建版：依据 nw_ui.h API 面 + 生成代码用法 + 架构记忆） */
#define UNICODE
#define _UNICODE
#define WIN32_LEAN_AND_MEAN
#include <initguid.h>  /* 先于 d2d1/dwrite：实例化 DEFINE_GUID（IID_IUnknown 等） */
#include <windows.h>
#include <d2d1.h>
#include <dwrite_1.h>
#include <wincodec.h>
#include <shellapi.h>
#include <wchar.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <math.h>
#include <time.h>

#include "nw_runtime.h"
#include "nw_ui.h"
#include "nw_platform.h"

/* ===== 调试输出（NW_DEBUG=1 启用） ===== */
static int g_dbg_on = -1;
static int dbg_on(void) {
    if (g_dbg_on < 0) g_dbg_on = getenv("NW_DEBUG") ? 1 : 0;
    return g_dbg_on;
}
#define dbg(...) do { if (dbg_on()) fprintf(stderr, __VA_ARGS__); } while (0)

/* ===== NWStr <-> 宽字符（M1：结果不回收，静态增长缓冲） ===== */
const wchar_t* nw_str_to_wide(NWStr s) {
    /* 环形缓冲（16 槽）：同一表达式的多个实参各自持独立转换结果。
     * 单 static 缓冲会让 f(to_wide(a), to_wide(b)) 两实参别名同一块内存
     * （后者覆盖前者）——documentElement 属性播种 value==name 的根源。
     * 消费方（doc_set_attr _wcsdup / dom_set_text memcpy / style_set 转窄）
     * 都立即拷贝，槽位轮转安全；JS 表达式嵌套深度远小于 16 */
    enum { SLOTS = 16 };
    static wchar_t* bufs[SLOTS] = { 0 };
    static size_t caps[SLOTS] = { 0 };
    static int idx = 0;
    wchar_t** buf = &bufs[idx];
    size_t* cap = &caps[idx];
    idx = (idx + 1) % SLOTS;
    int need = MultiByteToWideChar(CP_UTF8, 0, s.data ? s.data : "", (int)s.len, NULL, 0);
    if (need <= 0) need = 1;
    if ((size_t)need + 1 > *cap) {
        *cap = (size_t)need + 64;
        wchar_t* nb = (wchar_t*)realloc(*buf, *cap * sizeof(wchar_t));
        if (!nb) return L"";
        *buf = nb;
    }
    MultiByteToWideChar(CP_UTF8, 0, s.data ? s.data : "", (int)s.len, *buf, need);
    (*buf)[need] = 0;
    return *buf;
}

/* ===== 前置声明 ===== */
static void nw_relayout(void);
void nw_restyle_now(void);
static void media_restyle_now(void);
static void ensure_invalidate(void);
static void wcls_rebuild(NWNode* n);
void nw_dump_nodes(void);
/* layout 库 C ABI（libnw_layout.a，见 layout crate 导出） */
void*  nw_layout_new(void);
void   nw_layout_free(void* tree);
unsigned int nw_layout_node(void* tree, const NWLayoutStyle* s);
void   nw_layout_add_child(void* tree, unsigned int parent, unsigned int child);
void   nw_layout_set_style(void* tree, unsigned int id, const NWLayoutStyle* s);
void   nw_layout_run(void* tree, unsigned int root, float avail_w, float avail_h, int fit);
void   nw_layout_get(void* tree, unsigned int id, float* out);
static int nw_sys_prefers_dark(void);
static int css_color_parse(const char* s, float out[4]);
static NWNode* hit_test(float px, float py_raw);
static int res_find(const wchar_t* path);
static void nw_snapshot_bmp(const char* path);
static wchar_t* gather_selection(void);
static int  text_index_at(NWNode* n, float px, float py);
static NWNode* text_node_at(float px, float py);
static NWNode* ta_at(float px, float py);
static void sel_highlight(ID2D1RenderTarget* rt, NWNode* n, IDWriteTextLayout* l, float ox, float oy);
static void touch_fire(double x, double y, NWNode* target, int up);
static int border_clip_push(ID2D1RenderTarget* rt, NWNode* n, float rad, float sy);
static ID2D1Layer* g_layer_tmp;

/* ===== 全局窗口/渲染状态 ===== */
int g_restyle_probe = 0; /* NW_PROBE=1 时打印 body 守卫探针 */
static HWND g_hwnd = NULL;
static int g_cw = 800, g_ch = 600;          /* 客户区逻辑尺寸（DIP） */
static int g_vw_set = 0;                     /* nw_app_set_viewport 预告知过 */
static ID2D1Factory* g_d2f = NULL;
static ID2D1HwndRenderTarget* g_hrt = NULL;
static ID2D1RenderTarget* g_rt = NULL;       /* = g_hrt 或快照 DC RT */
static ID2D1SolidColorBrush* g_brush = NULL;
static IDWriteFactory* g_dw = NULL;
static IWICImagingFactory* g_wicf = NULL;
static void* g_tree = NULL;                  /* taffy 布局树（nw_layout_*） */
static LARGE_INTEGER g_t0;

static float g_page_bg[4] = { 1, 1, 1, 1 };  /* 整窗清屏底色（body 背景同步） */

/* ===== 滚动状态 ===== */
static float g_scroll_y = 0.0f, g_scroll_target = 0.0f;
static float g_content_h = 0.0f;

/* ===== 节点表（构建序；relayout/重放遍历用） ===== */
#define NW_NODES_CAP 8192
typedef struct NWNode NWNode;
struct NWNode {
    char* tag;                    /* 小写标签；文本节点 = "text" */
    char* id;
    char* cls;                    /* 空格分隔类名（窄，构建期） */
    wchar_t* wcls;                /* 宽类名缓存（JS classList 运行时变更后重建） */
    struct NWNode* parent;
    struct NWNode* first_child;
    struct NWNode* last_child;
    struct NWNode* next;
    struct NWNode* prev;
    wchar_t* text;
    int text_len;

    NWLayoutStyle st;             /* taffy 桥样式（repr(C) 传 Rust） */
    unsigned int taffy_id;

    /* 几何（relayout 输出） */
    float x, y, w, h;
    int eff_z, z;

    /* 字体/文本 */
    float font_size;
    int font_weight;
    wchar_t* font_family;
    int text_align;               /* 0=left 1=center 2=right */
    float color[4];
    int color_set;
    int color_inherited;          /* 色来自 color_inh 快照（JS 改祖先色时需重算传播） */
    int inline_color;             /* JS style.color 写入：永久压过级联（restyle 重放让位） */
    int inline_bg_res;            /* JS style.backgroundImage：reset 后恢复 + 级联让位 */
    int inline_bg_fit;
    float ls_v; int ls_unit;      /* letter-spacing 值+单位（0px 1vw 2vh 3em） */
    float lh_v; int lh_unit;      /* line-height（-1 默认 0倍数 1px 2vw 3vh 4em） */

    /* 盒装饰 */
    float bg[4]; int has_bg;
    float radius;
    float border_w[4]; float border_c[4][4]; int has_border;
    int has_shadow; float sh_ox, sh_oy, sh_blur, sh_spread, sh_c[4];
    int ts_on; float ts_dx, ts_dy; int ts_ux, ts_uy; float ts_c[4];
    int tr_kx, tr_ky; float tr_x, tr_y;   /* translate 值+单位（0无 1px 2% 11vw 12vh 13em） */
    int overflow_hidden;

    /* 渐变 */
    int grad_on; float grad_deg; float grad_stops[16][5]; int grad_n;

    /* 图片 */
    int img_res, bg_img_res;
    float img_w, img_h;
    int bg_fit;

    /* 文本测量缓存（get_layout） */
    float dl_nat_w, dl_nat_h;
    void* dlayout;
    float dl_len, dl_box_w, dl_size, dl_ls, dl_lh;
    int dl_weight, dl_align; wchar_t* dl_family;

    /* 事件 */
    void (*onclick)(NWNode*);
    void (*onkey)(int vk);        /* 未用（全局） */

    /* 状态 */
    int detached;                 /* relayout 时 display:none（切页/隐藏） */
    int detached_prev;            /* 切页前 detached 状态（DOM 永生恢复） */
    int manual_hidden;            /* nw_query_show_only / style.display 手动隐藏 */
    int h_auto;                   /* 高度是 relayout 自动写入（非 CSS 显式）——
                                     第二遍重测只准修正这类节点，否则 CSS
                                     height 被裸文本高度踩掉（themeBtn 竖排） */
    int no_scroll;                /* 位于 fixed/absolute 祖先子树内——不随页面
                                     滚动（浏览器语义；否则 wheel 滚 5px 时
                                     hud 按钮视觉漂移 + 命中区错位=乱走点不中） */
    int page_id;                  /* 所属页（多页导航） */
    int order;                    /* 文档序（= g_nodes 下标）——文本选择跨节点排序用 */
    int caret;                    /* textarea 光标（字符序） */
    int is_ta;                    /* textarea 标记 */
    int focused;

    /* 属性表（构建期窄键 + 运行期宽值） */
    struct NWAttr* attrs;

    /* 行盒（容器动态包裹；relayout 维护） */
    struct NWNode* lb_first;      /* 容器的第一个匿名行盒（链表经 next 串） */
    int lb_built;
    int flex_explicit;            /* uigen 显式发射过 flex_dir（块级容器默认 column 化的豁免标志） */
};

typedef struct NWAttr {
    char* name;                   /* 窄 */
    wchar_t* value;               /* 宽（运行时可改） */
    struct NWAttr* next;
} NWAttr;

static NWNode* g_hover_node = NULL;    /* :hover 当前节点 */
static int g_cur_page = -1;             /* 当前页（多页导航；nav 区维护） */
static NWNode* g_nodes[NW_NODES_CAP];
static int g_node_count = 0;
static NWNode* g_root = NULL;    /* 当前页根（body） */
static int g_zpass = 0;          /* 0=文档流 1=fixed/z>0 顶层 */
static int g_batch_restyle = 0;  /* >0=级联批量重放中：style_set/touch 只写样式不逐条 relayout
                                  * （浏览器语义：批量改 DOM 一次 reflow；否则 23 个 slide 的
                                  * classList 切换 × 每条声明全量布局 = 点击卡 4 秒布局风暴） */
static int g_hover_stale = 0;

/* 立根（惰性）：nw_app_run 之前 DOM 查询/操作 API 也可能被调用
 * （main 里 nw_app_run 先于一切用户脚本尾逻辑）——浏览器语义=DOM
 * 解析完成即可查询，不等首帧。实现见多页面导航段（依赖 g_pages）。 */
void nw_ensure_root(void);

static void node_register(NWNode* n) {
    n->order = g_node_count;   /* 文档序：g_nodes 按下标即文档序 */
    if (g_node_count < NW_NODES_CAP) g_nodes[g_node_count++] = n;
}

/* 行盒节点也注册进 g_nodes（dump/重放可见），page_id 跟随宿主 */

/* ===== CSS 变量与主题 ===== */
#define NW_THEMES 4
#define NW_VARS 64
typedef struct { char* name; char* value; } NWVar;
static NWVar g_vars[NW_THEMES][NW_VARS];
static int g_var_n[NW_THEMES];
static int g_theme = 0;

/* 主题重放登记：节点 + 属性 + 含 var() 模板 */
typedef struct NWVarDecl {
    NWNode* n; char* prop; char* tmpl;
    struct NWVarDecl* next;
} NWVarDecl;
static NWVarDecl* g_vardecls = NULL;

/* var() 替换（tmpl 中 --name 换成当前主题值；未知保留原样） */
static char* substitute_vars_c(const char* tmpl, int theme) {
    /* 简单线性扫描 var(--x) */
    size_t cap = strlen(tmpl) * 2 + 64;
    char* out = (char*)malloc(cap);
    size_t o = 0;
    const char* p = tmpl;
    while (*p) {
        if (p[0] == 'v' && p[1] == 'a' && p[2] == 'r' && p[3] == '(' && p[4] == '-') {
            const char* q = p + 5;
            char name[64]; size_t nl = 0;
            while (*q && *q != ')' && *q != ',' && nl < 63) name[nl++] = *q++;
            name[nl] = 0;
            /* 归一化：q 从第二个 '-' 起扫，name 恒带一条前缀 '-'（如 "-text"）；
             * 注册表存裸名 nw_var_add(0,"text",...)，不剥则查找恒失败 */
            if (nl >= 1 && name[0] == '-') {
                memmove(name, name + 1, nl);
                nl -= 1;
            }
            /* 跳到 ) 或 , 取回退值 */
            const char* fb = NULL; size_t fbl = 0;
            while (*q && *q != ')') {
                if (*q == ',') { q++; while (*q == ' ') q++; fb = q; }
                else q++;
            }
            if (fb) { const char* e = q; while (e > fb && (e[-1] == ' ')) e--; fbl = (size_t)(e - fb); }
            if (*q == ')') {
                /* 查变量表（name 可带或不带 -- 前缀） */
                const char* found = NULL;
                for (int i = 0; i < g_var_n[theme]; i++) {
                    const char* vn = g_vars[theme][i].name;
                    if (!strcmp(vn, name) || (!strncmp(vn, "--", 2) && !strcmp(vn + 2, name))) {
                        found = g_vars[theme][i].value; break;
                    }
                }
                if (found) {
                    size_t l = strlen(found);
                    if (o + l + 1 < cap) { memcpy(out + o, found, l); o += l; }
                    p = q + 1;
                    continue;
                } else if (fb) {
                    if (o + fbl + 1 < cap) { memcpy(out + o, fb, fbl); o += fbl; }
                    p = q + 1;
                    continue;
                }
            }
        }
        if (o + 1 < cap) out[o++] = *p;
        p++;
    }
    out[o] = 0;
    return out;
}

void nw_var_add(int theme, const char* name, const char* value) {
    if (theme < 0 || theme >= NW_THEMES) return;
    if (g_var_n[theme] < NW_VARS) {
        g_vars[theme][g_var_n[theme]].name = _strdup(name);
        g_vars[theme][g_var_n[theme]].value = _strdup(value);
        g_var_n[theme]++;
    }
}

static void vardecl_apply(NWVarDecl* d) {
    char* v = substitute_vars_c(d->tmpl, g_theme);
    wchar_t* wv = NULL;
    { int n = MultiByteToWideChar(CP_UTF8, 0, v, -1, NULL, 0);
      if (n > 0) { wv = (wchar_t*)malloc((size_t)n * sizeof(wchar_t));
                   MultiByteToWideChar(CP_UTF8, 0, v, -1, wv, n); } }
    if (wv) { nw_st_style_set(d->n, d->prop, wv); free(wv); }
    free(v);
}

void nw_reg_var_decl(NWNode* n, const char* prop, const char* tmpl) {
    NWVarDecl* d = (NWVarDecl*)malloc(sizeof(NWVarDecl));
    d->n = n; d->prop = _strdup(prop); d->tmpl = _strdup(tmpl);
    d->next = g_vardecls; g_vardecls = d;
}

void nw_theme_replay(void) {
    g_batch_restyle++;
    for (NWVarDecl* d = g_vardecls; d; d = d->next) vardecl_apply(d);
    g_batch_restyle--;
    nw_restyle_now();
}

void nw_theme_set(int theme) {
    if (theme < 0 || theme >= NW_THEMES) return;
    g_theme = theme;
    nw_theme_replay();
}

int nw_theme_count(void) { return 1; } /* uigen 校验用（编译期已查） */

/* ===== DOM 构建 ===== */
static void node_init_common(NWNode* n) {
    n->img_res = -1; n->bg_img_res = -1;
    n->inline_bg_res = -1;
    n->bg_fit = 0;
    n->lh_unit = -1;
    n->ls_unit = 0;
    n->page_id = g_cur_page; /* 惰性切页 build 的节点必须归属当前页（否则切回时旧页不隐藏→重影） */
    n->font_size = 16.0f;
    n->font_weight = 400;
    memset(n->color, 0, sizeof(n->color));
    memset(n->bg, 0, sizeof(n->bg));
    memset(n->border_w, 0, sizeof(n->border_w));
    memset(n->border_c, 0, sizeof(n->border_c));
    memset(n->grad_stops, 0, sizeof(n->grad_stops));
    n->st.col_gap = -1.0f; n->st.row_gap = -1.0f;
}

NWNode* nw_dom_create(const char* tag, const char* id, const char* cls) {
    NWNode* n = (NWNode*)calloc(1, sizeof(NWNode));
    n->tag = _strdup(tag);
    n->id = id ? _strdup(id) : NULL;
    n->cls = cls && *cls ? _strdup(cls) : NULL;
    wcls_rebuild(n); /* 宽类名缓存与窄串同步建立（否则首次 classList 变更会整体替换类串） */
    if (!g_tree) g_tree = nw_layout_new();
    n->taffy_id = nw_layout_node(g_tree, &n->st);
    node_init_common(n);
    node_register(n);
    return n;
}

NWNode* nw_dom_text(const wchar_t* text) {
    NWNode* n = (NWNode*)calloc(1, sizeof(NWNode));
    n->tag = _strdup("text");
    nw_dom_set_text(n, text);
    if (!g_tree) g_tree = nw_layout_new();
    n->taffy_id = nw_layout_node(g_tree, &n->st);
    node_init_common(n);
    node_register(n);
    return n;
}

void nw_dom_append(NWNode* parent, NWNode* child) {
    if (!parent || !child || child->parent) return;
    child->parent = parent;
    child->prev = parent->last_child;
    if (parent->last_child) parent->last_child->next = child;
    else parent->first_child = child;
    parent->last_child = child;
    nw_layout_add_child(g_tree, parent->taffy_id, child->taffy_id);
}

void nw_dom_set_text(NWNode* node, const wchar_t* text) {
    if (!node) return;
    /* 浏览器 textContent 语义：替换全部子节点。旧文本子节点不清理会与
     * node->text 双份叠印（secTag/themeBtn 文字重叠的根源） */
    for (NWNode* c = node->first_child; c; c = c->next) {
        if (c->detached) continue;
        c->detached = 1;
        if (c->tag && !strcmp(c->tag, "text") && g_tree) {
            c->st.display = 1; /* taffy 布局同步剔除（行盒/流内不残留） */
            nw_layout_set_style(g_tree, c->taffy_id, &c->st);
        }
    }
    free(node->text);
    size_t n = text ? wcslen(text) : 0;
    node->text = (wchar_t*)malloc((n + 1) * sizeof(wchar_t));
    if (n) memcpy(node->text, text, (n + 1) * sizeof(wchar_t));
    node->text_len = (int)n;
    nw_touch(node);
}

/* ===== 属性 ===== */
void nw_nav_click(NWNode* n); /* fwd：<a href> UA 默认行为（定义见导航区） */

void nw_el_attr(NWNode* n, const char* name, const char* value) {
    if (!n || !name) return;
    wchar_t* wv = NULL;
    if (value) {
        int wn = MultiByteToWideChar(CP_UTF8, 0, value, -1, NULL, 0);
        if (wn > 0) {
            wv = (wchar_t*)malloc((size_t)wn * sizeof(wchar_t));
            MultiByteToWideChar(CP_UTF8, 0, value, -1, wv, wn);
        }
    }
    for (NWAttr* a = n->attrs; a; a = a->next) {
        if (!strcmp(a->name, name)) { free(a->value); a->value = wv; return; }
    }
    NWAttr* a = (NWAttr*)malloc(sizeof(NWAttr));
    a->name = _strdup(name); a->value = wv; a->next = n->attrs;
    n->attrs = a;
    /* <a href> UA 默认行为自动接线（空 href 的 nav_click 自行忽略）；
     * 显式 onclick（JS 包装器）仍可后续经 nw_dom_on_click 覆盖 */
    if (n->tag && !strcmp(n->tag, "a") && !strcmp(name, "href") && !n->onclick)
        n->onclick = nw_nav_click;
}

static NWAttr* attr_find_w(NWNode* n, const wchar_t* name) {
    if (!n || !name) return NULL;
    /* 宽键转窄比较（属性名 ASCII 为主） */
    char u8[128];
    int nl = WideCharToMultiByte(CP_UTF8, 0, name, -1, u8, 128, NULL, NULL);
    if (nl <= 0) return NULL;
    for (NWAttr* a = n->attrs; a; a = a->next)
        if (!strcmp(a->name, u8)) return a;
    return NULL;
}

const wchar_t* nw_el_get_attr(NWNode* n, const wchar_t* name) {
    NWAttr* a = attr_find_w(n, name);
    return a ? a->value : NULL;
}

void nw_el_set_attr(NWNode* n, const wchar_t* name, const wchar_t* value) {
    if (!n || !name) return;
    char u8[128];
    int nl = WideCharToMultiByte(CP_UTF8, 0, name, -1, u8, 128, NULL, NULL);
    if (nl <= 0) return;
    wchar_t* wv = NULL;
    if (value) {
        size_t ln = wcslen(value);
        wv = (wchar_t*)malloc((ln + 1) * sizeof(wchar_t));
        memcpy(wv, value, (ln + 1) * sizeof(wchar_t));
    }
    for (NWAttr* a = n->attrs; a; a = a->next) {
        if (!strcmp(a->name, u8)) { free(a->value); a->value = wv; return; }
    }
    NWAttr* a = (NWAttr*)malloc(sizeof(NWAttr));
    a->name = _strdup(u8); a->value = wv; a->next = n->attrs;
    n->attrs = a;
    /* data-theme 桥（documentElement 级由 nw_doc_set_attr 处理） */
}

int nw_node_attr_is(NWNode* n, const char* name, const char* val) {
    if (!n || !name) return 0;
    for (NWAttr* a = n->attrs; a; a = a->next) {
        if (strcmp(a->name, name)) continue;
        if (!val) return 1; /* 存在性 */
        if (!a->value) return val[0] == 0;
        char u8[256];
        int nl = WideCharToMultiByte(CP_UTF8, 0, a->value, -1, u8, 256, NULL, NULL);
        return nl > 0 && !strcmp(u8, val);
    }
    return 0;
}

NWStr nw_el_dataset_get(NWNode* n, const char* key) {
    char full[96];
    snprintf(full, sizeof full, "data-%s", key);
    for (NWAttr* a = n ? n->attrs : NULL; a; a = a->next) {
        if (!strcmp(a->name, full) && a->value) {
            int nl = WideCharToMultiByte(CP_UTF8, 0, a->value, -1, NULL, 0, NULL, NULL);
            if (nl > 0) {
                char* u8 = (char*)malloc((size_t)nl);
                WideCharToMultiByte(CP_UTF8, 0, a->value, -1, u8, nl, NULL, NULL);
                NWStr r; r.len = (size_t)nl - 1; r.data = u8;
                return r;
            }
        }
    }
    return nw_slit("");
}

/* ===== 类名（运行时宽字符集合；构建期窄串缓存） ===== */
static void wcls_rebuild(NWNode* n) {
    free(n->wcls);
    if (n->cls && *n->cls) {
        int wn = MultiByteToWideChar(CP_UTF8, 0, n->cls, -1, NULL, 0);
        n->wcls = (wchar_t*)malloc((size_t)wn * sizeof(wchar_t));
        MultiByteToWideChar(CP_UTF8, 0, n->cls, -1, n->wcls, wn);
    } else {
        n->wcls = NULL;
    }
}

static int cls_list_has(const char* list, const char* cls) {
    /* 空格分隔类名集合包含判断 */
    size_t cl = strlen(cls);
    const char* p = list;
    while (*p) {
        while (*p == ' ') p++;
        const char* s = p;
        while (*p && *p != ' ') p++;
        if ((size_t)(p - s) == cl && !strncmp(s, cls, cl)) return 1;
    }
    return 0;
}

int nw_cls_has_c(NWNode* n, const char* cls) {
    return n && n->cls && cls_list_has(n->cls, cls);
}

static void wcls_sync(NWNode* n) {
    /* 运行时宽类名变更 → 回写窄串（守卫 nw_sel_match 用窄串） */
    free(n->cls);
    n->cls = NULL;
    if (n->wcls && *n->wcls) {
        int nl = WideCharToMultiByte(CP_UTF8, 0, n->wcls, -1, NULL, 0, NULL, NULL);
        if (nl > 0) {
            n->cls = (char*)malloc((size_t)nl);
            WideCharToMultiByte(CP_UTF8, 0, n->wcls, -1, n->cls, nl, NULL, NULL);
        }
    }
}

void nw_cls_add(NWNode* n, const wchar_t* cls) {
    if (!n || !cls || !*cls) return;
    if (!n->wcls && n->cls && *n->cls) wcls_rebuild(n); /* 惰性兜底 */
    if (n->wcls && wcsstr(n->wcls, cls)) {
        /* 粗查已有（词边界精确判断） */
        const wchar_t* p = n->wcls;
        size_t cl = wcslen(cls);
        while ((p = wcsstr(p, cls)) != NULL) {
            int bow = (p == n->wcls || p[-1] == ' ');
            int eow = (p[cl] == 0 || p[cl] == ' ');
            if (bow && eow) return;
            p++;
        }
    }
    size_t ol = n->wcls ? wcslen(n->wcls) : 0;
    size_t al = wcslen(cls);
    wchar_t* nw2 = (wchar_t*)malloc((ol + (ol ? 1 : 0) + al + 1) * sizeof(wchar_t));
    if (ol) { memcpy(nw2, n->wcls, ol * sizeof(wchar_t)); nw2[ol] = ' '; memcpy(nw2 + ol + 1, cls, (al + 1) * sizeof(wchar_t)); }
    else memcpy(nw2, cls, (al + 1) * sizeof(wchar_t));
    free(n->wcls); n->wcls = nw2;
    wcls_sync(n);
    media_restyle_now(); /* classList 变更 → 守卫/display 全量重算（J11 语义） */
}

static void cls_remove_w(NWNode* n, const wchar_t* cls) {
    if (!n || !cls || !*cls) return;
    if (!n->wcls) { /* 惰性兜底：窄串有类则建缓存，否则无类可删 */
        if (n->cls && *n->cls) wcls_rebuild(n); else return;
    }
    size_t cl = wcslen(cls);
    wchar_t* r = (wchar_t*)malloc((wcslen(n->wcls) + 1) * sizeof(wchar_t));
    size_t o = 0;
    const wchar_t* p = n->wcls;
    while (*p) {
        while (*p == ' ') p++;
        const wchar_t* s = p;
        while (*p && *p != ' ') p++;
        if ((size_t)(p - s) == cl && !wcsncmp(s, cls, cl)) continue; /* 跳过（删除） */
        if (p > s) {
            if (o) r[o++] = ' ';
            memcpy(r + o, s, (size_t)(p - s) * sizeof(wchar_t));
            o += (size_t)(p - s);
        }
    }
    r[o] = 0;
    free(n->wcls);
    n->wcls = (*r) ? r : NULL;
    if (!*r) free(r);
    wcls_sync(n);
}

void nw_cls_remove(NWNode* n, const wchar_t* cls) {
    cls_remove_w(n, cls);
    media_restyle_now(); /* classList 变更 → 全量重算 */
}

int nw_cls_toggle(NWNode* n, const wchar_t* cls) {
    /* contains ? remove : add（返回切换后是否拥有）；变更后全量重算 */
    if (nw_cls_contains(n, cls)) { cls_remove_w(n, cls); nw_restyle_now(); return 0; }
    nw_cls_add(n, cls); /* add 内含 restyle */
    return 1;
}

int nw_cls_toggle_force(NWNode* n, const wchar_t* cls, int force) {
    if (force) { nw_cls_add(n, cls); return 1; } /* add 内含 restyle */
    cls_remove_w(n, cls);
    nw_restyle_now();
    return 0;
}

int nw_cls_contains(NWNode* n, const wchar_t* cls) {
    if (!n || !cls || !*cls) return 0;
    if (!n->wcls && n->cls && *n->cls) wcls_rebuild(n); /* 惰性兜底 */
    if (!n->wcls) return 0;
    size_t cl = wcslen(cls);
    const wchar_t* p = n->wcls;
    while (*p) {
        while (*p == ' ') p++;
        const wchar_t* s = p;
        while (*p && *p != ' ') p++;
        if ((size_t)(p - s) == cl && !wcsncmp(s, cls, cl)) return 1;
    }
    return 0;
}

/* ===== 样式 setter（直传枚举值，禁止钳制） ===== */
void nw_st_display(NWNode* n, int flex) { if (n) n->st.display = flex ? 0 : 1; }
void nw_st_flex_dir(NWNode* n, int dir) {
    if (!n) return;
    n->st.flex_direction = dir; /* 直传枚举：0=row 1=column 2=row-reverse 3=column-reverse（禁止钳制！） */
    n->flex_explicit = 1; /* 显式发射：块级容器默认 column 化的豁免 */
}
void nw_st_flex_wrap(NWNode* n, int wrap) { if (n) n->st.flex_wrap = wrap ? 1 : 0; }
void nw_st_overflow(NWNode* n, int hidden) { if (n) n->overflow_hidden = hidden ? 1 : 0; }
void nw_st_justify(NWNode* n, int v) { if (n) n->st.justify = v; }
void nw_st_align(NWNode* n, int v) { if (n) n->st.align = v; }
void nw_st_gap(NWNode* n, float px) { if (n) n->st.gap = px; }
void nw_st_grid_gap(NWNode* n, float col, float row) {
    if (!n) return;
    n->st.col_gap = col; n->st.row_gap = row;
}
void nw_st_padding(NWNode* n, float t, float r, float b, float l) {
    if (!n) return;
    n->st.padding[0] = t; n->st.padding[1] = r; n->st.padding[2] = b; n->st.padding[3] = l;
}
void nw_st_margin(NWNode* n, float t, float r, float b, float l) {
    if (!n) return;
    n->st.margin[0] = t; n->st.margin[1] = r; n->st.margin[2] = b; n->st.margin[3] = l;
}
void nw_st_margin_auto(NWNode* n, int mask) { if (n) n->st.margin_auto = (unsigned char)mask; }
void nw_st_width_px(NWNode* n, float v) { if (n) { n->st.width_kind = 1; n->st.width = v; } }
void nw_st_width_pct(NWNode* n, float v) { if (n) { n->st.width_kind = 2; n->st.width = v; } }
void nw_st_height_px(NWNode* n, float v) { if (n) { n->st.height_kind = 1; n->st.height = v; n->h_auto = 0; } }
void nw_st_height_pct(NWNode* n, float v) { if (n) { n->st.height_kind = 2; n->st.height = v; n->h_auto = 0; } }
void nw_st_width_auto(NWNode* n) { if (n) n->st.width_kind = 0; }
void nw_st_height_auto(NWNode* n) { if (n) n->st.height_kind = 0; }
void nw_st_min_w(NWNode* n, float px) { if (n) { n->st.min_w_kind = 1; n->st.min_w = px; } }
void nw_st_max_w_px(NWNode* n, float v) { if (n) { n->st.max_w_kind = 1; n->st.max_w = v; } }
void nw_st_max_w_pct(NWNode* n, float v) { if (n) { n->st.max_w_kind = 2; n->st.max_w = v; } }
void nw_st_z(NWNode* n, int z) { if (n) { n->z = z; } }
void nw_st_grow(NWNode* n, float g) { if (n) n->st.flex_grow = g; }

void nw_st_bg(NWNode* n, float r, float g, float b, float a) {
    if (!n) return;
    n->bg[0] = r; n->bg[1] = g; n->bg[2] = b; n->bg[3] = a;
    n->has_bg = 1;
}

void nw_set_page_bg(float r, float g, float b, float a) {
    g_page_bg[0] = r; g_page_bg[1] = g; g_page_bg[2] = b; g_page_bg[3] = a;
    ensure_invalidate();
}

void nw_st_radius(NWNode* n, float px) { if (n) n->radius = px; }

void nw_st_color(NWNode* n, float r, float g, float b, float a) {
    if (!n) return;
    /* inline（JS style.color）永久压过级联：restyle 重放级联时不得覆盖。
     * dark()/light() 切主题走 style_set 路径（不经此函数）不受影响 */
    if (n->inline_color) return;
    n->color[0] = r; n->color[1] = g; n->color[2] = b; n->color[3] = a;
    n->color_set = 1;
    n->color_inherited = 0; /* 显式色：不参与继承重算 */
    /* 子孙继承在 relayout 前由 nw_st_color_inh 沿链处理 */
}

void nw_st_font(NWNode* n, float size_px, int weight, const wchar_t* family) {
    if (!n) return;
    /* 部分更新语义：编译器把 size/family/weight 拆成多条调用发射，
     * 0/NULL = 不改该槽位（绝不整体覆盖——曾把 250% 字号覆盖回 0） */
    if (size_px > 0) n->font_size = size_px;
    if (weight >= 100 && weight <= 900) n->font_weight = weight;
    if (family && *family) {
        free(n->font_family);
        n->font_family = _wcsdup(family);
    }
}

void nw_st_text_align(NWNode* n, int v) { if (n) n->text_align = v; }

void nw_st_position(NWNode* n, int kind,
                    float t, int tk, float r, int rk, float b, int bk, float l, int lk) {
    if (!n) return;
    n->st.pos_kind = kind;
    n->st.inset[0] = t; n->st.inset_kind[0] = tk;
    n->st.inset[1] = r; n->st.inset_kind[1] = rk;
    n->st.inset[2] = b; n->st.inset_kind[2] = bk;
    n->st.inset[3] = l; n->st.inset_kind[3] = lk;
}

void nw_st_border_side(NWNode* n, int side, float w, float r, float g, float b, float a) {
    if (!n || side < 0 || side > 3) return;
    n->border_w[side] = w;
    n->border_c[side][0] = r; n->border_c[side][1] = g;
    n->border_c[side][2] = b; n->border_c[side][3] = a;
    if (w > 0) n->has_border = 1;
}

void nw_st_border_color(NWNode* n, int side, float r, float g, float b, float a) {
    if (!n || side < 0 || side > 3) return;
    n->border_c[side][0] = r; n->border_c[side][1] = g;
    n->border_c[side][2] = b; n->border_c[side][3] = a;
    if (n->border_w[side] > 0) n->has_border = 1;
}

void nw_st_shadow(NWNode* n, float ox, float oy, float blur, float spread,
                  float r, float g, float b, float a) {
    if (!n) return;
    n->has_shadow = 1;
    n->sh_ox = ox; n->sh_oy = oy; n->sh_blur = blur; n->sh_spread = spread;
    n->sh_c[0] = r; n->sh_c[1] = g; n->sh_c[2] = b; n->sh_c[3] = a;
}

void nw_st_translate(NWNode* n, float dx, int kx, float dy, int ky) {
    if (!n) return;
    n->tr_x = dx; n->tr_kx = kx; n->tr_y = dy; n->tr_ky = ky;
}

void nw_st_tshadow(NWNode* n, float dx, int ux, float dy, int uy, float r, float g, float b, float a) {
    if (!n) return;
    n->ts_on = 1; n->ts_dx = dx; n->ts_ux = ux; n->ts_dy = dy; n->ts_uy = uy;
    n->ts_c[0] = r; n->ts_c[1] = g; n->ts_c[2] = b; n->ts_c[3] = a;
}

void nw_st_lspace(NWNode* n, float v, int unit) {
    if (!n) return;
    n->ls_v = v; n->ls_unit = unit;
}

void nw_st_lineh(NWNode* n, float v, int unit) {
    if (!n) return;
    n->lh_v = v; n->lh_unit = unit;
}

void nw_st_gradient_begin(NWNode* n, float deg) {
    if (!n) return;
    n->grad_on = 1; n->grad_deg = deg; n->grad_n = 0;
}

void nw_st_gradient_stop(NWNode* n, float pos, float r, float g, float b, float a) {
    if (!n || n->grad_n >= 16) return;
    n->grad_stops[n->grad_n][0] = pos;
    n->grad_stops[n->grad_n][1] = r;
    n->grad_stops[n->grad_n][2] = g;
    n->grad_stops[n->grad_n][3] = b;
    n->grad_stops[n->grad_n][4] = a;
    n->grad_n++;
}

void nw_st_gradient_end(NWNode* n) { if (n) n->grad_on = n->grad_n >= 2; }

/* ===== 样式复位（级联重算第一步：所有节点每段重放前调用） ===== */
void nw_st_reset(NWNode* n) {
    if (!n) return;
    NWLayoutStyle z;
    memset(&z, 0, sizeof(z));
    z.col_gap = -1.0f; z.row_gap = -1.0f;
    n->st = z;
    n->flex_explicit = 0;
    n->h_auto = 0;
    n->has_bg = 0;
    n->radius = 0;
    memset(n->border_w, 0, sizeof(n->border_w));
    memset(n->border_c, 0, sizeof(n->border_c));
    n->has_border = 0;
    n->has_shadow = 0;
    n->ts_on = 0;
    n->tr_kx = 0; n->tr_ky = 0; n->tr_x = 0; n->tr_y = 0;
    n->overflow_hidden = 0;
    n->grad_on = 0; n->grad_n = 0;
    /* img_res 是 DOM 内容属性（<img src>），严禁在样式 reset 里清——
     * restyle 重放不会重发 src，清了之后惰性 build 页的图片引用全灭
     * （固有尺寸丢失 → logo/内容图 0×0/塌缩）。只有 bg_img_res 是样式属性 */
    n->bg_img_res = -1; n->bg_fit = 0; /* 背景图是样式属性：级联重算后由规则重设 */
    /* inline（JS）声明永久压过级联：reset 后立即恢复——级联重放遇到 inline 标志会让位，
     * 若这里不恢复，JS 设的背景图/颜色在每次 restyle 后消失 */
    if (n->inline_bg_res >= 0) { n->bg_img_res = n->inline_bg_res; n->bg_fit = n->inline_bg_fit; }
    n->ls_v = 0; n->ls_unit = 0;
    n->lh_v = 0; n->lh_unit = -1;
    n->font_size = 16.0f; n->font_weight = 400;
    free(n->font_family); n->font_family = NULL;
    n->text_align = 0;
    if (!n->inline_color) { /* inline 色保留不动 */
        n->color_set = 0;
        n->color_inherited = 0;
        memset(n->color, 0, sizeof(n->color));
    }
    n->z = 0;
    /* onclick 保留：事件绑定属 DOM 非样式，restyle 重放不得清（否则二次点击失效） */
}

/* 颜色运行时继承：取祖先链最近显式色（级联继承近似） */
void nw_st_color_inh(NWNode* n) {
    if (!n || n->color_set) return;
    for (NWNode* p = n->parent; p; p = p->parent) {
        if (p->color_set) {
            memcpy(n->color, p->color, sizeof(n->color));
            n->color_set = 1;
            n->color_inherited = 1; /* 快照标记：祖先色变更时需重算 */
            return;
        }
    }
}

/* ===== 运行时 inline style（JS style.* 写入） ===== */
void nw_st_style_set(NWNode* n, const char* prop, const wchar_t* value) {
    if (!n || !prop || !value) return;
    /* 宽值转窄 */
    char u8[512];
    int nl = WideCharToMultiByte(CP_UTF8, 0, value, -1, u8, 512, NULL, NULL);
    if (nl <= 0) return;
    const char* val = u8;
    if (!strcmp(prop, "width") || !strcmp(prop, "height")) {
        int is_w = prop[0] == 'w';
        float px = 0; int kind = 0;
        if (!strcmp(val, "auto")) kind = 0;
        else {
            const char* e = strchr(val, 0);
            if (nl >= 3 && !strcmp(e - 3, "px)")) { /* 无此形态，占位 */ }
            char* endp = NULL;
            double num = strtod(val, &endp);
            if (endp && endp != val) {
                if (!strcmp(endp, "px")) { px = (float)num; kind = 1; }
                else if (!strcmp(endp, "%")) { px = (float)num; kind = 2; }
                else if (!strcmp(endp, "vw") && is_w) { px = (float)num * (float)g_cw / 100.0f; kind = 1; }
                else if (!strcmp(endp, "vh") && !is_w) { px = (float)num * (float)g_ch / 100.0f; kind = 1; }
                else if (!*endp) { px = (float)num; kind = 1; } /* 无单位数字按 px */
            }
        }
        if (is_w) { n->st.width_kind = kind; n->st.width = px; }
        else { n->st.height_kind = kind; n->st.height = px; }
    } else if (!strcmp(prop, "background") || !strcmp(prop, "background-color")) {
        float c[4] = { 0, 0, 0, 1 };
        if (css_color_parse(val, c)) { n->bg[0]=c[0]; n->bg[1]=c[1]; n->bg[2]=c[2]; n->bg[3]=c[3]; n->has_bg = 1; }
    } else if (!strcmp(prop, "color")) {
        float c[4] = { 0, 0, 0, 1 };
        if (css_color_parse(val, c)) {
            n->color[0]=c[0]; n->color[1]=c[1]; n->color[2]=c[2]; n->color[3]=c[3];
            n->color_set = 1;
            n->color_inherited = 0;
            n->inline_color = 1; /* 永久压过级联：restyle 重放 nw_st_color 时让位 */
            /* 继承传播（浏览器语义）：祖先色变更 → 所有依赖继承快照的文本节点重算。
             * dark()/light() 整树换色全走此路径；onload 一次性，O(N×深) 可接受 */
            for (int i = 0; i < g_node_count; i++) {
                NWNode* t = g_nodes[i];
                if (t == n || !t->color_inherited) continue;
                t->color_set = 0;
                nw_st_color_inh(t);
            }
        }
    } else if (!strcmp(prop, "background-image")) {
        /* url('...') → 资源表查路径 */
        const char* u = strstr(val, "url(");
        if (u) {
            u += 4;
            while (*u == '\'' || *u == '"' || *u == ' ') u++;
            char path[256]; size_t pl = 0;
            while (*u && *u != '\'' && *u != '"' && *u != ')' && pl < 255) path[pl++] = *u++;
            path[pl] = 0;
            /* 反斜杠归一正斜杠 */
            for (char* q = path; *q; q++) if (*q == '\\') *q = '/';
            wchar_t wpath[256];
            int wn = MultiByteToWideChar(CP_UTF8, 0, path, -1, wpath, 256);
            if (wn > 0) {
                /* JS 显式设置：必须总能写入并更新 inline 快照。
                 * 不能走 nw_st_bgimg——它的 inline 守卫是拦 CSS 级联重放的，
                 * 在这里会把第二次 JS 设置（dark→light 换 bk1）一并拦下，
                 * 然后 inline_bg_res 被更新成旧值——背景永久锁死在首图 */
                int r = res_find(wpath);
                if (r >= 0) {
                    n->bg_img_res = r;
                    n->bg_fit = 0;
                    n->inline_bg_res = r;
                    n->inline_bg_fit = 0;
                }
            }
        }
    } else if (!strcmp(prop, "box-shadow")) {
        /* "0 10px 34px rgba(...)" / "none"（主题重放路径：nw_st_style_set 是
         * var() 声明重放唯一入口，缺此分支=阴影永久锁死编译期主题值） */
        if (!strncmp(val, "none", 4)) {
            n->has_shadow = 0;
        } else {
            float nums[4] = { 0, 0, 0, 0 };
            int nn = 0;
            char colbuf[128] = "";
            int coll = 0;
            /* 括号感知空格切分，收尾 push（铁律） */
            char cur[128]; int cl = 0, depth = 0;
            for (int k = 0; ; k++) {
                char ch = val[k];
                if (ch == '(') depth++;
                else if (ch == ')') depth--;
                if ((ch == ' ' || ch == 0) && depth <= 0) {
                    if (cl > 0) {
                        cur[cl] = 0;
                        if ((cur[0] >= '0' && cur[0] <= '9') || cur[0] == '-' || cur[0] == '+' || cur[0] == '.') {
                            if (nn < 4) nums[nn++] = (float)strtod(cur, NULL);
                        } else if (coll + cl < 127) {
                            if (coll) colbuf[coll++] = ' ';
                            memcpy(colbuf + coll, cur, (size_t)cl + 1);
                            coll += cl;
                        }
                        cl = 0;
                    }
                    if (ch == 0) break;
                } else if (cl < 127) {
                    cur[cl++] = ch;
                }
            }
            float c[4] = { 0, 0, 0, 0.5f };
            if (colbuf[0]) css_color_parse(colbuf, c);
            n->has_shadow = 1;
            n->sh_ox = nums[0]; n->sh_oy = nums[1];
            n->sh_blur = nums[2]; n->sh_spread = nums[3];
            n->sh_c[0] = c[0]; n->sh_c[1] = c[1]; n->sh_c[2] = c[2]; n->sh_c[3] = c[3];
        }
    } else if (!strcmp(prop, "display")) {
        if (!strcmp(val, "none")) { n->manual_hidden = 1; }
        else { n->manual_hidden = 0; }
    }
    nw_touch(n);
}

/* ===== CSS 颜色解析（#hex / rgb() / rgba() / 常用命名色） ===== */
static int hexval(char c) {
    if (c >= '0' && c <= '9') return c - '0';
    if (c >= 'a' && c <= 'f') return c - 'a' + 10;
    if (c >= 'A' && c <= 'F') return c - 'A' + 10;
    return -1;
}

static int css_color_parse(const char* s, float out[4]) {
    while (*s == ' ') s++;
    out[0] = 0; out[1] = 0; out[2] = 0; out[3] = 1;
    if (*s == '#') {
        s++;
        int h[8]; int n = 0;
        while (n < 8 && s[n] && hexval(s[n]) >= 0) { h[n] = hexval(s[n]); n++; }
        if (n == 3) {
            for (int i = 0; i < 3; i++) out[i] = (h[i] * 17) / 255.0f;
            return 1;
        }
        if (n == 4) {
            for (int i = 0; i < 3; i++) out[i] = (h[i] * 17) / 255.0f;
            out[3] = (h[3] * 17) / 255.0f;
            return 1;
        }
        if (n == 6) {
            for (int i = 0; i < 3; i++) out[i] = (h[i*2] * 16 + h[i*2+1]) / 255.0f;
            return 1;
        }
        if (n == 8) {
            for (int i = 0; i < 3; i++) out[i] = (h[i*2] * 16 + h[i*2+1]) / 255.0f;
            out[3] = (h[6] * 16 + h[7]) / 255.0f;
            return 1;
        }
        return 0;
    }
    if (!strncmp(s, "rgb(", 4) || !strncmp(s, "rgba(", 5)) {
        int has_a = s[3] == 'a';
        s += has_a ? 5 : 4;
        float v[4] = { 0, 0, 0, 1 };
        int i = 0;
        char* endp;
        while (*s && i < 4) {
            double num = strtod(s, &endp);
            if (endp == s) break;
            int pcent = (*endp == '%');
            if (pcent) endp++;
            if (i == 3) {
                /* alpha 槽：CSS 语义 0-1 浮点（% 已是 0-100 → /100），绝不除 255 */
                double a = pcent ? num / 100.0 : num;
                v[i++] = (float)(a > 1 ? 1 : (a < 0 ? 0 : a));
            } else {
                double c = pcent ? num / 100.0 : num / 255.0;
                v[i++] = (float)(c > 1 ? 1 : (c < 0 ? 0 : c));
            }
            s = endp;
            while (*s == ' ') s++;
            if (*s == ',') s++;
            while (*s == ' ') s++;
        }
        if (i >= 3) { memcpy(out, v, sizeof(v)); return 1; }
        return 0;
    }
    /* 命名色（常用集） */
    static const struct { const char* n; float c[4]; } NAMED[] = {
        { "white",     { 1,1,1,1 } },       { "black",     { 0,0,0,1 } },
        { "red",       { 1,0,0,1 } },       { "green",     { 0,0.5f,0,1 } },
        { "lime",      { 0,1,0,1 } },       { "blue",      { 0,0,1,1 } },
        { "yellow",    { 1,1,0,1 } },       { "cyan",      { 0,1,1,1 } },
        { "aqua",      { 0,1,1,1 } },       { "magenta",   { 1,0,1,1 } },
        { "fuchsia",   { 1,0,1,1 } },       { "gray",      { 0.5f,0.5f,0.5f,1 } },
        { "grey",      { 0.5f,0.5f,0.5f,1 } }, { "silver", { 0.75f,0.75f,0.75f,1 } },
        { "orange",    { 1,0.647f,0,1 } },  { "purple",    { 0.5f,0,0.5f,1 } },
        { "pink",      { 1,0.753f,0.796f,1 } }, { "brown", { 0.647f,0.165f,0.165f,1 } },
        { "navy",      { 0,0,0.5f,1 } },    { "teal",      { 0,0.5f,0.5f,1 } },
        { "maroon",    { 0.5f,0,0,1 } },    { "olive",     { 0.5f,0.5f,0,1 } },
        { "transparent", { 0,0,0,0 } },
    };
    for (size_t k = 0; k < sizeof(NAMED)/sizeof(NAMED[0]); k++) {
        size_t l = strlen(NAMED[k].n);
        if (!strncmp(s, NAMED[k].n, l) && (s[l] == 0 || s[l] == ' ')) {
            memcpy(out, NAMED[k].c, sizeof(NAMED[k].c));
            return 1;
        }
    }
    /* rgba(255,255,255,.62) 这类浮点 alpha 已在 rgb() 分支覆盖 */
    return 0;
}

/* ===== display 动态规则（classList 变更后重评；构建期注册） =====
 * 动态数组：条目数随页面规模线性增长（tttt 23 页即 1935 条），
 * 固定 512 曾静默溢出——溢出后的节点 classList 切换永远不改 display */
typedef struct {
    NWNode* n;
    char classes[128];    /* 空格分隔 */
    int spec;
    int display_flex;     /* 匹配时 display=flex(0) 否则 none(1) */
    float minw, maxw, minh, maxh;
    int orient;           /* 0=无 1=landscape 2=portrait */
    int active;           /* 当前是否匹配（重放计算） */
} NWShow;
static NWShow* g_shows = NULL;
static int g_shows_n = 0;
static int g_shows_cap = 0;

void nw_reg_display_rule(NWNode* n, const char* classes, int spec, int display_flex,
                         float minw, float maxw, float minh, float maxh, int orient) {
    if (!n) return;
    if (g_shows_n >= g_shows_cap) {
        int ncap = g_shows_cap ? g_shows_cap * 2 : 512;
        NWShow* ns = (NWShow*)realloc(g_shows, (size_t)ncap * sizeof(NWShow));
        if (!ns) return; /* OOM：保底丢弃本条（旧语义） */
        g_shows = ns; g_shows_cap = ncap;
    }
    NWShow* s = &g_shows[g_shows_n++];
    s->n = n;
    snprintf(s->classes, sizeof s->classes, "%s", classes ? classes : "");
    s->spec = spec; s->display_flex = display_flex;
    s->minw = minw; s->maxw = maxw; s->minh = minh; s->maxh = maxh;
    s->orient = orient; s->active = -1;
}

/* 视口窗口匹配（min/max 语义：0/1e9 = 无界） */
static int media_window_match(float minw, float maxw, float minh, float maxh, int orient) {
    float w = (float)g_cw, h = (float)g_ch;
    if (minw > 0 && minw != -1e9f && !(w > minw)) return 0;
    if (maxw < 1e8f && maxw != -1e9f && !(w <= maxw)) return 0;
    if (minh > 0 && minh != -1e9f && !(h > minh)) return 0;
    if (maxh < 1e8f && maxh != -1e9f && !(h <= maxh)) return 0;
    if (orient == 1 && w < h) return 0;
    if (orient == 2 && w >= h) return 0;
    return 1;
}

/* 类名交集判断（运行时窄类串 vs 规则类串） */
static int show_matches(NWShow* s) {
    if (!media_window_match(s->minw, s->maxw, s->minh, s->maxh, s->orient)) return 0;
    if (s->spec < 0) return 1; /* 无条件规则（媒体条件已过） */
    /* 主 compound 类交集非空（放宽匹配：JS classList 变更后重评） */
    if (!s->classes[0]) return 0;
    /* compound 类 AND 匹配（浏览器语义）：规则串里的每个类节点都必须有。
     * 任一（OR）会让 .slide.active 命中无 active 的 slide → 所有幻灯片叠加显示。
     * classList 变更后的重评由 media_restyle_now 全量重放覆盖 */
    const char* p = s->classes;
    while (*p) {
        while (*p == ' ') p++;
        const char* st = p;
        while (*p && *p != ' ') p++;
        if (p > st) {
            char one[64];
            size_t l = (size_t)(p - st);
            if (l < 64) {
                memcpy(one, st, l); one[l] = 0;
                if (!nw_cls_has_c(s->n, one)) return 0;
            }
        }
    }
    return 1;
}

/* display 规则重放（classList/media 变更后；不改规则表）
 * 聚合语义：同一节点的多条规则按级联序（注册序）取最后一条命中者决定 display；
 * 全部不命中 → 不动（保持 reset/inline 后的值，浏览器级联语义） */
static void reapply_display(void) {
    int i, j;
    for (i = 0; i < g_shows_n; i++) {
        NWShow* s = &g_shows[i];
        s->active = s->n->detached ? 0 : show_matches(s);
    }
    for (i = 0; i < g_shows_n; i++) {
        NWShow* s = &g_shows[i];
        if (s->n->detached) continue;
        /* 该节点是否还有更后的条目：有则交给最后一条聚合写回 */
        int has_later = 0;
        for (j = i + 1; j < g_shows_n; j++)
            if (g_shows[j].n == s->n) { has_later = 1; break; }
        if (has_later) continue;
        /* s 是该节点级联序最后条目：取最后命中的 display_flex */
        int flex = -1;
        for (j = 0; j <= i; j++) {
            NWShow* t = &g_shows[j];
            if (t->n == s->n && t->active) flex = t->display_flex;
        }
        if (flex >= 0) s->n->st.display = flex ? 0 : 1;
    }
}

/* ===== 选择器规格串匹配（守卫块用） =====
 * 规格："t:tag,i:id,c:cls,h,a:name[=value]"，compound 内逗号分隔，Part 间 >> 后代 */
static int spec_compound_match(NWNode* n, char* compound);

int nw_sel_match(NWNode* n, const char* spec) {
    if (!n || !spec) return 0;
    /* Part 分割（" >> "） */
    char buf[512];
    snprintf(buf, sizeof buf, "%s", spec);
    char* parts[16]; int np = 0;
    char* p = buf;
    parts[np++] = p;
    while (*p && np < 16) {
        if (p[0] == '>' && p[1] == '>' && (p == buf || p[-1] == ' ')) {
            *p = 0; p += 2;
            while (*p == ' ') p++;
            parts[np++] = p;
        } else p++;
    }
    /* 自底向上：最后一 Part 匹配自身，其余逐级祖先 */
    NWNode* cur = n;
    for (int i = np - 1; i >= 0; i--) {
        while (cur && !spec_compound_match(cur, parts[i])) {
            if (i == np - 1) return 0; /* 主体必须匹配 */
            cur = cur->parent;
        }
        if (!cur) return 0;
        if (i > 0) cur = cur->parent;
    }
    return 1;
}

static int spec_compound_match(NWNode* n, char* compound) {
    /* compound 内按逗号分割简单条件 */
    char buf[256];
    snprintf(buf, sizeof buf, "%s", compound);
    char* save = NULL;
    char* tok = strtok_r(buf, ",", &save);
    int any_cond = 0;
    while (tok) {
        while (*tok == ' ') tok++;
        any_cond = 1;
        if (tok[0] == 't' && tok[1] == ':') {
            if (!n->tag || _stricmp(n->tag, tok + 2) != 0) return 0;
        } else if (tok[0] == 'i' && tok[1] == ':') {
            if (!n->id || strcmp(n->id, tok + 2) != 0) return 0;
        } else if (tok[0] == 'c' && tok[1] == ':') {
            if (!nw_cls_has_c(n, tok + 2)) return 0;
        } else if (tok[0] == 'h') {
            /* :hover 段：与全局 hover 节点比较 */
            if (g_hover_node != n) return 0;
        } else if (tok[0] == 'a' && tok[1] == ':') {
            const char* cond = tok + 2;
            const char* eq = strchr(cond, '=');
            if (eq) {
                char name[64];
                size_t l = (size_t)(eq - cond);
                if (l >= 64) return 0;
                memcpy(name, cond, l); name[l] = 0;
                if (!nw_node_attr_is(n, name, eq + 1)) return 0;
            } else {
                if (!nw_node_attr_is(n, cond, NULL)) return 0;
            }
        }
        tok = strtok_r(NULL, ",", &save);
    }
    (void)any_cond;
    return 1;
}

/* ===== 查询 ===== */
static int node_has_cls(NWNode* n, const char* cls) { return nw_cls_has_c(n, cls); }

static void collect_cls(NWNode* n, const char* cls, NWNode** out, int* cnt, int cap) {
    if (!n) return;
    if (node_has_cls(n, cls) && *cnt < cap) out[(*cnt)++] = n;
    for (NWNode* c = n->first_child; c; c = c->next) collect_cls(c, cls, out, cnt, cap);
}

int nw_query_count(const char* cls) {
    nw_ensure_root();
    if (!g_root) return 0;
    NWNode* tmp[NW_NODES_CAP];
    int cnt = 0;
    collect_cls(g_root, cls, tmp, &cnt, NW_NODES_CAP);
    return cnt;
}

void nw_query_show_only(const char* cls, int idx) {
    nw_ensure_root();
    if (!g_root) return;
    NWNode* list[NW_NODES_CAP];
    int cnt = 0;
    collect_cls(g_root, cls, list, &cnt, NW_NODES_CAP);
    for (int i = 0; i < cnt; i++) {
        if (i == idx) list[i]->manual_hidden = 0;
        else list[i]->manual_hidden = 1;
    }
    nw_touch(g_root);
}

/* querySelectorAll 子集：#id / .class / tag（逗号任一） */
NWList* nw_query_all(const char* sel) {
    NWList* l = (NWList*)malloc(sizeof(NWList));
    l->items = (NWNode**)malloc(sizeof(NWNode*) * NW_NODES_CAP);
    l->len = 0;
    nw_ensure_root(); /* main 在 nw_app_run 前调用时 g_root 尚未立（曾致 query 全空） */
    if (g_root && sel) {
        /* 简化：单选择器（第一段）；逗号后续段忽略（当前生成代码未用复合） */
        char buf[128];
        snprintf(buf, sizeof buf, "%s", sel);
        char* save = NULL;
        char* tok = strtok_r(buf, ",", &save);
        if (tok) {
            while (*tok == ' ') tok++;
            if (*tok == '#') {
                const char* id = tok + 1;
                for (int i = 0; i < g_node_count; i++)
                    if (g_nodes[i]->id && !strcmp(g_nodes[i]->id, id))
                        l->items[l->len++] = g_nodes[i];
            } else if (*tok == '.') {
                collect_cls(g_root, tok + 1, l->items, &l->len, NW_NODES_CAP);
            } else {
                for (int i = 0; i < g_node_count; i++)
                    if (g_nodes[i]->tag && !strcmp(g_nodes[i]->tag, tok) && g_nodes[i]->page_id == g_cur_page)
                        l->items[l->len++] = g_nodes[i];
            }
        }
    }
    return l;
}

NWNode* nw_list_at(NWList* l, int i) {
    if (!l || i < 0 || i >= l->len) return NULL;
    return l->items[i];
}

void nw_list_foreach(NWList* l, void (*fn)(NWNode* el, double i)) {
    if (!l || !fn) return;
    for (int i = 0; i < l->len; i++) fn(l->items[i], (double)i);
}

NWStr nw_str_or(NWStr a, NWStr b) {
    return (a.len && a.data && a.data[0]) ? a : b;
}

NWNode* nw_dom_prev_elem(NWNode* n) {
    if (!n) return NULL;
    NWNode* p = n->prev;
    while (p && !strcmp(p->tag, "text")) p = p->prev; /* 跳过文本节点 */
    return p;
}

/* ===== localStorage（进程内存键值） ===== */
#define NW_LS_CAP 128
static struct { char* key; wchar_t* val; } g_ls[NW_LS_CAP];
static int g_ls_n = 0;

NWStr nw_ls_get(const char* key) {
    for (int i = 0; i < g_ls_n; i++) {
        if (!strcmp(g_ls[i].key, key) && g_ls[i].val) {
            size_t l = wcslen(g_ls[i].val);
            char* u8 = (char*)malloc(l * 4 + 1);
            int nl = WideCharToMultiByte(CP_UTF8, 0, g_ls[i].val, -1, u8, (int)(l * 4 + 1), NULL, NULL);
            if (nl > 0) { NWStr r; r.len = (size_t)nl - 1; r.data = u8; return r; }
            free(u8);
        }
    }
    return nw_slit("");
}

void nw_ls_set(const char* key, const wchar_t* value) {
    for (int i = 0; i < g_ls_n; i++) {
        if (!strcmp(g_ls[i].key, key)) {
            free(g_ls[i].val);
            g_ls[i].val = value ? _wcsdup(value) : NULL;
            return;
        }
    }
    if (g_ls_n < NW_LS_CAP) {
        g_ls[g_ls_n].key = _strdup(key);
        g_ls[g_ls_n].val = value ? _wcsdup(value) : NULL;
        g_ls_n++;
    }
}

/* ===== matchMedia 子集 ===== */
int nw_media_matches(const char* query) {
    if (!query) return 0;
    char buf[256];
    snprintf(buf, sizeof buf, "%s", query);
    int ok = 1, any = 0;
    char* save = NULL;
    char* tok = strtok_r(buf, " ", &save);
    /* 简单逐 token："(prefers-color-scheme:"、"light)" / "dark)"、"(min-width:" Npx)" */
    float minw = -1e9f, maxw = -1e9f, minh = -1e9f, maxh = -1e9f;
    int want_dark = -1;
    char cur[64] = { 0 };
    while (tok) {
        strcat(cur, tok);
        if (strchr(tok, ')')) {
            /* 完成一个条件 */
            if (strstr(cur, "prefers-color-scheme")) {
                if (strstr(cur, "dark")) { if (want_dark < 0) want_dark = nw_sys_prefers_dark(); }
                else if (strstr(cur, "light")) { if (want_dark < 0) want_dark = !nw_sys_prefers_dark(); }
            } else if (strstr(cur, "min-width:")) {
                minw = (float)atof(strstr(cur, ":") + 1);
            } else if (strstr(cur, "max-width:")) {
                maxw = (float)atof(strstr(cur, ":") + 1);
            } else if (strstr(cur, "min-height:")) {
                minh = (float)atof(strstr(cur, ":") + 1);
            } else if (strstr(cur, "max-height:")) {
                maxh = (float)atof(strstr(cur, ":") + 1);
            }
            cur[0] = 0;
        }
        tok = strtok_r(NULL, " ", &save);
    }
    if (minw > -1e8f && !((float)g_cw > minw)) ok = 0;
    if (maxw > -1e8f && !((float)g_cw <= maxw)) ok = 0;
    if (minh > -1e8f && !((float)g_ch > minh)) ok = 0;
    if (maxh > -1e8f && !((float)g_ch <= maxh)) ok = 0;
    if (want_dark == 0) ok = 0;
    (void)any;
    return ok;
}

int nw_sys_prefers_dark(void) {
    /* 系统浅色/深色应用设置（HKCU ...Themes\Personalize AppsUseLightTheme） */
    DWORD v = 1, sz = sizeof(v);
    HKEY k;
    if (RegOpenKeyExA(HKEY_CURRENT_USER, "Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize",
                      0, KEY_READ, &k) == ERROR_SUCCESS) {
        RegQueryValueExA(k, "AppsUseLightTheme", NULL, NULL, (BYTE*)&v, &sz);
        RegCloseKey(k);
    }
    return v == 0;
}

/* ===== location.hash ===== */
static wchar_t g_hash[256] = { 0 };
NWStr nw_location_hash(void) {
    char u8[512];
    int nl = WideCharToMultiByte(CP_UTF8, 0, g_hash, -1, u8, 512, NULL, NULL);
    if (nl > 0) {
        char* d = (char*)malloc((size_t)nl);
        memcpy(d, u8, (size_t)nl);
        NWStr r; r.len = (size_t)nl - 1; r.data = d;
        return r;
    }
    return nw_slit("");
}
void nw_location_set_hash(const wchar_t* h) {
    wcsncpy(g_hash, h ? h : L"", 255);
    g_hash[255] = 0;
}

/* ===== document 属性（documentElement 级表） ===== */
#define NW_DOC_ATTRS 32
static struct { char* name; wchar_t* value; } g_doc_attrs[NW_DOC_ATTRS];
static int g_doc_attrs_n = 0;
static void (*g_theme_name_fn)(const wchar_t*) = NULL;

const wchar_t* nw_doc_get_attr(const wchar_t* name) {
    char u8[128];
    int nl = WideCharToMultiByte(CP_UTF8, 0, name, -1, u8, 128, NULL, NULL);
    if (nl > 0) {
        for (int i = 0; i < g_doc_attrs_n; i++)
            if (!strcmp(g_doc_attrs[i].name, u8)) return g_doc_attrs[i].value;
    }
    return NULL;
}

void nw_doc_set_attr(const wchar_t* name, const wchar_t* value) {
    char u8[128];
    int nl = WideCharToMultiByte(CP_UTF8, 0, name, -1, u8, 128, NULL, NULL);
    if (nl <= 0) return;
    for (int i = 0; i < g_doc_attrs_n; i++) {
        if (!strcmp(g_doc_attrs[i].name, u8)) {
            free(g_doc_attrs[i].value);
            g_doc_attrs[i].value = value ? _wcsdup(value) : NULL;
            /* data-theme 桥 */
            if (!strcmp(u8, "data-theme") && g_theme_name_fn && value) g_theme_name_fn(value);
            return;
        }
    }
    if (g_doc_attrs_n < NW_DOC_ATTRS) {
        g_doc_attrs[g_doc_attrs_n].name = _strdup(u8);
        g_doc_attrs[g_doc_attrs_n].value = value ? _wcsdup(value) : NULL;
        g_doc_attrs_n++;
    }
    if (!strcmp(u8, "data-theme") && g_theme_name_fn && value) g_theme_name_fn(value);
}

void nw_dom_on_theme_name(void (*fn)(const wchar_t*)) { g_theme_name_fn = fn; }

void nw_el_scroll_top(NWNode* n, float v) { (void)n; (void)v; } /* 滚动系统未实现：空实现 */

/* closest 复合匹配：CSS 迷你解析（tag / .cls / #id；支持 "a.c1" 组合） */
static int closest_compound_match(NWNode* n, const char* sel) {
    const char* p = sel;
    char tok[64];
    while (*p) {
        if (*p == '.') {
            p++; int l = 0;
            while (*p && *p != '.' && *p != '#') { if (l < 63) tok[l++] = *p; p++; }
            tok[l] = 0;
            if (l && !nw_cls_has_c(n, tok)) return 0;
        } else if (*p == '#') {
            p++; int l = 0;
            while (*p && *p != '.' && *p != '#') { if (l < 63) tok[l++] = *p; p++; }
            tok[l] = 0;
            if (l && (!n->id || strcmp(n->id, tok) != 0)) return 0;
        } else {
            int l = 0;
            while (*p && *p != '.' && *p != '#') { if (l < 63) tok[l++] = *p; p++; }
            tok[l] = 0;
            if (l && (!n->tag || _stricmp(n->tag, tok) != 0)) return 0;
        }
    }
    return 1;
}

int nw_el_closest(NWNode* n, const char* sel) {
    if (!n || !sel) return 0;
    /* CSS 语义：逗号 = OR；每支一个简单 compound（tag/.cls/#id）。
     * 此前直灌 nw_sel_match（t:/i:/c: 规格格式）——裸 tag 记号被静默忽略
     * → 复合恒真 → e.target.closest(...) 恒真 → 文档级点击永远提前返回 */
    for (NWNode* c = n; c; c = c->parent) {
        const char* p = sel;
        while (*p) {
            const char* q = strchr(p, ',');
            int l = q ? (int)(q - p) : (int)strlen(p);
            char alt[128];
            if (l >= 128) l = 127;
            memcpy(alt, p, l); alt[l] = 0;
            /* 去首尾空白："a, button" 按逗号切出 " button"——带空格的 tag
             * 永远匹配不上 → closest 恒假 → 点按钮时文档级点击照常触发 */
            char* s = alt;
            while (*s == ' ' || *s == '\t') s++;
            char* e = s + strlen(s);
            while (e > s && (e[-1] == ' ' || e[-1] == '\t')) e--;
            *e = 0;
            if (*s && closest_compound_match(c, s)) return 1;
            p = q ? q + 1 : p + l;
        }
    }
    return 0;
}

/* ===== 事件注册 ===== */
static void (*g_load_fn)(void) = NULL;
static void (*g_key_fn)(int vk) = NULL;
static void (*g_doc_click_fn)(double, double, NWNode*) = NULL;
static void (*g_key_event_fn)(const wchar_t*) = NULL;
static void (*g_touch_start_fn)(double, double, NWNode*) = NULL;
static void (*g_touch_end_fn)(double, double, NWNode*) = NULL;

void nw_dom_on_load(void (*fn)(void)) { g_load_fn = fn; }
void nw_dom_on_click(NWNode* n, void (*fn)(NWNode*)) { if (n) n->onclick = fn; }
void nw_dom_on_key(void (*fn)(int vk)) { g_key_fn = fn; }
void nw_dom_on_doc_click(void (*fn)(double x, double y, NWNode* target)) { g_doc_click_fn = fn; }
void nw_dom_on_key_event(void (*fn)(const wchar_t* key)) { g_key_event_fn = fn; }
void nw_dom_on_touch_start(void (*fn)(double x, double y, NWNode* target)) { g_touch_start_fn = fn; }
void nw_dom_on_touch_end(void (*fn)(double x, double y, NWNode* target)) { g_touch_end_fn = fn; }

double nw_win_w(void) { return (double)g_cw; }
double nw_win_h(void) { return (double)g_ch; }

/* ===== Date ===== */
/* epoch 语义 = UTC。必须 GetSystemTime（UTC）转 FILETIME——曾用 GetLocalTime
 * （本地时间）当 UTC 转 epoch，消费端 FileTimeToLocalFileTime 再 +8h，
 * 双重偏移：本地 22:04 显示成 06:04，主题按小时判断错乱（白天/夜晚颠倒） */
double nw_now_ms(void) {
    SYSTEMTIME st;
    GetSystemTime(&st);
    FILETIME ft;
    SystemTimeToFileTime(&st, &ft);
    ULARGE_INTEGER u;
    u.LowPart = ft.dwLowDateTime; u.HighPart = ft.dwHighDateTime;
    return (double)(u.QuadPart - 116444736000000000ULL) / 10000.0;
}

static void epoch_to_systime(double ms, SYSTEMTIME* st) {
    ULARGE_INTEGER u;
    u.QuadPart = (unsigned long long)(ms * 10000.0) + 116444736000000000ULL;
    FILETIME ft;
    ft.dwLowDateTime = u.LowPart; ft.dwHighDateTime = u.HighPart;
    FILETIME lft;
    FileTimeToLocalFileTime(&ft, &lft);
    SYSTEMTIME s;
    FileTimeToSystemTime(&lft, &s);
    *st = s;
}

double nw_date_hours(double ms)   { SYSTEMTIME s; epoch_to_systime(ms, &s); return s.wHour; }
double nw_date_minutes(double ms) { SYSTEMTIME s; epoch_to_systime(ms, &s); return s.wMinute; }
double nw_date_seconds(double ms) { SYSTEMTIME s; epoch_to_systime(ms, &s); return s.wSecond; }

double nw_parse_int(const wchar_t* s) {
    if (!s) return 0;
    while (*s == ' ') s++;
    return (double)wcstod(s, NULL);
}

/* ===== setTimeout / setInterval ===== */
#define NW_TIMER_MAX 128
#define NW_TIMER_ID_BASE 100
static void (*g_timer_fns[NW_TIMER_MAX])(void) = { NULL };
static int g_timer_slot = 0;

double nw_timer_after(double ms, void (*fn)(void)) {
    if (!g_hwnd || !fn) return -1;
    /* 找空槽（复用已触发槽位） */
    int slot = -1;
    for (int i = 0; i < NW_TIMER_MAX; i++) {
        int idx = (g_timer_slot + i) % NW_TIMER_MAX;
        if (!g_timer_fns[idx]) { slot = idx; break; }
    }
    if (slot < 0) return -1;
    g_timer_fns[slot] = fn;
    g_timer_slot = slot + 1;
    UINT id = (UINT)(NW_TIMER_ID_BASE + slot);
    if (SetTimer(g_hwnd, id, (UINT)(ms < 1.0 ? 1.0 : ms), NULL) == 0) { g_timer_fns[slot] = NULL; return -1; }
    return (double)id;
}

/* ===== 正则子集（回溯 VM：^ $ \d \D \w \W \s \S . [类] () 捕获 | * + ?） =====
 * 编译为节点序列；匹配从每个起点尝试（无 ^ 锚时），单遍线性 + 分支回溯。 */
typedef enum { R_CH, R_ANY, R_CLASS, R_GROUP, R_ALT, R_STAR, R_PLUS, R_OPT, R_BOL, R_EOL } RKind;
typedef struct RNode {
    RKind kind;
    wchar_t ch;              /* R_CH */
    int neg;                 /* R_CLASS 取反 */
    wchar_t set[64]; int set_n;
    struct RNode* kids[8]; int nkids;  /* group/alt 分支（alt: 2 个） */
    struct RNode* child;     /* star/plus/opt 重复体 */
    int capidx;              /* group 捕获序号 */
} RNode;

#define R_MAXNODES 256
typedef struct { RNode pool[R_MAXNODES]; int n; } RPool;
typedef struct { const wchar_t* s; int len; int pos; int (*caps)[2]; int ncaps; } RCtx;

static RNode* r_alloc(RPool* p) {
    if (p->n >= R_MAXNODES) return NULL;
    RNode* r = &p->pool[p->n++];
    memset(r, 0, sizeof(RNode));
    return r;
}

static const wchar_t* R_SPECIAL = L"\\^$.[]()|*+?";

/* 字符类匹配（\d \w \s 展开 + 字面集合） */
static int r_class_match(RNode* nd, wchar_t c) {
    int m = 0;
    for (int i = 0; i < nd->set_n; i++) {
        if (nd->set[i] == c) { m = 1; break; }
    }
    if (nd->neg) m = !m;
    return m;
}

static int r_match_seq(RNode** seq, int nseq, RCtx* ctx, int si);

static int r_match_one(RNode* nd, RCtx* ctx, int si) {
    switch (nd->kind) {
    case R_CH:   return (si < ctx->len && ctx->s[si] == nd->ch) ? si + 1 : -1;
    case R_ANY:  return (si < ctx->len && ctx->s[si] != L'\n') ? si + 1 : -1;
    case R_BOL:  return (si == 0) ? si : -1;
    case R_EOL:  return (si == ctx->len) ? si : -1;
    case R_CLASS: {
        if (si >= ctx->len) return -1;
        wchar_t c = ctx->s[si];
        int dm = (c >= L'0' && c <= L'9'), wm = (iswalnum(c) || c == L'_'), sm = iswspace(c);
        for (int i = 0; i < nd->set_n; i++) {
            wchar_t t = nd->set[i];
            if (t == L'D' && !dm) return si + 1;
            if (t == L'W' && !wm) return si + 1;
            if (t == L'S' && !sm) return si + 1;
        }
        return r_class_match(nd, c) ? si + 1 : -1;
    }
    case R_GROUP: {
        int end = r_match_seq(nd->kids, nd->nkids, ctx, si);
        if (end >= 0 && nd->capidx > 0 && nd->capidx < ctx->ncaps) {
            ctx->caps[nd->capidx][0] = si;
            ctx->caps[nd->capidx][1] = end;
        }
        return end;
    }
    case R_STAR: case R_OPT: {
        /* 贪心：尽量多，回溯退让 */
        int positions[64]; int np = 0;
        positions[np++] = si;
        if (nd->kind == R_STAR) {
            int cur = si;
            while (np < 64) {
                int nx = r_match_one(nd->child, ctx, cur);
                if (nx < 0 || nx == cur) break;
                positions[np++] = cur = nx;
            }
        }
        for (int k = np - 1; k >= 1; k--) {
            /* 前缀匹配 kid 一次（k 次重复后的起点→匹配剩余 seq 由调用者做，
             * 这里简化：STAR 用 seq 内循环） */
        }
        /* 回退处理交给调用者（seq 级），OPT = 0/1 次 */
        int end = r_match_one(nd->child, ctx, si);
        return end; /* 调用者需同时尝试跳过 */
    }
    case R_PLUS: {
        int e1 = r_match_one(nd->child, ctx, si);
        return e1; /* 至少一次由调用者循环保证 */
    }
    default: return -1;
    }
}

/* seq 匹配：支持 STAR/PLUS/OPT 的循环展开（简化回溯） */
static int r_match_seq(RNode** seq, int nseq, RCtx* ctx, int si) {
    if (nseq == 0) return si;
    RNode* nd = seq[0];
    if (nd->kind == R_STAR) {
        /* 0..n 次 child，然后匹配剩余 */
        int positions[128]; int np = 0;
        positions[np++] = si;
        int cur = si;
        while (np < 128) {
            int nx = r_match_one(nd->child, ctx, cur);
            if (nx < 0 || nx == cur) break;
            positions[np++] = cur = nx;
        }
        for (int k = np - 1; k >= 0; k--) {
            int r = r_match_seq(seq + 1, nseq - 1, ctx, positions[k]);
            if (r >= 0) return r;
        }
        return -1;
    }
    if (nd->kind == R_PLUS) {
        int positions[128]; int np = 0;
        int cur = si;
        while (np < 128) {
            int nx = r_match_one(nd->child, ctx, cur);
            if (nx < 0 || nx == cur) break;
            positions[np++] = cur = nx;
        }
        for (int k = np - 1; k >= 1; k--) { /* 至少 1 次 */
            int r = r_match_seq(seq + 1, nseq - 1, ctx, positions[k]);
            if (r >= 0) return r;
        }
        return -1;
    }
    if (nd->kind == R_OPT) {
        int e = r_match_one(nd->child, ctx, si);
        if (e >= 0) {
            int r = r_match_seq(seq + 1, nseq - 1, ctx, e);
            if (r >= 0) return r;
        }
        return r_match_seq(seq + 1, nseq - 1, ctx, si);
    }
    if (nd->kind == R_ALT) {
        int r = r_match_seq(nd->kids[0]->kids, nd->kids[0]->nkids, ctx, si);
        if (r < 0) {
            /* ALT 两个分支各自是子序列：尝试第二分支接剩余 seq */
            RNode* b1 = nd->kids[1];
            RNode* combined[64]; int cn = 0;
            for (int i = 0; i < b1->nkids && cn < 64; i++) combined[cn++] = b1->kids[i];
            for (int i = 1; i < nseq && cn < 64; i++) combined[cn++] = seq[i];
            r = r_match_seq(combined, cn, ctx, si);
        }
        return r;
    }
    int e = r_match_one(nd, ctx, si);
    if (e < 0) return -1;
    return r_match_seq(seq + 1, nseq - 1, ctx, e);
}

/* 解析器（递归下降；| 顶层 split） */
typedef struct { const wchar_t* p; RPool* pool; int ncaps; int err; } RParser;

static RNode* r_parse_alt(RParser* ps);

static void r_add_class_char(RNode* nd, wchar_t c) { if (nd->set_n < 64) nd->set[nd->set_n++] = c; }

static RNode* r_parse_atom(RParser* ps) {
    if (*ps->p == 0) return NULL;
    wchar_t c = *ps->p;
    if (c == L'(') {
        ps->p++;
        RNode* g = r_alloc(ps->pool);
        if (!g) { ps->err = 1; return NULL; }
        g->kind = R_GROUP;
        g->capidx = ++ps->ncaps;
        RNode* inner = r_parse_alt(ps);
        if (!inner || ps->err) { ps->err = 1; return NULL; }
        g->kids[0] = inner; g->nkids = inner->nkids > 0 ? inner->nkids : 1;
        /* inner 是 ALT/SEQ 容器：展开为一个 seq 的语义由 kids 结构保证 */
        if (*ps->p == L')') ps->p++;
        else { ps->err = 1; return NULL; }
        return g;
    }
    if (c == L'[') {
        ps->p++;
        RNode* cl = r_alloc(ps->pool);
        if (!cl) { ps->err = 1; return NULL; }
        cl->kind = R_CLASS;
        if (*ps->p == L'^') { cl->neg = 1; ps->p++; }
        while (*ps->p && *ps->p != L']') {
            if (*ps->p == L'\\' && ps->p[1]) {
                ps->p++;
                r_add_class_char(cl, *ps->p++);
            } else r_add_class_char(cl, *ps->p++);
        }
        if (*ps->p == L']') ps->p++;
        else { ps->err = 1; return NULL; }
        return cl;
    }
    if (c == L'.') { ps->p++; RNode* n = r_alloc(ps->pool); if (n) n->kind = R_ANY; return n; }
    if (c == L'^') { ps->p++; RNode* n = r_alloc(ps->pool); if (n) n->kind = R_BOL; return n; }
    if (c == L'$') { ps->p++; RNode* n = r_alloc(ps->pool); if (n) n->kind = R_EOL; return n; }
    if (c == L'\\' && ps->p[1]) {
        ps->p++;
        wchar_t e = *ps->p++;
        RNode* n = r_alloc(ps->pool);
        if (!n) { ps->err = 1; return NULL; }
        if (e == L'd' || e == L'D' || e == L'w' || e == L'W' || e == L's' || e == L'S') {
            n->kind = R_CLASS;
            r_add_class_char(n, e);
        } else {
            n->kind = R_CH; n->ch = e;
        }
        return n;
    }
    if (wcschr(R_SPECIAL, c) && c != L'(' && c != L'[') { ps->err = 1; return NULL; }
    ps->p++;
    RNode* n = r_alloc(ps->pool);
    if (!n) { ps->err = 1; return NULL; }
    n->kind = R_CH; n->ch = c;
    return n;
}

/* SEQ：atom 后跟量词，串接 */
static RNode* r_parse_seq(RParser* ps) {
    RNode* head[64]; int n = 0;
    while (*ps->p && *ps->p != L'|' && *ps->p != L')' && n < 64) {
        RNode* a = r_parse_atom(ps);
        if (!a || ps->err) return NULL;
        if (*ps->p == L'*' || *ps->p == L'+' || *ps->p == L'?') {
            RNode* w = r_alloc(ps->pool);
            if (!w) { ps->err = 1; return NULL; }
            w->kind = *ps->p == L'*' ? R_STAR : *ps->p == L'+' ? R_PLUS : R_OPT;
            w->child = a;
            ps->p++;
            if (*ps->p == L'?') ps->p++; /* 懒惰量词忽略（按贪心） */
            a = w;
        }
        head[n++] = a;
    }
    RNode* s = r_alloc(ps->pool);
    if (!s) { ps->err = 1; return NULL; }
    s->kind = R_GROUP; s->capidx = 0; /* 无捕获 seq 容器 */
    for (int i = 0; i < n && i < 8; i++) s->kids[i] = head[i];
    s->nkids = n;
    return s;
}

static RNode* r_parse_alt(RParser* ps) {
    RNode* a = r_parse_seq(ps);
    if (!a || ps->err) return a;
    if (*ps->p == L'|') {
        ps->p++;
        RNode* b = r_parse_alt(ps);
        if (!b || ps->err) { ps->err = 1; return NULL; }
        RNode* alt = r_alloc(ps->pool);
        if (!alt) { ps->err = 1; return NULL; }
        alt->kind = R_ALT;
        alt->kids[0] = a; alt->kids[1] = b; alt->nkids = 2;
        return alt;
    }
    return a;
}

/* 执行入口 */
NWStrList* nw_regex_exec(const char* pat, const wchar_t* text) {
    if (!pat || !text) return NULL;
    RPool pool; pool.n = 0;
    RParser ps = { text, &pool, 0, 0 };
    RNode* root = r_parse_alt(&ps);
    if (!root || ps.err || *ps.p != 0) return NULL;
    int len = (int)wcslen(text);
    int caps[16][2];
    for (int start = 0; start <= len; start++) {
        memset(caps, 0, sizeof(caps));
        RCtx ctx = { text, len, start, caps, 16 };
        /* root 是 GROUP(cap=0) 容器包 ALT/SEQ */
        RNode** seq = root->kids; int nseq = root->nkids;
        int end = r_match_seq(seq, nseq, &ctx, start);
        if (end >= 0) {
            NWStrList* l = (NWStrList*)malloc(sizeof(NWStrList));
            l->items = (NWStr*)malloc(sizeof(NWStr) * 16);
            l->len = 0;
            /* NWStr = UTF-8 窄串：宽捕获转窄（malloc，不回收——原型内存模型） */
            wchar_t wbuf[256];
            int wl = end - start; if (wl > 255) wl = 255;
            if (wl > 0) memcpy(wbuf, text + start, (size_t)wl * sizeof(wchar_t));
            wbuf[wl > 0 ? wl : 0] = 0;
            l->items[l->len++] = nw_wide_to_str(wbuf);
            for (int i = 1; i < ps.ncaps + 1 && l->len < 16; i++) {
                if (caps[i][1] > caps[i][0]) {
                    int cl = caps[i][1] - caps[i][0]; if (cl > 255) cl = 255;
                    wchar_t c2[257];
                    memcpy(c2, text + caps[i][0], (size_t)cl * sizeof(wchar_t));
                    c2[cl] = 0;
                    l->items[l->len++] = nw_wide_to_str(c2);
                } else {
                    l->items[l->len++] = nw_slit("");
                }
            }
            return l;
        }
    }
    return NULL;
}

int nw_regex_test(const char* pat, const wchar_t* text) {
    return nw_regex_exec(pat, text) != NULL;
}

NWStr nw_sl_group(NWStrList* m, int i) {
    NWStr e; e.len = 0; e.data = "";
    if (!m || i < 0 || i >= m->len) return e;
    return m->items[i];
}

/* ===== 触摸合成（WM 触摸消息到达时调用；沙箱测试用 NW_TOUCH 直接调） ===== */
static void touch_fire(double x, double y, NWNode* target, int up) {
    if (up) { if (g_touch_end_fn) g_touch_end_fn(x, y, target); }
    else    { if (g_touch_start_fn) g_touch_start_fn(x, y, target); }
}

/* ===== 图片子系统 ===== */
static const NWRes* g_resw = NULL;
static int g_res_n = 0;

void nw_img_register(const NWRes* tab, int n) { g_resw = tab; g_res_n = n; }

static int res_find(const wchar_t* path) {
    if (!g_resw || !path) return -1;
    /* 宽路径转窄（UTF-8，资源路径 ASCII 为主） */
    char u8[512];
    int nl = WideCharToMultiByte(CP_UTF8, 0, path, -1, u8, 512, NULL, NULL);
    if (nl <= 0) return -1;
    /* 反斜杠归一正斜杠 */
    for (char* q = u8; *q; q++) if (*q == '\\') *q = '/';
    for (int i = 0; i < g_res_n; i++) {
        if (g_resw[i].path && !strcmp(g_resw[i].path, u8)) return i;
    }
    return -1;
}

static IWICImagingFactory* wic_factory(void) {
    if (!g_wicf) {
        CoCreateInstance(&CLSID_WICImagingFactory, NULL, CLSCTX_INPROC_SERVER,
                         &IID_IWICImagingFactory, (void**)&g_wicf);
    }
    return g_wicf;
}

static int img_dims(int res, float* w, float* h) {
    *w = *h = 0;
    IWICImagingFactory* f = wic_factory();
    if (!f || !g_resw || res < 0 || res >= g_res_n) return 0;
    IWICStream* stream = NULL;
    IWICBitmapDecoder* dec = NULL;
    IWICBitmapFrameDecode* frame = NULL;
    UINT iw = 0, ih = 0;
    int ok = 0;
    HRESULT hrst = 0;
    if (SUCCEEDED(f->lpVtbl->CreateStream(f, &stream)) && stream) {
        if (SUCCEEDED(hrst = stream->lpVtbl->InitializeFromMemory(stream, (BYTE*)g_resw[res].data,
                                                           (DWORD)g_resw[res].len))
            && SUCCEEDED(hrst = f->lpVtbl->CreateDecoderFromStream(f, (IStream*)stream, NULL,
                                                            WICDecodeMetadataCacheOnDemand, &dec))
            && dec && SUCCEEDED(hrst = dec->lpVtbl->GetFrame(dec, 0, &frame)) && frame
            && SUCCEEDED(hrst = frame->lpVtbl->GetSize(frame, &iw, &ih))) {
            *w = (float)iw; *h = (float)ih; ok = 1;
        }
    }
    if (!ok) dbg("[nw] img_dims res=%d len=%u FAILED hr=0x%08lx\n", res, (unsigned)g_resw[res].len, (unsigned long)hrst);
    if (frame) frame->lpVtbl->Release(frame);
    if (dec) dec->lpVtbl->Release(dec);
    if (stream) stream->lpVtbl->Release(stream);
    return ok;
}

/* D2D 位图缓存：RT 重建后按代数失效（D2DERR_RECREATE_TARGET / Resize 不换代——
 * HwndRenderTarget Resize 保留位图，仅整体重建时换代）。
 * 按显示尺寸降采样解码：bucket = 目标像素宽向上取 128 的倍数（0 = 原始全尺寸），
 * 同图多尺寸各占一项，LRU 循环替换——4020x4452 原图解码 68MB，缩到显示宽后 <1MB */
typedef struct { int res; int gen; int bucket; float bw, bh; ID2D1Bitmap* bmp; } ImgBmp;
#define NW_IMGBMP_CAP 24
static ImgBmp g_bmpcache[NW_IMGBMP_CAP];
static int g_bmplist_n = 0;
static int g_rt_gen = 1;


static ID2D1Bitmap* img_bitmap(ID2D1RenderTarget* rt, int res, float* w, float* h,
                               float want_w, float want_h) {
    *w = *h = 0;
    int bucket = (want_w > 1.0f && want_h > 1.0f) ? ((int)(want_w + 127.0f) / 128) * 128 : 0;
    for (int i = 0; i < g_bmplist_n; i++) {
        if (g_bmpcache[i].res == res && g_bmpcache[i].gen == g_rt_gen
            && g_bmpcache[i].bucket == bucket) {
            *w = g_bmpcache[i].bw; *h = g_bmpcache[i].bh;
            return g_bmpcache[i].bmp;
        }
    }
    IWICImagingFactory* f = wic_factory();
    if (!f || !g_resw || res < 0 || res >= g_res_n) return NULL;
    IWICStream* stream = NULL;
    IWICBitmapDecoder* dec = NULL;
    IWICBitmapFrameDecode* frame = NULL;
    IWICBitmapScaler* scaler = NULL;      /* 存活至 CreateBitmap 之后（conv 惰性拉数据） */
    IWICFormatConverter* conv = NULL;
    ID2D1Bitmap* bmp = NULL;
    IWICBitmapSource* src = NULL;
    HRESULT hr = f->lpVtbl->CreateStream(f, &stream);
    int step = SUCCEEDED(hr) && stream ? 1 : 0;
    if (step) {
        hr = stream->lpVtbl->InitializeFromMemory(stream, (BYTE*)g_resw[res].data,
                                                  (DWORD)g_resw[res].len);
        if (SUCCEEDED(hr)) step = 2;
    }
    if (step == 2) {
        hr = f->lpVtbl->CreateDecoderFromStream(f, (IStream*)stream, NULL,
                                                WICDecodeMetadataCacheOnDemand, &dec);
        if (SUCCEEDED(hr) && dec) step = 3;
    }
    if (step == 3) {
        hr = dec->lpVtbl->GetFrame(dec, 0, &frame);
        if (SUCCEEDED(hr) && frame) { step = 4; src = (IWICBitmapSource*)frame; }
    }
    if (step == 4) {
        hr = f->lpVtbl->CreateFormatConverter(f, &conv);
        if (SUCCEEDED(hr) && conv) step = 5;
    }
    if (step == 5 && bucket > 0) {
        /* 降采样：目标像素 = 显示逻辑尺寸 x RT DPI/96；只缩不放，cap 原图 */
        UINT sw = 0, sh = 0;
        float dpix = 96.0f, dpiy = 96.0f;
        rt->lpVtbl->GetDpi(rt, &dpix, &dpiy);
        frame->lpVtbl->GetSize(frame, &sw, &sh);
        dbg("[nw] img res=%d src=%ux%u bucket=%d\n", res, (unsigned)sw, (unsigned)sh, bucket);
        if (sw > 0 && sh > 0) {
            float tw = (float)bucket * dpix / 96.0f;
            float th = tw * (float)sh / (float)sw;
            if (tw > (float)sw) { tw = (float)sw; th = (float)sh; }
            if (th < 1.0f) th = 1.0f;
            if (f->lpVtbl->CreateBitmapScaler(f, &scaler) == S_OK && scaler
                && SUCCEEDED(scaler->lpVtbl->Initialize(scaler, (IWICBitmapSource*)frame,
                                                        (UINT)tw, (UINT)th,
                                                        WICBitmapInterpolationModeFant))) {
                src = (IWICBitmapSource*)scaler;
            }
        }
    }
    if (step == 5) {
        hr = conv->lpVtbl->Initialize(conv, src,
                                      &GUID_WICPixelFormat32bppPBGRA, WICBitmapDitherTypeNone,
                                      NULL, 0.0, WICBitmapPaletteTypeCustom);
        if (SUCCEEDED(hr)) step = 6;
    }
    if (step == 6) {
        hr = rt->lpVtbl->CreateBitmapFromWicBitmap(rt, (IWICBitmapSource*)conv, NULL, &bmp);
        if (SUCCEEDED(hr) && bmp) {
            step = 7;
        } else {
            /* GDI 兼容目标（DC RT，快照通道）不支持 PBGRA：降级 32bppBGR 重试（源不变） */
            conv->lpVtbl->Initialize(conv, src,
                                     &GUID_WICPixelFormat32bppBGR, WICBitmapDitherTypeNone,
                                     NULL, 0.0, WICBitmapPaletteTypeCustom);
            hr = rt->lpVtbl->CreateBitmapFromWicBitmap(rt, (IWICBitmapSource*)conv, NULL, &bmp);
            if (SUCCEEDED(hr) && bmp) step = 7;
        }
    }
    if (step == 7) {
        /* 位图尺寸取自 WIC conv（可能已降采样）；D2D GetSize ABI 分歧绕开 */
        UINT bw = 0, bh2 = 0;
        ((IWICBitmapSource*)conv)->lpVtbl->GetSize((IWICBitmapSource*)conv, &bw, &bh2);
        *w = (float)bw; *h = (float)bh2;
        if (g_bmplist_n < NW_IMGBMP_CAP) {
            g_bmpcache[g_bmplist_n++] = (ImgBmp){ res, g_rt_gen, bucket, *w, *h, bmp };
        } else {
            static int evict = 0;
            g_bmpcache[evict % NW_IMGBMP_CAP].bmp->lpVtbl->Base.Base.Base.Release((IUnknown*)g_bmpcache[evict % NW_IMGBMP_CAP].bmp);
            g_bmpcache[evict % NW_IMGBMP_CAP] = (ImgBmp){ res, g_rt_gen, bucket, *w, *h, bmp };
            evict++;
        }
    } else {
        fprintf(stderr, "Nowebiew: 图片解码失败（资源 %d，步骤 %d，hr=0x%08lX）\n", res, step, (unsigned long)hr);
    }
    if (scaler) scaler->lpVtbl->Release(scaler);
    if (conv) conv->lpVtbl->Release(conv);
    if (frame) frame->lpVtbl->Release(frame);
    if (dec) dec->lpVtbl->Release(dec);
    if (stream) stream->lpVtbl->Release(stream);
    return bmp;
}

void nw_el_img_src(NWNode* n, const wchar_t* path) {
    if (!n || !path) return;
    int r = res_find(path);
    if (r < 0) { fprintf(stderr, "Nowebiew: 未嵌入的资源: %ls\n", path); return; }
    if (n->img_res != r) {
        n->img_res = r;
        img_dims(r, &n->img_w, &n->img_h); /* 固有尺寸（宽高 auto 时按替换元素语义） */
        nw_touch(n);
    }
}

void nw_st_bgimg(NWNode* n, const wchar_t* path, int fit) {
    if (!n || !path) return;
    int r = res_find(path);
    if (r < 0) { fprintf(stderr, "Nowebiew: 未嵌入的资源: %ls\n", path); return; }
    /* inline（JS style.backgroundImage）永久压过级联 */
    if (n->inline_bg_res >= 0) return;
    n->bg_img_res = r;
    n->bg_fit = fit;
}

/* 计算目标矩形：fit 0=fill 1=cover 2=contain 3=固有尺寸（居中） */
static D2D1_RECT_F img_fit_rect(float bw, float bh, float iw, float ih, int fit) {
    D2D1_RECT_F d = { 0, 0, bw, bh };
    if (iw <= 0 || ih <= 0) return d;
    if (fit == 0) return d;
    if (fit == 3) { d.right = d.left + iw; d.bottom = d.top + ih; return d; }
    float scale = (fit == 1) ? (bw / iw > bh / ih ? bw / iw : bh / ih)
                             : (bw / iw < bh / ih ? bw / iw : bh / ih);
    float dw = iw * scale, dh = ih * scale;
    d.left += (bw - dw) * 0.5f; d.top += (bh - dh) * 0.5f;
    d.right = d.left + dw; d.bottom = d.top + dh;
    return d;
}

/* 绘制：kind 0 = <img> 内容图（fill 铺满盒），1 = background-image（按 bg_fit） */
static void draw_img(ID2D1RenderTarget* rt, NWNode* n, float sy, int kind) {
    int res = kind ? n->bg_img_res : n->img_res;
    if (res < 0) return;
    float iw, ih;
    /* 降采样目标：盒子尺寸（cover 溢出部分裁剪，视觉差可忽略）；
     * fit=3（固有尺寸）传 0 → 全尺寸 */
    float want_w = (n->bg_fit == 3 && kind) ? 0.0f : n->w;
    float want_h = (n->bg_fit == 3 && kind) ? 0.0f : n->h;
    ID2D1Bitmap* bmp = img_bitmap(rt, res, &iw, &ih, want_w, want_h);
    if (!bmp) return;
    float rad = n->radius;
    if (rad > fminf(n->w, n->h) * 0.5f) rad = fminf(n->w, n->h) * 0.5f;
    D2D1_RECT_F box = { n->x, n->y + sy, n->x + n->w, n->y + sy + n->h };
    int clipped = 0;
    if (rad > 0.0f) clipped = border_clip_push(rt, n, rad, sy);
    D2D1_RECT_F d = kind ? img_fit_rect(n->w, n->h, iw, ih, n->bg_fit) : box;
    rt->lpVtbl->DrawBitmap(rt, bmp, &d, 1.0f, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, NULL);
    if (clipped) {
        /* border_clip_push 返回 2 = PushLayer+PushAxisAlignedClip 两层，须逆序全弹出 */
        rt->lpVtbl->PopAxisAlignedClip(rt);
        if (clipped == 2) {
            rt->lpVtbl->PopLayer(rt);
            if (g_layer_tmp) {
                ((IUnknownVtbl*)(g_layer_tmp)->lpVtbl)->Release((IUnknown*)(g_layer_tmp));
                g_layer_tmp = NULL;
            }
        }
    }
}

/* ===== DWrite 文本 ===== */
static const GUID NW_IID_IDWriteTextLayout1_ =
    { 0x9064d822, 0x80a7, 0x465c, { 0xa9, 0x86, 0xdf, 0x65, 0xf7, 0x8b, 0x8f, 0xeb } };
#define NW_IID_IDWriteTextLayout1 NW_IID_IDWriteTextLayout1_

/* letter-spacing / line-height 运行时值换算 */
static float ls_px(NWNode* n) {
    if (!n || n->ls_v == 0) return 0;
    switch (n->ls_unit) {
    case 1: return n->ls_v * (float)g_cw / 100.0f;
    case 2: return n->ls_v * (float)g_ch / 100.0f;
    case 3: return n->ls_v * n->font_size;
    default: return n->ls_v;
    }
}

static float lh_px(NWNode* n) {
    if (!n || n->lh_unit < 0) return 0;
    switch (n->lh_unit) {
    case 0: return n->lh_v * n->font_size;   /* 倍数 */
    case 1: return n->lh_v;
    case 2: return n->lh_v * (float)g_cw / 100.0f;
    case 3: return n->lh_v * (float)g_ch / 100.0f;
    case 4: return n->lh_v * n->font_size;   /* em 相对自身字号 */
    default: return 0;
    }
}

/* ===== @font-face 自定义字体（嵌入 TTF/OTF；别名 → 数据） ===== */
#define NW_FONTS_CAP 16
static struct { wchar_t alias[64]; const unsigned char* data; unsigned int len; } g_fonts[NW_FONTS_CAP];
static int g_fonts_n = 0;

void nw_font_register(const wchar_t* alias, const unsigned char* data, unsigned int len) {
    if (g_fonts_n < NW_FONTS_CAP) {
        wcsncpy(g_fonts[g_fonts_n].alias, alias ? alias : L"", 63);
        g_fonts[g_fonts_n].data = data;
        g_fonts[g_fonts_n].len = len;
        g_fonts_n++;
    }
}

/* 自定义字体集合 loader（DWrite）：铁律——loader 必须 RegisterFontFileLoader，
 * key 按 DWrite 字节复制（传「指向指针变量的地址」），COM QI 严格匹配 IID */
typedef struct NCLoaderVtbl {
    HRESULT(STDMETHODCALLTYPE* QueryInterface)(void*, const IID*, void**);
    ULONG(STDMETHODCALLTYPE* AddRef)(void*);
    ULONG(STDMETHODCALLTYPE* Release)(void*);
    HRESULT(STDMETHODCALLTYPE* CreateStreamFromKey)(void*, const void*, void**);
} NCLoaderVtbl;

typedef struct NCLoader { const NCLoaderVtbl* lpVtbl; LONG refs; } NCLoader;

/* 简易 IStream（内存只读）：MinGW C 模式 COM */
typedef struct NCStreamVtbl {
    HRESULT(STDMETHODCALLTYPE* QueryInterface)(void*, const IID*, void**);
    ULONG(STDMETHODCALLTYPE* AddRef)(void*);
    ULONG(STDMETHODCALLTYPE* Release)(void*);
    HRESULT(STDMETHODCALLTYPE* Read)(void*, void*, ULONG, ULONG*);
    HRESULT(STDMETHODCALLTYPE* Seek)(void*, LARGE_INTEGER, DWORD, ULARGE_INTEGER*);
    HRESULT(STDMETHODCALLTYPE* SetSize)(void*, ULARGE_INTEGER);
    HRESULT(STDMETHODCALLTYPE* CopyTo)(void*, void*, ULARGE_INTEGER, ULARGE_INTEGER*, ULARGE_INTEGER*);
    HRESULT(STDMETHODCALLTYPE* Commit)(void*, DWORD);
    HRESULT(STDMETHODCALLTYPE* Revert)(void*);
    HRESULT(STDMETHODCALLTYPE* LockRegion)(void*, ULARGE_INTEGER, ULARGE_INTEGER, DWORD);
    HRESULT(STDMETHODCALLTYPE* UnlockRegion)(void*, ULARGE_INTEGER, ULARGE_INTEGER, DWORD);
    HRESULT(STDMETHODCALLTYPE* Stat)(void*, void*, DWORD);
    HRESULT(STDMETHODCALLTYPE* Clone)(void*, void**);
} NCStreamVtbl;

typedef struct NCStream { const NCStreamVtbl* lpVtbl; LONG refs; const unsigned char* data; UINT len; UINT pos; } NCStream;

static HRESULT NCStream_QI(void* self, const IID* riid, void** out) {
    if (!memcmp(riid, &IID_IUnknown, sizeof(IID))) { *out = self; return S_OK; }
    return E_NOINTERFACE; /* 严格匹配：不认的 IID 一律拒绝（OOB vtbl 崩溃教训） */
}
static ULONG NCStream_AddRef(void* self) { NCStream* s = self; return ++s->refs; }
static ULONG NCStream_Release(void* self) {
    NCStream* s = self;
    ULONG r = --s->refs;
    if (r == 0) free(s);
    return r;
}
static HRESULT NCStream_Read(void* self, void* buf, ULONG n, ULONG* got) {
    NCStream* s = self;
    ULONG avail = s->len > s->pos ? s->len - s->pos : 0;
    ULONG take = n < avail ? n : avail;
    memcpy(buf, s->data + s->pos, take);
    s->pos += take;
    if (got) *got = take;
    return take == n ? S_OK : S_FALSE;
}
static HRESULT NCStream_Seek(void* self, LARGE_INTEGER d, DWORD origin, ULARGE_INTEGER* npos) {
    NCStream* s = self;
    long long np = d.QuadPart;
    if (origin == STREAM_SEEK_CUR) np += s->pos;
    else if (origin == STREAM_SEEK_END) np += s->len;
    if (np < 0) np = 0;
    if (np > (long long)s->len) np = s->len;
    s->pos = (UINT)np;
    if (npos) npos->QuadPart = (unsigned long long)s->pos;
    return S_OK;
}
static HRESULT NCStream_SetSize(void* self, ULARGE_INTEGER v) { (void)self;(void)v; return E_NOTIMPL; }
static HRESULT NCStream_CopyTo(void* self, void* st, ULARGE_INTEGER n, ULARGE_INTEGER* r1, ULARGE_INTEGER* r2) { (void)self;(void)st;(void)n;(void)r1;(void)r2; return E_NOTIMPL; }
static HRESULT NCStream_Commit(void* self, DWORD f) { (void)self;(void)f; return E_NOTIMPL; }
static HRESULT NCStream_Revert(void* self) { (void)self; return E_NOTIMPL; }
static HRESULT NCStream_Lock(void* self, ULARGE_INTEGER a, ULARGE_INTEGER b, DWORD f) { (void)self;(void)a;(void)b;(void)f; return E_NOTIMPL; }
static HRESULT NCStream_Stat(void* self, void* st, DWORD f) { (void)self;(void)st;(void)f; return E_NOTIMPL; }
static HRESULT NCStream_Clone(void* self, void** out) { (void)self;(void)out; return E_NOTIMPL; }

static const NCStreamVtbl g_ncstream_vtbl = {
    NCStream_QI, NCStream_AddRef, NCStream_Release,
    NCStream_Read, NCStream_Seek, NCStream_SetSize,
    NCStream_CopyTo, NCStream_Commit, NCStream_Revert,
    NCStream_Lock, NCStream_Lock,
    NCStream_Stat, NCStream_Clone
};

static HRESULT NCLoader_QI(void* self, const IID* riid, void** out) {
    NCLoader* l = self;
    if (!memcmp(riid, &IID_IUnknown, sizeof(IID))) { l->lpVtbl = (const NCLoaderVtbl*)&l->lpVtbl; *out = self; return S_OK; }
    return E_NOINTERFACE;
}
static ULONG NCLoader_AddRef(void* self) { NCLoader* l = self; return ++l->refs; }
static ULONG NCLoader_Release(void* self) {
    NCLoader* l = self;
    ULONG r = --l->refs;
    if (r == 0) free(l);
    return r;
}
static HRESULT NCLoader_CreateStreamFromKey(void* self, const void* key, void** out) {
    (void)self;
    /* key = 指向字体记录指针变量的地址：DWrite 字节复制语义（值 = 记录指针） */
    const unsigned char* data = NULL;
    unsigned int len = 0;
    if (key) {
        void* const* kp = (void* const*)key;
        /* key 值 = 记录指针，记录前 4 字节 len、其后数据（我们自己的打包格式） */
        const unsigned char* rec = (const unsigned char*)*kp;
        if (rec) {
            memcpy(&len, rec, 4);
            data = rec + 4;
        }
    }
    NCStream* s = (NCStream*)calloc(1, sizeof(NCStream));
    if (!s) return E_OUTOFMEMORY;
    s->lpVtbl = &g_ncstream_vtbl;
    s->refs = 1;
    s->data = data; s->len = len; s->pos = 0;
    *out = s;
    return S_OK;
}

static const NCLoaderVtbl g_ncloader_vtbl = {
    NCLoader_QI, NCLoader_AddRef, NCLoader_Release, NCLoader_CreateStreamFromKey
};

/* 字体记录打包：4 字节 len + 数据；注册时分配，key = &record_ptr（指针变量的地址） */
static unsigned char* g_font_records[NW_FONTS_CAP];
static void* g_font_keys[NW_FONTS_CAP];
static NCLoader* g_ncloader = NULL;
static int g_loader_registered = 0;

static void font_loader_init(IDWriteFactory* dw) {
    if (g_loader_registered || !dw) return;
    /* 打包字体记录（key 指向它们） */
    for (int i = 0; i < g_fonts_n; i++) {
        unsigned int len = g_fonts[i].len;
        g_font_records[i] = (unsigned char*)malloc(4 + len);
        if (g_font_records[i]) {
            memcpy(g_font_records[i], &len, 4);
            memcpy(g_font_records[i] + 4, g_fonts[i].data, len);
            g_font_keys[i] = &g_font_records[i]; /* 指向指针变量的地址（铁律） */
        }
    }
    g_ncloader = (NCLoader*)calloc(1, sizeof(NCLoader));
    g_ncloader->lpVtbl = &g_ncloader_vtbl;
    g_ncloader->refs = 1;
    if (SUCCEEDED(dw->lpVtbl->RegisterFontFileLoader(dw, (IDWriteFontFileLoader*)g_ncloader))) {
        g_loader_registered = 1;
    } else {
        dbg("[nw] custom font loader register failed\n");
    }
}

/* family 解析：@font-face 别名 → 自定义集合；通用族 → 系统映射 */
static const wchar_t* font_map(const wchar_t* family, IDWriteFontCollection** fcoll);

/* 系统族映射（浏览器默认字体栈近似） */
static const struct { const wchar_t* in; const wchar_t* out; } FAMMAP[] = {
    { L"sans-serif", L"Segoe UI" }, { L"serif", L"Georgia" },
    { L"monospace", L"Consolas" }, { L"cursive", L"Segoe Script" },
    { L"fantasy", L"Impact" }, { L"system-ui", L"Segoe UI" },
    { L"Arial", L"Arial" }, { L"微软雅黑", L"Microsoft YaHei" },
};

static const wchar_t* font_map(const wchar_t* family, IDWriteFontCollection** fcoll) {
    *fcoll = NULL;
    if (!family) return L"Segoe UI";
    /* 自定义字体别名 */
    for (int i = 0; i < g_fonts_n; i++) {
        if (!wcscmp(g_fonts[i].alias, family)) {
            /* 自定义集合（首帧惰性建；引用计数由 DWrite 管理） */
            static IDWriteFontCollection* custom_coll = NULL;
            if (!custom_coll && g_dw) {
                font_loader_init(g_dw);
                /* 逐个 CreateCustomFontFileReference + CreateFontFace → 集合简化：
                 * DWrite 自定义集合需要 collection loader，原型期用
                 * CreateCustomFontCollection 替代路径复杂——退化为
                 * AddFontMemResourceEx GDI 方案（系统级加载内存字体） */
                DWORD nfonts = 0;
                AddFontMemResourceEx((PVOID)g_fonts[i].data, g_fonts[i].len, NULL, &nfonts);
                return g_fonts[i].alias; /* GDI 加载后系统字体表可按名解析 */
            }
            return g_fonts[i].alias;
        }
    }
    for (size_t k = 0; k < sizeof(FAMMAP)/sizeof(FAMMAP[0]); k++) {
        if (!wcscmp(FAMMAP[k].in, family)) return FAMMAP[k].out;
    }
    return family;
}

/* DWrite factory 惰性创建：文本测量可能早于 nw_app_run（main 里 init 脚本的
 * classList restyle 先触发 relayout），没有 factory 时全部测量失败 →
 * shrink-to-fit 控件（按钮/hud）文字宽 0 塌缩竖排 */
static void ensure_dwrite(void) {
    if (g_dw) return;
    CoInitialize(NULL);
    void* dw = NULL;
    DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED, &IID_IDWriteFactory, (IUnknown**)&dw);
    g_dw = (IDWriteFactory*)dw;
    dbg("[nw] dwrite lazy-init %p\n", (void*)g_dw);
}

/* ===== 文本布局（DWrite；缓存键 = 内容/宽度/字号/字重/族/对齐/字距/行高） ===== */
static IDWriteTextLayout* get_layout(NWNode* n, float box_w) {
    IDWriteTextLayout* l = (IDWriteTextLayout*)n->dlayout;
    if (l && n->dl_len == n->text_len && n->dl_box_w == box_w
        && n->dl_size == n->font_size && n->dl_weight == n->font_weight
        && n->dl_family == n->font_family && n->dl_align == n->text_align
        && n->dl_ls == ls_px(n) && n->dl_lh == (n->lh_unit >= 0 ? lh_px(n) : 0.0f)) {
        return l;
    }
    if (l) { l->lpVtbl->Release(l); n->dlayout = NULL; }
    ensure_dwrite();
    if (!g_dw || !n->text || n->text_len == 0) {
        if (dbg_on()) fprintf(stderr, "[nw] gl early: g_dw=%p\n", (void*)g_dw);
        return NULL;
    }

    IDWriteTextFormat* fmt = NULL;
    IDWriteFontCollection* fcoll = NULL;
    const wchar_t* family = font_map(n->font_family ? n->font_family : L"Segoe UI", &fcoll);
    if (getenv("NW_NOFONT")) { family = n->font_family ? n->font_family : L"Segoe UI"; fcoll = NULL; }
    DWRITE_FONT_WEIGHT weight = (DWRITE_FONT_WEIGHT)n->font_weight;
    if (g_dw->lpVtbl->CreateTextFormat(g_dw, family, fcoll, weight,
            DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL,
            n->font_size, L"zh-cn", &fmt) != S_OK) {
        if (!fcoll) {
            if (dbg_on()) fprintf(stderr, "[nw] gl fmt fail: fam=%ls size=%.0f\n", family, n->font_size);
            return NULL;
        }
        /* 自定义字体创建失败（文件损坏等）：回退系统默认 */
        dbg("[nw] custom font %ls failed -> Segoe UI\n", family);
        if (g_dw->lpVtbl->CreateTextFormat(g_dw, L"Segoe UI", NULL, weight,
                DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL,
                n->font_size, L"zh-cn", &fmt) != S_OK) return NULL;
    }
    if (g_dw->lpVtbl->CreateTextLayout(g_dw, n->text, (UINT32)n->text_len, fmt,
            box_w, 10000.0f, &l) != S_OK) {
        fmt->lpVtbl->Release(fmt);
        return NULL;
    }
    fmt->lpVtbl->Release(fmt);
    DWRITE_TEXT_ALIGNMENT ta = n->text_align == 1 ? DWRITE_TEXT_ALIGNMENT_CENTER
                            : n->text_align == 2 ? DWRITE_TEXT_ALIGNMENT_TRAILING
                            : DWRITE_TEXT_ALIGNMENT_LEADING;
    l->lpVtbl->SetTextAlignment(l, ta);
    /* letter-spacing（DWrite1：layout1 QI 严格 IID；MinGW 头命名 SetCharacterSpacing） */
    {
        IDWriteTextLayout1* l1 = NULL;
        if (l->lpVtbl->QueryInterface(l, &NW_IID_IDWriteTextLayout1, (void**)&l1) == S_OK && l1) {
            float sp = ls_px(n);
            if (sp != 0.0f && n->text_len > 0) {
                DWRITE_TEXT_RANGE all = { 0, (UINT32)n->text_len };
                l1->lpVtbl->SetCharacterSpacing(l1, sp, sp, 0.0f, all);
            }
            l1->lpVtbl->Release(l1);
        }
    }
    /* line-height（显式声明时 UNIFORM：基线近似 = 半leading + 0.8em 上浮） */
    if (n->lh_unit >= 0) {
        float lh = lh_px(n);
        if (lh > 0.0f) {
            float bl = (lh - n->font_size) * 0.5f + n->font_size * 0.8f;
            if (bl < 0.0f) bl = 0.0f;
            l->lpVtbl->SetLineSpacing(l, DWRITE_LINE_SPACING_METHOD_UNIFORM, lh, bl);
        }
    }

    DWRITE_TEXT_METRICS tm;
    l->lpVtbl->GetMetrics(l, &tm);
    n->dl_nat_w = tm.widthIncludingTrailingWhitespace;
    n->dl_nat_h = tm.height;
    n->dlayout = l;    n->dl_len = n->text_len; n->dl_box_w = box_w;
    n->dl_size = n->font_size; n->dl_weight = n->font_weight;
    n->dl_family = n->font_family; n->dl_align = n->text_align;
    n->dl_ls = ls_px(n);
    n->dl_lh = n->lh_unit >= 0 ? lh_px(n) : 0.0f;
    return l;
}

/* ===== inline 判定与行盒 ===== */
static int tag_is_inline(const char* tag) {
    if (!tag) return 0;
    static const char* INL[] = {
        "text", "span", "a", "button", "img", "b", "strong", "i", "em",
        "u", "s", "small", "label", "code", "input", NULL
    };
    for (int i = 0; INL[i]; i++) if (!strcmp(tag, INL[i])) return 1;
    return 0;
}

static int node_inline_self(NWNode* n) {
    if (!n) return 0;
    if (!strcmp(n->tag, "text")) return 1;
    if (!strcmp(n->tag, "linebox")) return 1;
    /* 作者 CSS display:block 化的 inline 标签 → 块级（查静态级联 st：flex row 默认）
     * 简化：直接按标签白名单（作者块级化的场景由 restyle 设置 flex_direction 列区分，
     * 当前生成代码未出现该形态） */
    return tag_is_inline(n->tag);
}

/* 容器是否 flex/grid 语义（行盒包裹只对块流容器） */
static int is_flexlike(NWNode* n) {
    return n && n->st.display == 0 && n->first_child; /* taffy 全 flex：由 restyle 阶段
    的 flex_direction/生成语义决定；linebox 划分排除 flex 容器由标记完成 */
}

/* 行盒构建：把块流容器的连续 inline 子节点包进匿名 linebox（构建期一次性） */
static void build_lineboxes(NWNode* parent) {
    if (!parent || parent->lb_built) return;
    parent->lb_built = 1;
    /* floatrow 由 uigen 建树时专门管理浮动子项（flex_dir 直发），
     * 严禁再包 linebox——双重包装 + taffy 双亲挂接会把布局彻底搅乱 */
    if (parent->tag && !strcmp(parent->tag, "floatrow")) return;
    NWNode* c = parent->first_child;
    while (c) {
        NWNode* next = c->next;
        int inl = node_inline_self(c);
        if (inl && strcmp(c->tag, "linebox") != 0) {
            /* 收集连续 inline 段 */
            NWNode* start = c;
            NWNode* end = c;
            NWNode* q = c;
            while (q && node_inline_self(q) && strcmp(q->tag, "linebox") != 0) {
                end = q;
                q = q->next;
            }
            next = q; /* 段后第一个非 inline（或 NULL） */
            /* 浮动子项排除：float 容器门控由 flex 语义近似（当前无 float 支持，保留位） */
            int multi = 0;
            for (NWNode* k = start; k != end->next; k = k->next) if (k != start) multi = 1;
            if (multi) {
                /* 创建 linebox 节点插入段首 */
                NWNode* lb = (NWNode*)calloc(1, sizeof(NWNode));
                lb->tag = _strdup("linebox");
                lb->page_id = parent->page_id;
                if (!g_tree) g_tree = nw_layout_new();
                lb->taffy_id = nw_layout_node(g_tree, &lb->st);
                node_init_common(lb);
                lb->st.flex_direction = 0; /* row */
                lb->st.align = 3;          /* stretch（inline 基线近似） */
                node_register(lb);
                /* 插入链表：start 之前 */
                lb->parent = parent;
                lb->prev = start->prev;
                lb->next = end->next;
                if (start->prev) start->prev->next = lb;
                else parent->first_child = lb;
                if (end->next) end->next->prev = lb;
                else parent->last_child = lb;
                /* 段内子节点全部挂到 lb（保留原顺序） */
                NWNode* k = start;
                while (k != end->next) {
                    NWNode* kn = k->next;
                    k->parent = lb;
                    k->prev = lb->last_child;
                    if (lb->last_child) lb->last_child->next = k;
                    else lb->first_child = k;
                    lb->last_child = k;
                    k->next = NULL;
                    k = kn;
                }
                lb->first_child = start;
                start->prev = NULL;
                end->next = NULL;
                /* layout 树：lb 挂 parent，子挂 lb */
                nw_layout_add_child(g_tree, parent->taffy_id, lb->taffy_id);
                for (NWNode* k2 = lb->first_child; k2; k2 = k2->next)
                    nw_layout_add_child(g_tree, lb->taffy_id, k2->taffy_id);
            }
        } else if (!inl && !(!strcmp(c->tag, "linebox"))) {
            /* 块级子：递归（文本节点不会走到这） */
            build_lineboxes(c);
        }
        c = next;
    }
}

/* ===== 布局同步 ===== */
static void read_pos(NWNode* n, float ox, float oy) {
    float out[6];
    nw_layout_get(g_tree, n->taffy_id, out);
    n->x = ox + out[0]; n->y = oy + out[1];
    n->w = out[2];      n->h = out[3];
    /* 有效 z：祖先链 max（子树跟随最高 z 祖先的层） */
    n->eff_z = n->parent ? (n->parent->eff_z > n->z ? n->parent->eff_z : n->z) : n->z;
    /* no_scroll：自身或任一祖先为 fixed/absolute → 不随页面滚动。
     * 浏览器语义：fixed 子树与滚动流无关（否则 wheel 后 hud 按钮视觉
     * 漂移且命中区错位——ttt 控件「乱走、点不中」的根源） */
    n->no_scroll = n->parent ? (n->parent->no_scroll || n->parent->st.pos_kind != 0)
                             : (n->st.pos_kind != 0);
    /* 根级 fixed/absolute：浏览器锚定初始包含块（视口 0,0）——由 inset 语义在 layout 处理，
     * 这里只累计内容高度（滚动范围） */
    if (n->st.pos_kind == 0 && !n->detached) {
        float bottom = n->y + n->h;
        if (bottom > g_content_h) g_content_h = bottom;
    }
    /* 子节点：layout 返回相对父坐标，累计父绝对偏移（ox,oy 必须传本节点绝对值，
     * 恒传根偏移会让第二层起全部坐标退化为局部值——绘制/命中全错） */
    for (NWNode* c = n->first_child; c; c = c->next) read_pos(c, n->x, n->y);
}

/* 父元素自身宽度是否确定（能被文本 100% 引用） */
static int width_definite_chain(NWNode* q) {
    if (!q) return 1;                        /* 根：视口确定 */
    if (q->st.width_kind != 0) return 1;     /* 显式宽度 */
    if (q->st.pos_kind != 0) return 0;       /* 收缩适应 */
    return width_definite_chain(q->parent);
}
static int parent_width_definite(NWNode* p) {
    if (!p) return 0;
    if (p->st.width_kind == 1 || p->st.width_kind == 2) return 1;
    if (p->st.pos_kind != 0) return 0;
    NWNode* gp = p->parent;
    if (!gp) return 1; /* 根：视作确定 */
    if (gp->st.align == 1 || gp->st.align == 2) return 0;
    return width_definite_chain(gp);
}

/* 块级上下文中的文本应撑满容器宽度并折行（浏览器语义）；
 * 行内容器（button/span/a）或 flex center/end（shrink-to-fit）保持自然宽度 */
static int text_stretch_parent(NWNode* p) {
    if (!p || !p->tag) return 0;
    if (tag_is_inline(p->tag) || !strcmp(p->tag, "linebox")) return 0;
    if (p->st.align == 1 || p->st.align == 2) return 0;
    /* flex 容器（justify/align 语义）下的直接子项不撑满 */
    return 1;
}

static void nw_relayout(void) {
    if (!g_tree || !g_root || g_cw <= 0) return;
    g_root->st.width_kind = 1;  g_root->st.width = (float)g_cw;
    g_root->st.height_kind = 1; g_root->st.height = (float)g_ch;
    g_content_h = 0.0f;
    /* 首次构建行盒（DOM 静态，构建期一次） */
    build_lineboxes(g_root);
    for (int i = 0; i < g_node_count; i++) {
        NWNode* n = g_nodes[i];
        if (n->detached || n->manual_hidden) {
            n->st.display = 1; /* Display::None：从布局树中剔除 */
            nw_layout_set_style(g_tree, n->taffy_id, &n->st);
            continue;
        }
        /* 块流模拟（浏览器语义）：HTML 块级容器默认竖排（column）；
         * 豁免：uigen 显式发射 flex_dir（flex 容器/行盒/floatrow）与 inline 容器 */
        if (!n->flex_explicit && n->tag
            && strcmp(n->tag, "linebox") && strcmp(n->tag, "floatrow")
            && !tag_is_inline(n->tag)) {
            n->st.flex_direction = 1;
            n->st.align = 3; /* 块流：子项横向撑满（浏览器块级 auto 宽=填满容器） */
        }
        /* button UA 默认样式：文字水平+垂直居中（浏览器按钮语义）。
         * 只在作者未显式设 align/justify 时生效（align=3 是上面的块流默认，
         * 同样视为未设置） */
        if (n->tag && !strcmp(n->tag, "button") && !n->flex_explicit) {
            if (n->st.align == 0 || n->st.align == 3) n->st.align = 1;   /* 交叉轴居中 */
            if (n->st.justify == 0) n->st.justify = 1;                   /* 主轴居中 */
        }
        /* column+wrap：taffy 按可用高度折「列」横向排（浏览器 auto 高容器永不折）。
         * 高度 auto 的 column 容器抑制 wrap——experience .timeline 横排爆炸根因 */
        if ((n->st.flex_direction == 1 || n->st.flex_direction == 3) && n->st.height_kind == 0)
            n->st.flex_wrap = 0;
        if (n->text && !n->is_ta) {
            /* 大宽度下测自然尺寸 */
            get_layout(n, 10000.0f);
            if (n->tag && !strcmp(n->tag, "text")) {
                /* 文本子节点：裸文本尺寸即盒尺寸 */
                NWNode* p = n->parent;
                if (p && !node_inline_self(n) && text_stretch_parent(p) && parent_width_definite(p)) {
                    n->st.width_kind = 2;  n->st.width = 100.0f; /* 撑满容器 → 折行 */
                } else {
                    n->st.width_kind = 1;  n->st.width = ceilf(n->dl_nat_w);
                }
                n->st.height_kind = 1; n->st.height = ceilf(n->dl_nat_h);
                n->h_auto = 1;
            } else {
                /* 元素节点（JS textContent 注入的文本）：浏览器盒语义——
                 * 内容尺寸 + padding + border；CSS 显式宽/高不覆盖
                 * （themeBtn height:32 曾被裸文本 17px 覆盖 → 竖排塌缩） */
                if (n->st.width_kind == 0) {
                    n->st.width_kind = 1;
                    n->st.width = ceilf(n->dl_nat_w) + n->st.padding[1] + n->st.padding[3]
                                + n->border_w[1] + n->border_w[3];
                }
                if (n->st.height_kind == 0) {
                    n->st.height_kind = 1;
                    n->st.height = ceilf(n->dl_nat_h) + n->st.padding[0] + n->st.padding[2]
                                 + n->border_w[0] + n->border_w[2];
                    n->h_auto = 1;
                }
            }
        }
        /* 替换元素（img）固有尺寸默认：宽高均 auto → 固有宽 + 固有比例 */
        if (n->img_res >= 0 && n->img_w > 0.0f && n->img_h > 0.0f) {
            if (n->st.width_kind == 0 && n->st.height_kind == 0) {
                n->st.width_kind = 1; n->st.width = n->img_w;
                /* 高度必须同步固有权重——只写宽时 taffy 高 auto 得 0
                 * （experience 页内容图全部 h=0 塌缩的根源），
                 * 宽被容器钳窄后的比例收缩由下方第二遍修正补齐 */
                n->st.height_kind = 1; n->st.height = n->img_h;
            }
            n->st.aspect = n->img_w / n->img_h;
            /* % 宽高 + 收缩适应父（fixed/absolute 无显式宽）：先以 max-content 定父盒，
             * % 再对父盒解析；高度按固有比例随宽 */
            if (n->st.width_kind == 2 && n->st.height_kind == 2 &&
                n->parent && n->parent->st.pos_kind != 0 && n->parent->st.width_kind == 0) {
                float base_w = n->img_w < (float)g_cw ? n->img_w : (float)g_cw;
                float wpx = base_w * n->st.width / 100.0f;
                n->st.width_kind = 1;  n->st.width = wpx;
                n->st.height_kind = 1; n->st.height = wpx / n->st.aspect;
            }
        }
        nw_layout_set_style(g_tree, n->taffy_id, &n->st);
        if (dbg_on() && n->cls && !strcmp(n->cls, "timeline"))
            fprintf(stderr, "[nw] probe set_style timeline taffy=%u fd=%d wrap=%d disp=%d w_k=%d w=%.0f\n",
                n->taffy_id, n->st.flex_direction, n->st.flex_wrap, n->st.display,
                n->st.width_kind, n->st.width);
    }
    nw_layout_run(g_tree, g_root->taffy_id, (float)g_cw, (float)g_ch, 0);
    read_pos(g_root, 0.0f, 0.0f);
    /* 第二遍：按第一遍分到的实际宽度重测文本（折行后高度变化），必要时再跑一次。
     * 只修正 h_auto 节点（relayout 自己写的高度）；CSS 显式高度绝不动，
     * 且元素节点要加回 padding/border（裸文本高度曾踩掉 themeBtn height:32） */
    int changed = 0;
    for (int i = 0; i < g_node_count; i++) {
        NWNode* n = g_nodes[i];
        if (!n->text || n->detached || n->w <= 0 || n->is_ta || !n->h_auto) continue;
        get_layout(n, n->w);
        float hh = ceilf(n->dl_nat_h);
        int is_elem = n->tag && strcmp(n->tag, "text") != 0;
        if (is_elem) hh += n->st.padding[0] + n->st.padding[2] + n->border_w[0] + n->border_w[2];
        if (hh != n->st.height) {
            n->st.height_kind = 1; n->st.height = hh;
            nw_layout_set_style(g_tree, n->taffy_id, &n->st);
            changed = 1;
        }
    }
    if (changed) {
        nw_layout_run(g_tree, g_root->taffy_id, (float)g_cw, (float)g_ch, 0);
        read_pos(g_root, 0.0f, 0.0f);
    }
    /* 第三遍：img 固有比例高度跟随。宽高均 auto 的替换元素第一遍写入固有权重，
     * 若被容器钳窄（max-width:100% / 父宽），高度必须按固有比例收缩（浏览器
     * 替换元素语义）；taffy 的 aspect 链路未接，这里按实际宽显式补一遍。 */
    {
        int ich = 0;
        for (int i = 0; i < g_node_count; i++) {
            NWNode* n = g_nodes[i];
            if (n->detached || n->img_res < 0 || n->img_w <= 0 || n->img_h <= 0) continue;
            if (n->st.height_kind != 1 || n->st.height <= 0) continue;
            float want_h = n->w * (n->img_h / n->img_w);
            if (n->w > 1.0f && fabsf(n->st.height - want_h) > 0.5f && want_h > 0.0f) {
                n->st.height = want_h;
                nw_layout_set_style(g_tree, n->taffy_id, &n->st);
                ich = 1;
            }
        }
        if (ich) {
            nw_layout_run(g_tree, g_root->taffy_id, (float)g_cw, (float)g_ch, 0);
            read_pos(g_root, 0.0f, 0.0f);
        }
    }
    {
        LARGE_INTEGER pf, now; QueryPerformanceFrequency(&pf); QueryPerformanceCounter(&now);
        dbg("[nw] relayout: nodes=%d client=%dx%d root=%.0fx%.0f (t=%.1f ms)\n",
            g_node_count, g_cw, g_ch, g_root->w, g_root->h,
            (double)(now.QuadPart - g_t0.QuadPart) * 1000.0 / (double)pf.QuadPart);
    }
    /* 滚动范围钳制（内容变矮后回弹） */
    {
        float max_scroll = g_content_h - (float)g_ch;
        if (max_scroll < 0.0f) max_scroll = 0.0f;
        if (g_scroll_y > max_scroll) g_scroll_y = max_scroll;
        if (g_scroll_y < 0.0f) g_scroll_y = 0.0f;
        g_scroll_target = g_scroll_y;
    }
    if (getenv("NW_DUMP")) {
        /* 动作钩子（click/key/...）存在时跳过逐次 relayout dump——init 期 20+ 次
         * relayout × 2500 行无缓冲 stderr 会把首帧拖到分钟级；动作后几何
         * 由 snap2 分支的 nw_dump_nodes 统一提供 */
        if (!(getenv("NW_CLICK") || getenv("NW_CLICK2") || getenv("NW_KEY") ||
              getenv("NW_TOUCH") || getenv("NW_RESIZE") || getenv("NW_SCROLL")))
            nw_dump_nodes();
    }
}

/* 全节点树 dump（NW_DUMP 钩子；relayout 末尾与 snap2/snap3 动作后共用） */
void nw_dump_nodes(void) {
    for (int i = 0; i < g_node_count; i++) {
        NWNode* n = g_nodes[i];
        if (n->detached) { dbg("  [%2d] %s#%s DETACHED\n", i, n->tag ? n->tag : "text", n->id ? n->id : "-"); continue; }
        dbg("  [%2d] %-6s#%-6s x=%6.1f y=%6.1f w=%6.1f h=%6.1f f=%5.1f cls=%s fd=%d fx=%d pg=%d%s len=%d\n",
            i, n->tag ? n->tag : "text", n->id ? n->id : "-",
            n->x, n->y, n->w, n->h, n->font_size, n->cls ? n->cls : "-",
            n->st.flex_direction, n->flex_explicit, n->page_id,
            n->onclick ? " ONCLICK" : "", n->text ? n->text_len : -1);
        if (n->text && n->text_len && dbg_on()) {
            fprintf(stderr, "        c=(%.2f,%.2f,%.2f,%.2f) set=%d chain:",
                n->color[0], n->color[1], n->color[2], n->color[3], n->color_set);
            for (NWNode* p2 = n->parent; p2; p2 = p2->parent)
                fprintf(stderr, " %s#%s%d", p2->tag ? p2->tag : "?", p2->id ? p2->id : "-", p2->color_set);
            fprintf(stderr, "\n");
        }
    }
}

void nw_touch(NWNode* n) {
    (void)n;
    if (g_batch_restyle) return; /* 批量重放期抑制：收尾由批量入口统一 relayout 一次 */
    if (g_hwnd) { nw_relayout(); InvalidateRect(g_hwnd, NULL, FALSE); }
}

static void ensure_invalidate(void) {
    if (g_hwnd) InvalidateRect(g_hwnd, NULL, FALSE);
}

/* ===== 绘制 ===== */
static ID2D1Layer* g_layer_tmp = NULL;

/* 圆角边框裁剪层：返回 0=无 1=直角 PushAxisAlignedClip（由调用方配对） 2=圆角层 */
static int border_clip_push(ID2D1RenderTarget* rt, NWNode* n, float rad, float sy) {
    D2D1_RECT_F r = { n->x, n->y + sy, n->x + n->w, n->y + sy + n->h };
    if (rad > 0.0f) {
        if (!g_layer_tmp) {
            { D2D1_SIZE_F zf = { 0, 0 }; rt->lpVtbl->CreateLayer(rt, &zf, &g_layer_tmp); }
        }
        if (g_layer_tmp) {
            D2D1_ROUNDED_RECT rr = { r, rad, rad };
            D2D1_LAYER_PARAMETERS lp = {
                .contentBounds = { r.left, r.top, r.right, r.bottom },
                .geometricMask = NULL,
                .maskAntialiasMode = D2D1_ANTIALIAS_MODE_ALIASED,
                .maskTransform = { 1, 0, 0, 1, 0, 0 },
                .opacity = 1.0f,
                .opacityBrush = NULL,
                .layerOptions = D2D1_LAYER_OPTIONS_NONE,
            };
            rt->lpVtbl->PushLayer(rt, &lp, g_layer_tmp);
            /* 圆角几何裁剪：用 axis clip 近似（圆角精确裁剪由 border 绘制本身处理） */
            rt->lpVtbl->PushAxisAlignedClip(rt, &r, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
            return 2;
        }
        return 0;
    }
    rt->lpVtbl->PushAxisAlignedClip(rt, &r, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
    return 1;
}

/* 渐变插值：t 0..1 → 颜色（CPU 逐带近似；0=向上 90=向右 CSS 角度） */
static void grad_eval(const NWNode* n, float t, float* or_, float* og, float* ob, float* oa) {
    if (n->grad_n == 0) { *or_=0; *og=0; *ob=0; *oa=0; return; }
    if (n->grad_n == 1 || t <= n->grad_stops[0][0]) {
        *or_=n->grad_stops[0][1]; *og=n->grad_stops[0][2]; *ob=n->grad_stops[0][3]; *oa=n->grad_stops[0][4];
        return;
    }
    for (int i = 1; i < n->grad_n; i++) {
        if (t <= n->grad_stops[i][0]) {
            const float* a = n->grad_stops[i-1];
            float p0 = n->grad_stops[i-1][0], p1 = n->grad_stops[i][0];
            float f = (p1 > p0) ? (t - p0) / (p1 - p0) : 0.0f;
            *or_ = a[1] + (n->grad_stops[i][1] - a[1]) * f;
            *og = a[2] + (n->grad_stops[i][2] - a[2]) * f;
            *ob = a[3] + (n->grad_stops[i][3] - a[3]) * f;
            *oa = a[4] + (n->grad_stops[i][4] - a[4]) * f;
            return;
        }
    }
    const float* l = n->grad_stops[n->grad_n - 1];
    *or_=l[1]; *og=l[2]; *ob=l[3]; *oa=l[4];
}

static void draw_node(ID2D1RenderTarget* rt, NWNode* n);

/* 容器绘制体（bg/边框/图/文本），sy = 滚动偏移 */
static void draw_node_body(ID2D1RenderTarget* rt, NWNode* n, float sy) {
    float rad = n->radius;
    if (rad > fminf(n->w, n->h) * 0.5f) rad = fminf(n->w, n->h) * 0.5f;
    D2D1_RECT_F box = { n->x, n->y + sy, n->x + n->w, n->y + sy + n->h };

    /* box-shadow（外阴影近似：偏移半透明矩形，blur 分 3 层 alpha 展开） */
    if (n->has_shadow && n->w > 0 && n->h > 0) {
        D2D1_COLOR_F sc = { n->sh_c[0], n->sh_c[1], n->sh_c[2], n->sh_c[3] };
        float blur = n->sh_blur;
        for (int pass = 2; pass >= 1; pass--) {
            float b = blur * (float)pass / 3.0f;
            D2D1_COLOR_F a2 = sc; a2.a = sc.a * (pass == 2 ? 0.25f : 0.45f);
            g_brush->lpVtbl->SetColor(g_brush, &a2);
            D2D1_ROUNDED_RECT rr = { {
                n->x - b + n->sh_ox, n->y + sy - b + n->sh_oy,
                n->x + n->w + b + n->sh_ox, n->y + sy + n->h + b + n->sh_oy,
            }, rad + b, rad + b };
            rt->lpVtbl->FillRoundedRectangle(rt, &rr, (ID2D1Brush*)g_brush);
        }
        (void)sc;
    }

    /* 背景：渐变 or 纯色 */
    if (n->grad_on && n->grad_n >= 2 && n->w > 0 && n->h > 0) {
        /* CSS 角度：0=向上 90=向右 → 单位向量（屏幕 y 向下） */
        float raddeg = n->grad_deg * 3.14159265f / 180.0f;
        float dxv = sinf(raddeg), dyv = -cosf(raddeg);
        /* 投影长度（盒对角在渐变方向上的投影范围） */
        float proj[4] = {
            n->x * dxv + (n->y + sy) * dyv,
            (n->x + n->w) * dxv + (n->y + sy) * dyv,
            n->x * dxv + (n->y + sy + n->h) * dyv,
            (n->x + n->w) * dxv + (n->y + sy + n->h) * dyv,
        };
        float pmin = proj[0], pmax = proj[0];
        for (int k = 1; k < 4; k++) { if (proj[k] < pmin) pmin = proj[k]; if (proj[k] > pmax) pmax = proj[k]; }
        float prange = pmax - pmin; if (prange <= 0) prange = 1;
        int bands = 24;
        float step = n->h / (float)bands; /* 近似：按竖直分带（水平渐变用宽） */
        if (fabsf(dxv) > fabsf(dyv)) step = n->w / (float)bands;
        D2D1_RECT_F clip = box;
        rt->lpVtbl->PushAxisAlignedClip(rt, &clip, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
        int horiz = fabsf(dxv) > fabsf(dyv);
        float cell = horiz ? n->w / (float)bands : n->h / (float)bands;
        for (int i = 0; i < bands; i++) {
            float t0 = (float)i / (float)bands;
            float mid = t0 + 0.5f / bands;
            float r, g, b, a;
            grad_eval(n, mid, &r, &g, &b, &a);
            D2D1_COLOR_F c = { r, g, b, a };
            g_brush->lpVtbl->SetColor(g_brush, &c);
            D2D1_RECT_F s;
            if (horiz) s = (D2D1_RECT_F){ n->x + i * cell, n->y + sy, n->x + (i + 1) * cell + 0.75f, n->y + sy + n->h };
            else       s = (D2D1_RECT_F){ n->x, n->y + sy + i * cell, n->x + n->w, n->y + sy + (i + 1) * cell + 0.75f };
            rt->lpVtbl->FillRectangle(rt, &s, (ID2D1Brush*)g_brush);
        }
        rt->lpVtbl->PopAxisAlignedClip(rt);
        (void)step; (void)pmin; (void)pmax; (void)prange;
    } else if (n->has_bg) {
        D2D1_COLOR_F c = { n->bg[0], n->bg[1], n->bg[2], n->bg[3] };
        g_brush->lpVtbl->SetColor(g_brush, &c);
        if (rad > 0.0f) {
            D2D1_ROUNDED_RECT rr = { box, rad, rad };
            rt->lpVtbl->FillRoundedRectangle(rt, &rr, (ID2D1Brush*)g_brush);
        } else {
            rt->lpVtbl->FillRectangle(rt, &box, (ID2D1Brush*)g_brush);
        }
    }
    /* 背景图 + 内容图 */
    if (n->bg_img_res >= 0) draw_img(rt, n, sy, 1);
    if (n->img_res >= 0) draw_img(rt, n, sy, 0);
    /* 边框：四边同宽同色 → 描边（内缩 w/2，border-box 一致）；各边不同 → 条带 */
    if (n->has_border) {
        float bw0 = n->border_w[0];
        int uniform = bw0 > 0.0f;
        for (int s = 1; s < 4 && uniform; s++) {
            if (fabsf(n->border_w[s] - bw0) > 0.01f) uniform = 0;
            for (int k = 0; k < 4 && uniform; k++)
                if (fabsf(n->border_c[s][k] - n->border_c[0][k]) > 0.004f) uniform = 0;
        }
        if (uniform) {
            D2D1_COLOR_F bc = { n->border_c[0][0], n->border_c[0][1], n->border_c[0][2], n->border_c[0][3] };
            g_brush->lpVtbl->SetColor(g_brush, &bc);
            float in = bw0 * 0.5f;
            if (rad > 0.0f) {
                float r2 = rad - in; if (r2 < 0.0f) r2 = 0.0f;
                D2D1_ROUNDED_RECT rr = { { box.left + in, box.top + in, box.right - in, box.bottom - in }, r2, r2 };
                rt->lpVtbl->DrawRoundedRectangle(rt, &rr, (ID2D1Brush*)g_brush, bw0, NULL);
            } else {
                D2D1_RECT_F r2 = { box.left + in, box.top + in, box.right - in, box.bottom - in };
                rt->lpVtbl->DrawRectangle(rt, &r2, (ID2D1Brush*)g_brush, bw0, NULL);
            }
        } else {
            float y0 = n->y + sy;
            int clipped = 0;
            if (rad > 0.0f) clipped = border_clip_push(rt, n, rad, sy);
            for (int s = 0; s < 4; s++) {
                float w = n->border_w[s];
                if (w <= 0.0f) continue;
                D2D1_COLOR_F bc = { n->border_c[s][0], n->border_c[s][1], n->border_c[s][2], n->border_c[s][3] };
                g_brush->lpVtbl->SetColor(g_brush, &bc);
                D2D1_RECT_F e;
                switch (s) {
                case 0: e = (D2D1_RECT_F){ n->x, y0, n->x + n->w, y0 + w }; break;
                case 1: e = (D2D1_RECT_F){ n->x + n->w - w, y0, n->x + n->w, y0 + n->h }; break;
                case 2: e = (D2D1_RECT_F){ n->x, y0 + n->h - w, n->x + n->w, y0 + n->h }; break;
                default: e = (D2D1_RECT_F){ n->x, y0, n->x + w, y0 + n->h }; break;
                }
                rt->lpVtbl->FillRectangle(rt, &e, (ID2D1Brush*)g_brush);
            }
            if (clipped) {
                /* border_clip_push 返回 2 = PushLayer+PushAxisAlignedClip 两层，逆序全弹出 */
                rt->lpVtbl->PopAxisAlignedClip(rt);
                if (clipped == 2) {
                    rt->lpVtbl->PopLayer(rt);
                    if (g_layer_tmp) {
                        ((IUnknownVtbl*)(g_layer_tmp)->lpVtbl)->Release((IUnknown*)(g_layer_tmp));
                        g_layer_tmp = NULL;
                    }
                }
            }
        }
    }

    /* 文本 */
    if (n->text && n->text_len > 0) {
        if (!n->color_set) nw_st_color_inh(n); /* CSS 继承：取祖先链最近显式色 */
        IDWriteTextLayout* l = get_layout(n, n->w > 0 ? n->w : 1.0f);
        if (l) {
            /* 文本绘制原点 = 内容盒左上（盒语义含 padding/border 偏移）；
             * 裸文本子节点盒即内容盒，不加偏移（themeBtn ☾ 曾被左边框裁半）。
             * 选择高亮/阴影/正文共用同一原点，保证三者像素对齐。 */
            float tox = n->x, toy = n->y + sy;
            if (n->tag && strcmp(n->tag, "text") != 0) {
                tox += n->border_w[3] + n->st.padding[3];
                toy += n->border_w[0] + n->st.padding[0];
            }
            /* 文本选择高亮（先底色后文字） */
            sel_highlight(rt, n, l, tox, toy);
            D2D1_COLOR_F c = { n->color[0], n->color[1], n->color[2], n->color_set ? n->color[3] : 1.0f };
            g_brush->lpVtbl->SetColor(g_brush, &c);
            /* text-shadow（先画影再画字） */
            if (n->ts_on) {
                float tdx = n->ts_dx, tdy = n->ts_dy;
                if (n->ts_ux == 1) tdx = n->ts_dx * (float)g_cw / 100.0f;
                if (n->ts_ux == 2) tdx = n->ts_dx * n->w / 100.0f;
                if (n->ts_uy == 1) tdy = n->ts_dy * (float)g_ch / 100.0f;
                if (n->ts_uy == 2) tdy = n->ts_dy * n->h / 100.0f;
                D2D1_COLOR_F sc = { n->ts_c[0], n->ts_c[1], n->ts_c[2], n->ts_c[3] };
                g_brush->lpVtbl->SetColor(g_brush, &sc);
                D2D1_POINT_2F so = { tox + tdx, toy + tdy };
                rt->lpVtbl->DrawTextLayout(rt, so, l, (ID2D1Brush*)g_brush, D2D1_DRAW_TEXT_OPTIONS_NONE);
            }
            D2D1_COLOR_F c2 = { n->color[0], n->color[1], n->color[2], n->color_set ? n->color[3] : 1.0f };
            g_brush->lpVtbl->SetColor(g_brush, &c2);
            D2D1_POINT_2F origin = { tox, toy };
            rt->lpVtbl->DrawTextLayout(rt, origin, l, (ID2D1Brush*)g_brush, D2D1_DRAW_TEXT_OPTIONS_NONE);
        }
    }
}

static int g_mark_count = 0;          /* NW_HITBOX 点击标记 */
static float g_marks[64][2];

/* 递归绘制：两遍 zpass——0=eff_z==0（文档流+无 z 的 fixed/absolute 按创建序合成），
 * 1=eff_z>0（顶层）。判定必须用 eff_z（祖先链 max）：fixed 容器内 pos=0 的子孙
 * 若被划进 pass0 而 fixed 背景（z=0）划进 pass1，背景会盖掉子孙文字。 */
static void draw_node(ID2D1RenderTarget* rt, NWNode* n) {
    if (!n || n->detached || n->manual_hidden || n->st.display == 1) return;
    int top = (n->eff_z > 0);
    int draw_self = (g_zpass == 0 && !top) || (g_zpass == 1 && top);
    /* overflow 裁剪（自绘前推一次；h=0 空盒也裁——子孙随不可见） */
    int ov = 0;
    if (draw_self && n->overflow_hidden) {
        D2D1_RECT_F r = { n->x, n->y, n->x + n->w, n->y + n->h };
        rt->lpVtbl->PushAxisAlignedClip(rt, &r, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
        ov = 1;
    }
    /* translate（浏览器 transform 语义：作用于自身+全部子孙） */
    float tdx = 0, tdy = 0;
    if (n->tr_kx == 1) tdx = n->tr_x;
    else if (n->tr_kx == 2) tdx = n->tr_x * n->w / 100.0f;
    else if (n->tr_kx == 11) tdx = n->tr_x * 0.01f * (float)g_cw;
    else if (n->tr_kx == 12) tdx = n->tr_x * 0.01f * (float)g_ch;
    else if (n->tr_kx == 13) tdx = n->tr_x * n->font_size;
    if (n->tr_ky == 1) tdy = n->tr_y;
    else if (n->tr_ky == 2) tdy = n->tr_y * n->h / 100.0f;
    else if (n->tr_ky == 11) tdy = n->tr_y * 0.01f * (float)g_cw;
    else if (n->tr_ky == 12) tdy = n->tr_y * 0.01f * (float)g_ch;
    else if (n->tr_ky == 13) tdy = n->tr_y * n->font_size;
    int tr_on = 0;
    D2D1_MATRIX_3X2_F saved;
    if (tdx != 0 || tdy != 0) {
        rt->lpVtbl->GetTransform(rt, &saved);
        D2D1_MATRIX_3X2_F tr = saved;
        tr.dx += tdx; tr.dy += tdy;
        rt->lpVtbl->SetTransform(rt, &tr);
        tr_on = 1;
    }
    if (draw_self) {
        float sy = (n->st.pos_kind == 0 && !n->no_scroll) ? -g_scroll_y : 0.0f;
        draw_node_body(rt, n, sy);
    }
    /* 子孙（无论自身是否绘制——溢出内容的子孙由裁剪控制） */
    for (NWNode* c = n->first_child; c; c = c->next) draw_node(rt, c);
    if (tr_on) rt->lpVtbl->SetTransform(rt, &saved);
    if (ov) rt->lpVtbl->PopAxisAlignedClip(rt);
}

/* ===== 帧绘制 ===== */
static void nw_draw(void) {
    if (!g_rt || !g_root) return;
    static int first_frame = 1;
    g_rt->lpVtbl->BeginDraw(g_rt);
    { /* 每帧强制 identity：防跨帧 transform 残留（GetTransform 污染会整体镜像/偏移） */
        D2D1_MATRIX_3X2_F ident = { 1, 0, 0, 1, 0, 0 };
        g_rt->lpVtbl->SetTransform(g_rt, &ident);
    }
    D2D1_COLOR_F clear = { g_page_bg[0], g_page_bg[1], g_page_bg[2], g_page_bg[3] };
    g_brush->lpVtbl->SetColor(g_brush, &clear);
    D2D1_RECT_F full = { 0, 0, (float)g_cw, (float)g_ch };
    g_rt->lpVtbl->FillRectangle(g_rt, &full, (ID2D1Brush*)g_brush);
    g_zpass = 0; draw_node(g_rt, g_root);   /* 底层（文档流） */
    g_zpass = 1; draw_node(g_rt, g_root);   /* 顶层（position:fixed + z-index>0 悬浮件） */
    g_zpass = 0;
    /* NW_HITBOX=1：点击标记红叉（程序真实收到的坐标，持久保留） */
    {
        int hitbox = getenv("NW_HITBOX") != NULL;
        if (hitbox && g_mark_count > 0) {
            D2D1_COLOR_F red = { 1, 0, 0, 1 };
            g_brush->lpVtbl->SetColor(g_brush, &red);
            for (int m = 0; m < g_mark_count && m < 64; m++) {
                float mx = g_marks[m][0], my = g_marks[m][1];
                D2D1_RECT_F h1 = { mx - 9, my - 2, mx + 9, my + 2 };
                D2D1_RECT_F h2 = { mx - 2, my - 9, mx + 2, my + 9 };
                g_rt->lpVtbl->FillRectangle(g_rt, &h1, (ID2D1Brush*)g_brush);
                g_rt->lpVtbl->FillRectangle(g_rt, &h2, (ID2D1Brush*)g_brush);
            }
        }
    }
    /* 滚动条（内容超出视口时） */
    if (g_content_h > (float)g_ch && g_ch > 0 && g_cw > 8) {
        float track_w = 6.0f;
        float max_scroll = g_content_h - (float)g_ch;
        float thumb_h = (float)g_ch * (float)g_ch / g_content_h;
        if (thumb_h < 24.0f) thumb_h = 24.0f;
        float thumb_y = (g_scroll_y / max_scroll) * ((float)g_ch - thumb_h);
        D2D1_COLOR_F trk = { 0, 0, 0, 0.05f };
        D2D1_COLOR_F th = { 0, 0, 0, 0.22f };
        D2D1_RECT_F tr = { (float)g_cw - track_w, 0, (float)g_cw, (float)g_ch };
        g_brush->lpVtbl->SetColor(g_brush, &trk);
        g_rt->lpVtbl->FillRectangle(g_rt, &tr, (ID2D1Brush*)g_brush);
        D2D1_RECT_F trh = { (float)g_cw - track_w, thumb_y, (float)g_cw - 1.0f, thumb_y + thumb_h };
        g_brush->lpVtbl->SetColor(g_brush, &th);
        g_rt->lpVtbl->FillRectangle(g_rt, &trh, (ID2D1Brush*)g_brush);
    }
    HRESULT hr = g_rt->lpVtbl->EndDraw(g_rt, NULL, NULL);
    if (hr != S_OK) dbg("[nw] EndDraw hr=0x%08lx\n", (unsigned long)hr);
    if (first_frame) {
        first_frame = 0;
        LARGE_INTEGER pf, now;
        QueryPerformanceFrequency(&pf); QueryPerformanceCounter(&now);
        dbg("[nw] first frame t=%.1f ms (since nw_app_run)\n",
            (double)(now.QuadPart - g_t0.QuadPart) * 1000.0 / (double)pf.QuadPart);
    }
    if (hr == (HRESULT)0x8899000C) { /* D2DERR_RECREATE_TARGET */
        g_rt->lpVtbl->Base.Base.Release((IUnknown*)g_rt); g_rt = NULL; g_hrt = NULL;
        g_rt_gen++; /* 缓存的 D2D 位图随旧 RT 失效：换代后按需重建 */
    }
    /* ===== 测试钩子（NW_* 环境变量，无头验证） ===== */
    static int snap_done = 0;
    static int snap2_done = 0;
    static int snap3_done = 0;
    if (!snap_done) {
        /* NW_SNAPSHOT_DELAY=秒：首帧不截，挂定时器到点再截 NW_SNAPSHOT 并退出
           （验证 onload/setTimer 异步填充——首帧截图时机太早） */
        const char* sdel = getenv("NW_SNAPSHOT_DELAY");
        if (sdel) {
            double dsec = atof(sdel);
            if (dsec > 0.0) {
                snap_done = 1;
                SetTimer(g_hwnd, 7, (UINT)(dsec * 1000.0), NULL);
                dbg("[nw] test: NW_SNAPSHOT_DELAY %.1fs armed\n", dsec);
                return;
            }
        }
        snap_done = 1;
        const char* snap = getenv("NW_SNAPSHOT");
        if (snap) nw_snapshot_bmp(snap);
        { /* NW_KEY=39x3：首帧后连发 n 次虚拟键 */
            const char* kv = getenv("NW_KEY");
            if (kv) {
                int vk = 0, times = 1;
                sscanf(kv, "%dx%d", &vk, &times);
                if (vk > 0) {
                    for (int k = 0; k < times; k++) {
                        PostMessageW(g_hwnd, WM_KEYDOWN, (WPARAM)vk, 0);
                        PostMessageW(g_hwnd, WM_KEYUP, (WPARAM)vk, 0);
                    }
                    dbg("[nw] test: synth key %d x%d\n", vk, times);
                }
            }
        }
        const char* click = getenv("NW_CLICK");
        const char* scroll = getenv("NW_SCROLL");
        const char* sel = getenv("NW_SEL");
        const char* touch = getenv("NW_TOUCH");
        if (click) {
            int cx = 0, cy = 0;
            if (sscanf(click, "%d,%d", &cx, &cy) == 2) {
                dbg("[nw] test: synth click at %d,%d\n", cx, cy);
                PostMessageW(g_hwnd, WM_LBUTTONDOWN, MK_LBUTTON, MAKELPARAM(cx, cy));
                PostMessageW(g_hwnd, WM_LBUTTONUP, 0, MAKELPARAM(cx, cy));
            } else { /* 单值语法：NW_CLICK=x + NW_CLICKY=y */
                cx = atoi(click);
                const char* cyv = getenv("NW_CLICKY");
                cy = cyv ? atoi(cyv) : 0;
                dbg("[nw] test: synth click at %d,%d\n", cx, cy);
                PostMessageW(g_hwnd, WM_LBUTTONDOWN, MK_LBUTTON, MAKELPARAM(cx, cy));
                PostMessageW(g_hwnd, WM_LBUTTONUP, 0, MAKELPARAM(cx, cy));
            }
        } else if (scroll) {
            int d = atoi(scroll);
            dbg("[nw] test: synth wheel delta %d\n", d);
            PostMessageW(g_hwnd, WM_MOUSEWHEEL, MAKEWPARAM(0, (WORD)d), 0);
        } else if (sel) {
            int x1, y1, x2, y2;
            if (sscanf(sel, "%d,%d,%d,%d", &x1, &y1, &x2, &y2) == 4) {
                dbg("[nw] test: synth drag (%d,%d)->(%d,%d)\n", x1, y1, x2, y2);
                PostMessageW(g_hwnd, WM_LBUTTONDOWN, MK_LBUTTON, MAKELPARAM(x1, y1));
                for (int st = 1; st <= 8; st++) {
                    int mx = x1 + (x2 - x1) * st / 8, my = y1 + (y2 - y1) * st / 8;
                    PostMessageW(g_hwnd, WM_MOUSEMOVE, MK_LBUTTON, MAKELPARAM(mx, my));
                }
                PostMessageW(g_hwnd, WM_LBUTTONUP, 0, MAKELPARAM(x2, y2));
            }
        } else if ((touch = getenv("NW_TOUCH")) != NULL) {
            int cx = 0, cy = 0;
            if (sscanf(touch, "%d,%d", &cx, &cy) == 2) {
                dbg("[nw] test: synth touchstart at %d,%d\n", cx, cy);
                touch_fire((double)cx, (double)cy, hit_test((float)cx, (float)cy), 0);
            }
        } else if (getenv("NW_QUIT_AFTER_SNAP")) {
            PostMessageW(g_hwnd, WM_CLOSE, 0, 0);
        }
        { /* NW_RESIZE=WxH：真实窗口 resize（SetWindowPos → WM_SIZE 全链路）。
           * 只在这里改窗口尺寸并失效——WM_PAINT 由消息循环异步派发，
           * 第二帧分支（snap2）里快照到的才是重绘后的新帧（同步快照会
           * 截到旧帧，控件看似消失） */
            static int resize_done = 0;
            const char* rs = getenv("NW_RESIZE");
            if (rs && !resize_done) {
                int rw = 0, rh = 0;
                if (sscanf(rs, "%dx%d", &rw, &rh) == 2 && rw > 0 && rh > 0) {
                    RECT r0 = { 0, 0, rw, rh };
                    resize_done = 1;
                    AdjustWindowRect(&r0, WS_OVERLAPPEDWINDOW, FALSE);
                    SetWindowPos(g_hwnd, NULL, 0, 0, r0.right - r0.left, r0.bottom - r0.top,
                                 SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
                    dbg("[nw] test: NW_RESIZE %s posted\n", rs);
                }
            }
        }
        if (!snap2_done && (getenv("NW_RESIZE") || getenv("NW_CLICK2"))) {
            /* 第二帧分支（snap2）需要一次重绘才会执行；空闲应用无动画不重绘，
             * 强制补一帧让 NW_RESIZE/NW_CLICK2 钩子可达 */
            InvalidateRect(g_hwnd, NULL, FALSE);
        }
    } else if (!snap2_done && (getenv("NW_CLICK") || getenv("NW_SCROLL") || getenv("NW_SEL") || getenv("NW_CLICK2") || getenv("NW_KEY") || getenv("NW_TOUCH") || getenv("NW_RESIZE"))) {
        snap2_done = 1;
        if (getenv("NW_DUMP")) nw_dump_nodes(); /* 动作后几何：点击/按键已处理 */
        const char* touch = getenv("NW_TOUCH");
        if (touch) {
            int cx = 0, cy = 0;
            if (sscanf(touch, "%d,%d", &cx, &cy) == 2) {
                dbg("[nw] test: synth touchend at %d,%d\n", cx, cy);
                touch_fire((double)cx, (double)cy, hit_test((float)cx, (float)cy), 1);
            }
        }
        const char* click2 = getenv("NW_CLICK2");
        if (click2) {
            int cx = 0, cy = 0;
            if (sscanf(click2, "%d,%d", &cx, &cy) == 2) {
                dbg("[nw] test: synth click2 at %d,%d\n", cx, cy);
                PostMessageW(g_hwnd, WM_LBUTTONDOWN, MK_LBUTTON, MAKELPARAM(cx, cy));
                PostMessageW(g_hwnd, WM_LBUTTONUP, 0, MAKELPARAM(cx, cy));
            }
            return;
        }
        if (getenv("NW_COPYTEST")) {
            wchar_t* s = gather_selection();
            if (s) {
                int u8len = WideCharToMultiByte(CP_UTF8, 0, s, -1, NULL, 0, NULL, NULL);
                char* u8 = (char*)malloc((size_t)u8len);
                if (u8) {
                    WideCharToMultiByte(CP_UTF8, 0, s, -1, u8, u8len, NULL, NULL);
                    dbg("[nw] clipboard-test: \"%s\"\n", u8);
                    free(u8);
                }
                free(s);
            } else {
                dbg("[nw] clipboard-test: <empty selection>\n");
            }
        }
        const char* snap2 = getenv("NW_SNAPSHOT2");
        /* NW_RESIZE 的 SetWindowPos 已在第一帧分支做过（WM_SIZE → relayout）；
         * 这里只负责快照重绘后的新帧 */
        if (snap2) nw_snapshot_bmp(snap2);
        PostMessageW(g_hwnd, WM_CLOSE, 0, 0); /* 测试结束自动退出 */
    } else if (!snap3_done && getenv("NW_CLICK2")) {
        snap3_done = 1;
        const char* snap3 = getenv("NW_SNAPSHOT3");
        if (snap3) nw_snapshot_bmp(snap3);
        PostMessageW(g_hwnd, WM_CLOSE, 0, 0);
    }
}

/* ===== 快照（软件自截图；DC RT + WIC encode） ===== */
void nw_snapshot_bmp(const char* path) {
    if (!g_hwnd || !g_root) return;
    IWICImagingFactory* f = wic_factory();
    if (!f) return;
    HDC dc = GetDC(g_hwnd);
    if (!dc) return;
    ID2D1DCRenderTarget* dcrt = NULL;
    D2D1_RENDER_TARGET_PROPERTIES props = {
        .type = D2D1_RENDER_TARGET_TYPE_SOFTWARE,
        .pixelFormat = { DXGI_FORMAT_B8G8R8A8_UNORM, D2D1_ALPHA_MODE_PREMULTIPLIED },
        .dpiX = 0, .dpiY = 0,
        .usage = D2D1_RENDER_TARGET_USAGE_NONE,
        .minLevel = D2D1_FEATURE_LEVEL_DEFAULT,
    };
    if (g_d2f->lpVtbl->CreateDCRenderTarget(g_d2f, &props, &dcrt) != S_OK || !dcrt) {
        dbg("[nw] snapshot: CreateDCRenderTarget 失败\n");
        ReleaseDC(g_hwnd, dc);
        return;
    }
    RECT rc = { 0, 0, g_cw, g_ch };
    dcrt->lpVtbl->BindDC(dcrt, dc, &rc); /* 绑定即候选；真正取帧走下方 BitBlt 窗口 DC */
    /* 帧内容来自窗口 DC（主 RT 已渲染的最后一帧），无需也无法在 DC RT 重画——
     * 全局画刷/位图属主 RT 资源域，跨 RT 使用报 D2DERR_WRONG_RESOURCE_DOMAIN */
    dcrt->lpVtbl->Base.Base.Base.Release((IUnknown*)dcrt);
    dcrt = NULL;

    /* 编码到文件（.png / .bmp）：GDI 位图拷贝帧 + WIC encode */
    {
        /* GDI 截帧：BitBlt 到内存 DC 再 WIC 包 PNG */
        HDC mem = CreateCompatibleDC(dc);
        HBITMAP hb = CreateCompatibleBitmap(dc, g_cw, g_ch);
        HGDIOBJ old = SelectObject(mem, hb);
        BitBlt(mem, 0, 0, g_cw, g_ch, dc, 0, 0, SRCCOPY);
        /* 构造 WIC 位图并逐行拷贝（BGRA premult） */
        IWICBitmap* wb = NULL;
        if (SUCCEEDED(f->lpVtbl->CreateBitmap(f, (UINT32)g_cw, (UINT32)g_ch,
                &GUID_WICPixelFormat32bppBGR, WICBitmapCacheOnLoad, &wb)) && wb) {
            BITMAPINFO bi;
            memset(&bi, 0, sizeof(bi));
            bi.bmiHeader.biSize = sizeof(BITMAPINFOHEADER);
            bi.bmiHeader.biWidth = g_cw;
            bi.bmiHeader.biHeight = -g_ch;
            bi.bmiHeader.biPlanes = 1;
            bi.bmiHeader.biBitCount = 32;
            bi.bmiHeader.biCompression = BI_RGB;
            /* 逐行写入（WIC 无整幅写接口——用 lock） */
            IWICBitmapLock* lock = NULL;
            if (SUCCEEDED(wb->lpVtbl->Lock(wb, NULL, WICBitmapLockWrite, &lock)) && lock) {
                BYTE* pdata = NULL; UINT psize = 0;
                lock->lpVtbl->GetDataPointer(lock, &psize, &pdata);
                if (pdata && psize >= (UINT)(g_cw * g_ch * 4)) {
            for (int y = 0; y < g_ch; y++) {
                BYTE* dst = pdata + (size_t)y * g_cw * 4;
                /* GetDIBits 的 scan 从底部计数——取 g_ch-1-y 才是自顶向下第 y 行，
                 * 直接传 y 会整图垂直翻转（快照倒置挂账） */
                GetDIBits(mem, hb, g_ch - 1 - y, 1, dst, &bi, DIB_RGB_COLORS);
                        /* BGR→不透明（alpha=255；DIB 无 alpha 通道） */
                        for (int x = 0; x < g_cw; x++) dst[x * 4 + 3] = 255;
                    }
                }
                lock->lpVtbl->Release(lock);
            }
            /* 编码器 */
            IWICBitmapEncoder* enc = NULL;
            wchar_t wpath[MAX_PATH];
            MultiByteToWideChar(CP_UTF8, 0, path, -1, wpath, MAX_PATH);
            IWICStream* out = NULL;
            int is_bmp = strstr(path, ".bmp") != NULL;
            const GUID* cont = &GUID_ContainerFormatPng;
            if (is_bmp) cont = &GUID_ContainerFormatBmp;
            if (SUCCEEDED(f->lpVtbl->CreateEncoder(f, cont, NULL, &enc)) && enc) {
                if (SUCCEEDED(f->lpVtbl->CreateStream(f, &out)) && out
                    && SUCCEEDED(out->lpVtbl->InitializeFromFilename(out, wpath, GENERIC_WRITE))) {
                    if (SUCCEEDED(enc->lpVtbl->Initialize(enc, (IStream*)out, WICBitmapEncoderNoCache))) {
                        IWICBitmapFrameEncode* fr = NULL;
                        IPropertyBag2* pb = NULL;
                        if (SUCCEEDED(enc->lpVtbl->CreateNewFrame(enc, &fr, &pb)) && fr) {
                            fr->lpVtbl->Initialize(fr, pb);
                            fr->lpVtbl->WriteSource(fr, (IWICBitmapSource*)wb, NULL);
                            fr->lpVtbl->Commit(fr);
                        }
                        enc->lpVtbl->Commit(enc);
                    }
                }
                if (out) out->lpVtbl->Release(out);
                if (enc) enc->lpVtbl->Release(enc);
            }
            if (wb) wb->lpVtbl->Release(wb);
            dbg("[nw] snapshot -> %s (%dx%d)\n", path, g_cw, g_ch);
        }
        SelectObject(mem, old);
        DeleteObject(hb);
        DeleteDC(mem);
    }
    /* dcrt 已在中段 Release 并置 NULL——此处严禁二次 Release：
     * use-after-free（-O2 下 UB 优化还会吃掉置 NULL），损坏堆栈后
     * 调用方 snapshot 之后的代码随机不执行（点击合成等钩子静默失踪的根源） */
    ReleaseDC(g_hwnd, dc);
}

/* ===== 命中测试 ===== */
static NWNode* hit_test(float px, float py_raw) {
    /* (eff_z, 创建序) 字典序最顶者胜——只按创建序扫会让全屏 fixed 背景层
     * 吞掉 z-index 更高的 logo/nav 点击（曾致 Blog 全页不可点） */
    float py = py_raw;
    NWNode* best = NULL;
    int best_i = -1;
    for (int i = 0; i < g_node_count; i++) {
        NWNode* n = g_nodes[i];
        if (n->detached || n->manual_hidden || n->st.display == 1) continue;
        /* 匿名盒（linebox/floatrow）不参与命中：浏览器语义中匿名盒对指针
         * 事件穿透，命中应落到其内容元素——否则动态创建的行盒注册序最晚，
         * 会在字典序里永久压住按钮等真实控件（ttt 翻页按钮点不到的根因） */
        if (n->tag && (!strcmp(n->tag, "linebox") || !strcmp(n->tag, "floatrow"))) continue;
        if (px >= n->x && px < n->x + n->w && py >= n->y && py < n->y + n->h) {
            if (n->st.pos_kind == 0 && !n->no_scroll) {
                /* 文档流节点按屏幕坐标（布局 y + 滚动偏移）；
                 * fixed/absolute 子树内节点不随滚动（no_scroll） */
                if (!(py_raw >= n->y - g_scroll_y && py_raw < n->y - g_scroll_y + n->h)) continue;
            }
            if (!best || n->eff_z > best->eff_z ||
                (n->eff_z == best->eff_z && i > best_i)) {
                best = n;
                best_i = i;
            }
        }
    }
    return best;
}

static float g_scroll_y_for_hit_val(void) { return g_scroll_y; }
#define g_scroll_y_for_hit() 0 /* 命中统一走屏幕坐标比较（见上），此处占位 */

/* 手型光标区：onclick 节点 */
static int cursor_hand_zone(void) {
    POINT p;
    if (!GetCursorPos(&p) || !g_hwnd) return 0;
    ScreenToClient(g_hwnd, &p);
    NWNode* n = hit_test((float)p.x, (float)p.y);
    for (; n; n = n->parent) if (n->onclick) return 1;
    return 0;
}

/* I 形光标区：指针落在可选文本上（textarea 或普通文本节点） */
static int cursor_text_zone(void) {
    POINT p;
    if (!GetCursorPos(&p) || !g_hwnd) return 0;
    ScreenToClient(g_hwnd, &p);
    if (ta_at((float)p.x, (float)p.y) != NULL) return 1;
    return text_node_at((float)p.x, (float)p.y) != NULL;
}

/* ===== 文本选择（浏览器语义：跨节点拖选 + 选区高亮 + Ctrl+C） =====
 * 锚点/焦点各为 (节点, 字符索引)；节点间先后用 NWNode.order（文档序）。
 * 索引换算走 DWrite HitTestPoint——按比例均分在折行/多行/字距下必然错位。 */
typedef struct { NWNode* node; int idx; } NWSelPos;
static NWSelPos g_sel_a = { NULL, -1 };   /* 锚点（按下处） */
static NWSelPos g_sel_b = { NULL, -1 };   /* 焦点（拖动处） */

static void sel_clear(void) {
    g_sel_a.node = g_sel_b.node = NULL;
    g_sel_a.idx = g_sel_b.idx = -1;
}
static void sel_begin(NWNode* n, int idx) {
    g_sel_a.node = g_sel_b.node = n;
    g_sel_a.idx = g_sel_b.idx = idx;
}
static void sel_extend(NWNode* n, int idx) {
    if (!n) return;
    if (!g_sel_a.node) { sel_begin(n, idx); return; }
    g_sel_b.node = n; g_sel_b.idx = idx;
}

/* 文本绘制原点（与 draw_node_body 同口径：元素带文本时加 border+padding） */
static void text_origin(NWNode* n, float* ox, float* oy) {
    *ox = n->x; *oy = n->y;
    if (n->tag && strcmp(n->tag, "text") != 0) {
        *ox += n->border_w[3] + n->st.padding[3];
        *oy += n->border_w[0] + n->st.padding[0];
    }
}

/* 节点内字符索引：命中点 → 字符边界（DWrite HitTestPoint） */
static int text_index_at(NWNode* n, float px, float py) {
    if (!n || !n->text || n->text_len <= 0) return 0;
    IDWriteTextLayout* l = get_layout(n, n->w > 0 ? n->w : 1.0f);
    if (!l) return 0;
    float ox, oy;
    text_origin(n, &ox, &oy);
    float sy = (n->st.pos_kind == 0 && !n->no_scroll) ? -g_scroll_y : 0.0f;
    BOOL trailing = FALSE, inside = FALSE;
    DWRITE_HIT_TEST_METRICS m;
    if (l->lpVtbl->HitTestPoint(l, px - ox, py - (oy + sy), &trailing, &inside, &m) != S_OK) return 0;
    int idx = (int)m.textPosition + (trailing ? 1 : 0);
    if (idx < 0) idx = 0;
    if (idx > n->text_len) idx = n->text_len;
    return idx;
}

/* 按点取文本节点：文档序最晚者即最上层（与 hit_test 字典序同解） */
static NWNode* text_node_at(float px, float py) {
    NWNode* best = NULL;
    for (int i = 0; i < g_node_count; i++) {
        NWNode* n = g_nodes[i];
        if (!n || !n->text || n->text_len <= 0 || n->is_ta) continue;
        if (n->detached || n->manual_hidden || n->st.display == 1) continue;
        if (n->tag && (!strcmp(n->tag, "linebox") || !strcmp(n->tag, "floatrow"))) continue;
        if (px < n->x || px >= n->x + n->w) continue;
        if (n->st.pos_kind == 0 && !n->no_scroll) {
            if (!(py >= n->y - g_scroll_y && py < n->y - g_scroll_y + n->h)) continue;
        } else if (py < n->y || py >= n->y + n->h) continue;
        int in_ta = 0;
        for (NWNode* p = n; p; p = p->parent) {
            if (p->is_ta) { in_ta = 1; break; }
            /* 表单控件内的文本不可选（浏览器 UA 语义），否则点按钮会拖出选区 */
            if (p->tag && (!strcmp(p->tag, "button") || !strcmp(p->tag, "input") || !strcmp(p->tag, "select"))) { in_ta = 1; break; }
        }
        if (in_ta) continue;
        best = n;
    }
    return best;
}

/* 节点落在选区内的字符区间 [a,b)；不在则返回 0 */
static int sel_range_in(NWNode* n, int* a, int* b) {
    if (!g_sel_a.node || !g_sel_b.node || !n->text || n->text_len <= 0) return 0;
    NWNode *A = g_sel_a.node, *B = g_sel_b.node;
    int ia = g_sel_a.idx, ib = g_sel_b.idx;
    if (A->order > B->order || (A == B && ia > ib)) {
        NWNode* tn = A; A = B; B = tn;
        int t = ia; ia = ib; ib = t;
    }
    if (n->order < A->order || n->order > B->order) return 0;
    *a = (n == A) ? ia : 0;
    *b = (n == B) ? ib : n->text_len;
    if (*a < 0) *a = 0;
    if (*b > n->text_len) *b = n->text_len;
    return (*b > *a);
}

/* 选区高亮：HitTestTextRange 取字形矩形（多行自动逐行分段） */
static void sel_highlight(ID2D1RenderTarget* rt, NWNode* n, IDWriteTextLayout* l, float ox, float oy) {
    int a, b;
    if (!sel_range_in(n, &a, &b)) return;
    UINT32 need = 0;
    /* 两次调用模式：首查数量时容量 0 → E_NOT_SUFFICIENT_BUFFER(0x8007007A) 并回填 need */
    HRESULT hr = l->lpVtbl->HitTestTextRange(l, (UINT32)a, (UINT32)(b - a), 0.0f, 0.0f, NULL, 0, &need);
    if ((hr != S_OK && hr != (HRESULT)0x8007007A) || need == 0) return;
    if (need > 256) need = 256;
    DWRITE_HIT_TEST_METRICS m[256];
    UINT32 got = 0;
    if (l->lpVtbl->HitTestTextRange(l, (UINT32)a, (UINT32)(b - a), 0.0f, 0.0f, m, need, &got) != S_OK) return;
    D2D1_COLOR_F hl = { 0.0f, 0.47f, 0.84f, 0.32f };   /* 选区蓝（半透明，文字仍可读） */
    g_brush->lpVtbl->SetColor(g_brush, &hl);
    for (UINT32 i = 0; i < got; i++) {
        D2D1_RECT_F r = { ox + m[i].left, oy + m[i].top,
                          ox + m[i].left + m[i].width, oy + m[i].top + m[i].height };
        rt->lpVtbl->FillRectangle(rt, &r, (ID2D1Brush*)g_brush);
    }
}

/* 选区文本：按文档序拼接（跨节点以 \r\n 分隔——剪贴板块间语义） */
static wchar_t* gather_selection(void) {
    if (!g_sel_a.node || !g_sel_b.node) return NULL;
    NWNode *A = g_sel_a.node, *B = g_sel_b.node;
    int ia = g_sel_a.idx, ib = g_sel_b.idx;
    if (A->order > B->order || (A == B && ia > ib)) {
        NWNode* tn = A; A = B; B = tn;
        int t = ia; ia = ib; ib = t;
    }
    size_t cap = 256, len = 0;
    wchar_t* out = (wchar_t*)malloc(cap * sizeof(wchar_t));
    int first = 1;
    for (int i = A->order; i <= B->order && i < g_node_count; i++) {
        NWNode* n = g_nodes[i];
        if (!n || !n->text || n->text_len <= 0) continue;
        if (n->detached || n->st.display == 1) continue;
        int a = (n == A) ? ia : 0;
        int b = (n == B) ? ib : n->text_len;
        if (a < 0) a = 0;
        if (b > n->text_len) b = n->text_len;
        if (b <= a) continue;
        size_t add = (size_t)(b - a) + (first ? 0 : 2);
        if (len + add + 2 > cap) {
            while (len + add + 2 > cap) cap *= 2;
            out = (wchar_t*)realloc(out, cap * sizeof(wchar_t));
        }
        if (!first) { out[len++] = L'\r'; out[len++] = L'\n'; }   /* 跨节点换行 */
        first = 0;
        memcpy(out + len, n->text + a, (size_t)(b - a) * sizeof(wchar_t));
        len += (size_t)(b - a);
    }
    if (len == 0) { free(out); return NULL; }
    out[len] = 0;
    return out;
}

static void copy_selection(void) {
    wchar_t* s = gather_selection();
    if (s) { nw_clip_set(s); free(s); }
}

/* ===== textarea ===== */
static NWNode* g_focus = NULL;
static int ta_caret_x(NWNode* n, int idx);
static void ta_move_caret(NWNode* n, int idx);
static void ta_backspace(NWNode* n) {
    if (!n || !n->text || n->text_len == 0 || n->caret == 0) return;
    int a = n->caret - 1;
    for (int i = a; i < n->text_len - 1; i++) n->text[i] = n->text[i + 1];
    n->text_len--;
    n->text[n->text_len] = 0;
    n->caret = a;
    ta_move_caret(n, n->caret);
    nw_touch(n);
}
static void ta_insert(NWNode* n, wchar_t ch) {
    if (!n || n->text_len >= 4096) return;
    for (int i = n->text_len; i > n->caret; i--) n->text[i] = n->text[i - 1];
    n->text[n->caret++] = ch;
    n->text_len++;
    n->text[n->text_len] = 0;
    nw_touch(n);
}
static void ta_move_caret(NWNode* n, int idx) {
    if (!n) return;
    if (idx < 0) idx = 0;
    if (idx > n->text_len) idx = n->text_len;
    n->caret = idx;
    InvalidateRect(g_hwnd, NULL, FALSE);
}
static NWNode* ta_at(float px, float py) {
    NWNode* n = hit_test(px, py);
    for (; n; n = n->parent) if (n->is_ta) return n;
    return NULL;
}
static void ta_click_caret(NWNode* n, float px, float py) {
    if (!n) return;
    ta_move_caret(n, text_index_at(n, px, py));
}
static void ta_draw(ID2D1RenderTarget* rt, NWNode* n, float sy) {
    /* textarea 绘制：文本 + 光标竖线 */
    if (!n->text) return;
    D2D1_COLOR_F c = { n->color[0], n->color[1], n->color[2], n->color_set ? n->color[3] : 1.0f };
    g_brush->lpVtbl->SetColor(g_brush, &c);
    IDWriteTextLayout* l = get_layout(n, n->w > 0 ? n->w : 1.0f);
    if (l) {
        D2D1_POINT_2F origin = { n->x, n->y + sy };
        rt->lpVtbl->DrawTextLayout(rt, origin, l, (ID2D1Brush*)g_brush, D2D1_DRAW_TEXT_OPTIONS_NONE);
    }
    if (g_focus == n) {
        float cx = (float)ta_caret_x(n, n->caret);
        D2D1_COLOR_F cc = { 0, 0, 0, 0.9f };
        g_brush->lpVtbl->SetColor(g_brush, &cc);
        D2D1_RECT_F bar = { cx, n->y + sy + 2, cx + 1.2f, n->y + sy + n->h - 2 };
        rt->lpVtbl->FillRectangle(rt, &bar, (ID2D1Brush*)g_brush);
    }
}

static int ta_caret_x(NWNode* n, int idx) {
    /* 简化：按字符比例（DWrite hit-test 未接） */
    if (!n->text || n->text_len == 0 || n->w <= 0) return (int)n->x;
    return (int)(n->x + (float)idx / (float)n->text_len * n->w);
}

/* ===== hover ===== */
static void hover_update(float px, float py) {
    NWNode* n = hit_test(px, py);
    if (n != g_hover_node) {
        g_hover_node = n;
        nw_restyle_now();
    }
}

/* ===== 媒体段（@media 响应式） ===== */
static const NWMediaSeg* g_segs = NULL;
static int g_nsegs = 0;
static int g_media_variant = 0;
static void (*g_media_restyle)(int variant) = NULL;

/* ===== 多页面导航 ===== */
#define NW_PAGES_CAP 16
static NWPageGen g_pages[NW_PAGES_CAP];
static int g_pages_n = 0;

void nw_nav_bind(const NWPageGen* tab, int n) {
    if (n > NW_PAGES_CAP) n = NW_PAGES_CAP;
    memcpy(g_pages, tab, sizeof(NWPageGen) * n);
    g_pages_n = n;
}

/* 立根（惰性，前向声明在 g_root 声明处）：供 nw_app_run 与一切
 * DOM 查询/操作入口调用——main 在 nw_app_run 之前就可能 query */
void nw_ensure_root(void) {
    if (g_root) return;
    if (g_pages_n > 0) {
        g_cur_page = 0;
        /* root 访问器非 NULL = main 已 build_dom（DOM 永生，严禁二次 build——否则
         * 节点/taffy 全部双份，注册表指向旧树，几何全灭） */
        g_root = g_pages[0].root ? g_pages[0].root() : NULL;
        if (!g_root && g_pages[0].build) {
            g_pages[0].build();
            g_root = g_pages[0].root ? g_pages[0].root() : NULL;
        }
        if (g_pages[0].restyle) g_pages[0].restyle(g_media_variant);
    }
    if (!g_root) {
        /* 单页应用（无页表）：生成代码 build_dom 已建树，取首个 body 为根 */
        for (int i = 0; i < g_node_count; i++)
            if (g_nodes[i]->tag && !strcmp(g_nodes[i]->tag, "body")) { g_root = g_nodes[i]; break; }
    }
    dbg("[nw] ensure_root: root=%p nodes=%d pages=%d\n", (void*)g_root, g_node_count, g_pages_n);
}

int nw_nav_cur(void) { return g_cur_page; }

NWNode* nw_dom_body(void) {
    /* document.body：当前页根节点（JS document.body.classList 等调用） */
    if (g_root) return g_root;
    if (g_pages_n > 0 && g_pages[g_cur_page].root) return g_pages[g_cur_page].root();
    return NULL;
}

void nw_nav_go(int idx) {
    if (idx < 0 || idx >= g_pages_n || idx == g_cur_page) return;
    /* 旧树整树隐藏（保留原 detached 状态——DOM 永生，切回恢复） */
    if (g_root) {
        for (int i = 0; i < g_node_count; i++) {
            NWNode* nd = g_nodes[i];
            if (nd->page_id == g_cur_page) {
                nd->detached_prev = nd->detached;
                nd->detached = 1;
            }
        }
    }
    NWNode* old_root = g_root;
    (void)old_root;
    g_cur_page = idx;
    /* root 访问器非 NULL = 该页 DOM 已建（永生），严禁二次 build */
    g_root = g_pages[idx].root ? g_pages[idx].root() : NULL;
    if (!g_root && g_pages[idx].build) g_pages[idx].build();      /* 首次构建 DOM */
    g_root = g_pages[idx].root ? g_pages[idx].root() : NULL;
    /* 恢复新树（首次构建的页 detached 本为 0） */
    for (int i = 0; i < g_node_count; i++) {
        NWNode* nd = g_nodes[i];
        if (nd->page_id == idx) {
            if (!nd->lb_built || nd->tag) { /* 新建节点 detached_prev=0 */ }
            nd->detached = 0;
        }
    }
    if (g_pages[idx].restyle) g_pages[idx].restyle(g_media_variant);
    nw_restyle_now();  /* display 规则重放 + relayout */
    if (g_pages[idx].init) g_pages[idx].init();
    if (g_pages[idx].title && g_hwnd) SetWindowTextW(g_hwnd, g_pages[idx].title());
    sel_clear();
    if (g_hwnd) InvalidateRect(g_hwnd, NULL, FALSE);
}

/* <a href> UA 默认行为：本地页名 → 切页；http(s) → 系统浏览器；锚点/空 → 忽略 */
void nw_nav_click(NWNode* n) {
    if (!n) return;
    NWAttr* a = NULL;
    for (a = n->attrs; a; a = a->next) if (!strcmp(a->name, "href")) break;
    if (!a || !a->value) return;
    char u8[512];
    int nl = WideCharToMultiByte(CP_UTF8, 0, a->value, -1, u8, 512, NULL, NULL);
    if (nl <= 1) return;
    if (!strncmp(u8, "http://", 7) || !strncmp(u8, "https://", 8)) {
        ShellExecuteW(g_hwnd, L"open", a->value, NULL, NULL, SW_SHOWNORMAL);
        return;
    }
    if (u8[0] == '#') return; /* 锚点 */
    /* 页面名（去 .html / .htm，小写） */
    char name[256];
    snprintf(name, sizeof name, "%s", u8);
    char* dot = strrchr(name, '.');
    if (dot && (!strcmp(dot, ".html") || !strcmp(dot, ".htm"))) *dot = 0;
    for (char* q = name; *q; q++) if (*q >= 'A' && *q <= 'Z') *q += 32;
    for (int i = 0; i < g_pages_n; i++) {
        if (!strcmp(g_pages[i].name, name)) { nw_nav_go(i); return; }
    }
}

/* ===== 媒体段（@media 响应式；定义见导航区前） ===== */
void nw_media_bind(const NWMediaSeg* segs, int n, int initial, void (*restyle)(int variant)) {
    g_segs = segs; g_nsegs = n;
    g_media_variant = initial;
    g_media_restyle = restyle;
}

static int media_seg_find(void) {
    for (int i = 0; i < g_nsegs; i++) {
        const NWMediaSeg* s = &g_segs[i];
        if ((s->wlo <= -1e8f || (float)g_cw > s->wlo) && (s->whi >= 1e8f || (float)g_cw <= s->whi)
            && (s->hlo <= -1e8f || (float)g_ch > s->hlo) && (s->hhi >= 1e8f || (float)g_ch <= s->hhi)) {
            int orient = (g_cw >= g_ch) ? 1 : 2;
            if (s->orient == 0 || s->orient == orient) return i;
        }
    }
    return g_media_variant;
}

static void media_restyle_now(void) {
    int v = g_segs ? media_seg_find() : g_media_variant;
    g_batch_restyle++;
    if (v != g_media_variant || !g_media_restyle) {
        g_media_variant = v;
        if (g_media_restyle) g_media_restyle(v);
        nw_restyle_now();
        if (g_hwnd) InvalidateRect(g_hwnd, NULL, FALSE);
    } else {
        if (g_media_restyle) g_media_restyle(v);
        nw_restyle_now();
        if (g_hwnd) InvalidateRect(g_hwnd, NULL, FALSE);
    }
    g_batch_restyle--;
}

/* ===== classList/:hover 变更后全量重算 ===== */
void nw_restyle_now(void) {
    g_batch_restyle++;
    reapply_display();
    nw_relayout();
    g_batch_restyle--;
    if (g_hwnd) InvalidateRect(g_hwnd, NULL, FALSE);
    { /* 临时探针：body 守卫验证 */
        static int probe = -1;
        if (probe < 0) probe = getenv("NW_PROBE") ? 1 : 0;
        if (probe && g_root) {
            int m = nw_sel_match(g_root, "t:body,c:dark");
            fprintf(stderr, "[nw] probe body: cls='%s' tag=%s guard=%d bg=(%.2f,%.2f,%.2f,%.2f)\n",
                g_root->cls ? g_root->cls : "", g_root->tag ? g_root->tag : "?",
                m, g_root->bg[0], g_root->bg[1], g_root->bg[2], g_root->bg[3]);
        }
    }
}

/* 全量状态重放：行内声明 + display 规则（媒体段切换后由生成代码主导） */
void nw_state_reapply(void) {
    g_batch_restyle++;
    reapply_display();
    nw_relayout();
    g_batch_restyle--;
}

/* ===== 视口 ===== */
void nw_app_set_viewport(int client_w, int client_h) {
    g_cw = client_w; g_ch = client_h;
    g_vw_set = 1;
}

double nw_vw(double x) { return x * (double)g_cw / 100.0; }
double nw_vh(double x) { return x * (double)g_ch / 100.0; }

/* ===== textarea 公共 API ===== */
const wchar_t* nw_ta_get_text(NWNode* n) { return n ? n->text : NULL; }
void nw_ta_set_text(NWNode* n, const wchar_t* text) { nw_dom_set_text(n, text); }

/* ===== 平台桥 ===== */
void* nw_platform_hwnd(void) { return (void*)g_hwnd; }

/* ===== WndProc ===== */
static LRESULT CALLBACK WndProc(HWND hwnd, UINT msg, WPARAM wp, LPARAM lp) {
    switch (msg) {
    case WM_PAINT: {
        PAINTSTRUCT ps;
        HDC dc = BeginPaint(hwnd, &ps);
        if (!g_rt && g_d2f) {
            D2D1_SIZE_U size = { (UINT32)g_cw, (UINT32)g_ch };
            D2D1_RENDER_TARGET_PROPERTIES props = {
                .type = D2D1_RENDER_TARGET_TYPE_DEFAULT,
                .pixelFormat = { DXGI_FORMAT_UNKNOWN, D2D1_ALPHA_MODE_PREMULTIPLIED },
                /* DPI 钉死 96：taffy 布局按物理像素算，绘制必须 1 DIP = 1 px。
                 * 默认取系统 DPI（125% 缩放 = 120）会把全部坐标 ×1.25——
                 * 内容向右下溢出 25%，右下/右上永远裁掉（用户实测根因） */
                .dpiX = 96, .dpiY = 96,
                .usage = D2D1_RENDER_TARGET_USAGE_NONE,
                .minLevel = D2D1_FEATURE_LEVEL_DEFAULT,
            };
            if (g_d2f->lpVtbl->CreateHwndRenderTarget(g_d2f, &props,
                    &(D2D1_HWND_RENDER_TARGET_PROPERTIES){ hwnd, size,
                        D2D1_PRESENT_OPTIONS_NONE }, &g_hrt) == S_OK && g_hrt) {
                g_rt = (ID2D1RenderTarget*)g_hrt;
                g_rt->lpVtbl->CreateSolidColorBrush(g_rt,
                    &(D2D1_COLOR_F){ 0, 0, 0, 1 }, NULL, &g_brush);
            }
        }
        if (g_rt) nw_draw();
        EndPaint(hwnd, &ps);
        return 0;
    }
    case WM_SIZE: {
        int w = LOWORD(lp), h = HIWORD(lp);
        if (w > 0 && h > 0 && (w != g_cw || h != g_ch)) {
            g_cw = w; g_ch = h;
            dbg("[nw] WM_SIZE %dx%d\n", g_cw, g_ch);
            if (g_hrt) {
                D2D1_SIZE_U s = { (UINT32)w, (UINT32)h };
                g_hrt->lpVtbl->Resize(g_hrt, &s);
            }
            nw_relayout();
            /* resize 防抖：静默期到重求媒体段 */
            SetTimer(hwnd, 2, 150, NULL);
        }
        return 0;
    }
    case WM_TIMER: {
        if (wp >= NW_TIMER_ID_BASE) { /* setTimeout 一次性定时器 */
            KillTimer(hwnd, wp);
            UINT idx = (UINT)(wp - NW_TIMER_ID_BASE);
            if (idx < NW_TIMER_MAX && g_timer_fns[idx]) {
                void (*fn)(void) = g_timer_fns[idx];
                g_timer_fns[idx] = NULL;
                fn();
            }
            return 0;
        }
        if (wp == 7) { /* NW_SNAPSHOT_DELAY 到点：延迟截图并退出 */
            KillTimer(hwnd, 7);
            const char* sd_snap = getenv("NW_SNAPSHOT");
            if (sd_snap) nw_snapshot_bmp(sd_snap);
            PostMessageW(hwnd, WM_CLOSE, 0, 0);
            return 0;
        }
        if (wp == 2) { /* resize 静默期到：媒体段重求值 + 全量重建 */
            KillTimer(hwnd, 2);
            media_restyle_now();
            return 0;
        }
        if (wp == 1) {
            float diff = g_scroll_target - g_scroll_y;
            if (diff < 0.1f && diff > -0.1f) {
                g_scroll_y = g_scroll_target;
                KillTimer(hwnd, 1);
            } else {
                g_scroll_y += diff * 0.45f;
            }
            InvalidateRect(hwnd, NULL, FALSE);
        }
        return 0;
    }
    case WM_MOUSEMOVE: {
        int x = (short)LOWORD(lp), y = (short)HIWORD(lp);
        hover_update((float)x, (float)y);
        /* 左键按住拖动 → 扩展选区焦点（浏览器拖选语义） */
        if ((wp & MK_LBUTTON) && g_sel_a.node) {
            NWNode* tn = text_node_at((float)x, (float)y);
            if (tn) { sel_extend(tn, text_index_at(tn, (float)x, (float)y)); InvalidateRect(hwnd, NULL, FALSE); }
        }
        return 0;
    }
    case WM_LBUTTONDOWN: {
        int x = (short)LOWORD(lp), y = (short)HIWORD(lp);
        SetFocus(hwnd);
        if (getenv("NW_HITBOX") && g_mark_count < 64) {
            g_marks[g_mark_count][0] = (float)x;
            g_marks[g_mark_count][1] = (float)y;
            g_mark_count++;
        }
        NWNode* hit = hit_test((float)x, (float)y);
        dbg("[nw] click hit: (%d,%d) -> %s#%s onclick=%d\n", x, y,
            hit && hit->tag ? hit->tag : "NULL", hit && hit->id ? hit->id : "-",
            hit && hit->parent ? (hit->parent->onclick ? 1 : 0) : -1);
        NWNode* ta = ta_at((float)x, (float)y);
        if (ta) {
            g_focus = ta;
            ta->focused = 1;
            ta_click_caret(ta, (float)x, (float)y);
            return 0;
        }
        g_focus = NULL;
        /* 文本选择：按下即设锚点（浏览器语义；拖动经 WM_MOUSEMOVE 扩展焦点） */
        {
            NWNode* tn = text_node_at((float)x, (float)y);
            if (tn) sel_begin(tn, text_index_at(tn, (float)x, (float)y));
            else sel_clear();
        }
        /* onclick 冒泡（命中节点沿祖先链找第一个 onclick） */
        for (NWNode* n = hit; n; n = n->parent) {
            if (n->onclick) {
                if (dbg_on()) fprintf(stderr, "[nw] onclick fire on %s#%s\n",
                    n->tag ? n->tag : "?", n->id ? n->id : "-");
                n->onclick(n);
                break;
            }
        }
        if (g_doc_click_fn) g_doc_click_fn((double)x, (double)y, hit);
        InvalidateRect(hwnd, NULL, FALSE);
        return 0;
    }
    case WM_LBUTTONUP: {
        int x = (short)LOWORD(lp), y = (short)HIWORD(lp);
        /* 抬起处即焦点（可跨节点；文本区外抬起保持原焦点） */
        if (g_sel_a.node) {
            NWNode* tn = text_node_at((float)x, (float)y);
            if (tn) sel_extend(tn, text_index_at(tn, (float)x, (float)y));
        }
        InvalidateRect(hwnd, NULL, FALSE);
        return 0;
    }
    case WM_MOUSEWHEEL: {
        int delta = (short)HIWORD(wp);
        g_scroll_target -= delta; /* 每格 120 → 3 行近似 */
        float max_scroll = g_content_h - (float)g_ch;
        if (max_scroll < 0) max_scroll = 0;
        if (g_scroll_target > max_scroll) g_scroll_target = max_scroll;
        if (g_scroll_target < 0.0f) g_scroll_target = 0.0f;
        SetTimer(hwnd, 1, 8, NULL); /* 平滑滚动 */
        return 0;
    }
    case WM_SETCURSOR: {
        if (LOWORD(lp) == HTCLIENT) {
            /* 手型（可点）优先于 I 形（可选文本） */
            SetCursor(LoadCursor(NULL, cursor_hand_zone() ? IDC_HAND
                                    : (cursor_text_zone() ? IDC_IBEAM : IDC_ARROW)));
            return 0;
        }
        break;
    }
    case WM_KEYDOWN: {
        int ctrl = GetKeyState(VK_CONTROL) & 0x8000;
        if (wp == 'C' && ctrl && (!g_focus)) copy_selection();
        if (g_focus) {
            switch (wp) {
            case VK_LEFT:  ta_move_caret(g_focus, g_focus->caret - 1); break;
            case VK_RIGHT: ta_move_caret(g_focus, g_focus->caret + 1); break;
            case VK_HOME:  g_focus->caret = 0; ta_move_caret(g_focus, 0); break;
            case VK_END:   g_focus->caret = g_focus->text_len; ta_move_caret(g_focus, g_focus->text_len); break;
            case VK_BACK:  ta_backspace(g_focus); break;
            default: break;
            }
            if (wp == VK_RETURN) ta_insert(g_focus, L'\n');
        }
        if (g_key_fn) g_key_fn((int)wp);
        if (g_key_event_fn) {
            /* KeyboardEvent.key 命名 */
            wchar_t kb[16];
            const wchar_t* name = NULL;
            if (wp >= 'A' && wp <= 'Z') {
                /* 字母：按 shift 状态（简化小写单字符） */
                SHORT sh = GetKeyState(VK_SHIFT);
                wchar_t c = (wchar_t)(L'a' + (wp - 'A'));
                if (sh & 0x8000) c = (wchar_t)(L'A' + (wp - 'A'));
                kb[0] = c; kb[1] = 0;
                name = kb;
            } else if (wp >= '0' && wp <= '9') {
                kb[0] = (wchar_t)wp; kb[1] = 0;
                name = kb;
            } else if (wp == VK_SPACE) { name = L" "; }
            else if (wp == VK_RETURN) { name = L"Enter"; }
            else if (wp == VK_ESCAPE) { name = L"Escape"; }
            else if (wp == VK_TAB) { name = L"Tab"; }
            else if (wp == VK_BACK) { name = L"Backspace"; }
            else if (wp == VK_LEFT) { name = L"ArrowLeft"; }
            else if (wp == VK_RIGHT) { name = L"ArrowRight"; }
            else if (wp == VK_UP) { name = L"ArrowUp"; }
            else if (wp == VK_DOWN) { name = L"ArrowDown"; }
            if (name) g_key_event_fn(name);
        }
        return 0;
    }
    case WM_CHAR: {
        if (g_focus && wp >= 32 && wp != 127) {
            ta_insert(g_focus, (wchar_t)wp);
            return 0;
        }
        break;
    }
    case WM_CLOSE:
        DestroyWindow(hwnd);
        return 0;
    case WM_DESTROY:
        PostQuitMessage(0);
        return 0;
    }
    return DefWindowProcW(hwnd, msg, wp, lp);
}

/* ===== 应用入口 ===== */
void nw_app_run(const wchar_t* title, int client_w, int client_h) {
    if (!g_vw_set) { g_cw = client_w; g_ch = client_h; }
    QueryPerformanceCounter(&g_t0);
    /* DPI 感知：必须在窗口/GDI 之前。缺失时 DWM 拉伸整窗位图（125% 缩放下
     * 900x640 物理像素被拉到 1125x800）——文字模糊、图形粗糙的唯一根源 */
    SetProcessDPIAware();
    WNDCLASSW wc;
    memset(&wc, 0, sizeof(wc));
    wc.lpfnWndProc = WndProc;
    wc.hCursor = LoadCursor(NULL, IDC_ARROW);
    wc.lpszClassName = L"NowebiewWnd";
    wc.hbrBackground = NULL;
    RegisterClassW(&wc);
    /* 客户区精确 client_w×client_h（AdjustWindowRect）；完整 WS_OVERLAPPEDWINDOW
     * = 可拖边调整大小 + 最大化（剥掉 THICKFRAME 会导致窗口不可缩放） */
    RECT r = { 0, 0, client_w, client_h };
    AdjustWindowRect(&r, WS_OVERLAPPEDWINDOW, FALSE);
    g_hwnd = CreateWindowW(L"NowebiewWnd", title,
        WS_OVERLAPPEDWINDOW,
        CW_USEDEFAULT, CW_USEDEFAULT, r.right - r.left, r.bottom - r.top,
        NULL, NULL, NULL, NULL);
    CoInitialize(NULL);
    {
        D2D1_FACTORY_OPTIONS fo = { D2D1_DEBUG_LEVEL_NONE };
        D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, &IID_ID2D1Factory, &fo, (void**)&g_d2f);
    }
    {
        void* dw = NULL;
        DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED, &IID_IDWriteFactory, (IUnknown**)&dw);
        g_dw = (IDWriteFactory*)dw;
        dbg("[nw] app_run: dwrite factory=%p\n", (void*)g_dw);
    }
    if (!g_tree) g_tree = nw_layout_new();
    nw_ensure_root(); /* 页表立根 / 单页扫 body（查询入口可能已提前立根） */
    nw_restyle_now(); /* display 规则初始重放 + relayout */
    /* onload：DOM 构建后、首帧显示前 */
    if (g_load_fn) g_load_fn();
    nw_relayout();
    ShowWindow(g_hwnd, SW_SHOW);
    UpdateWindow(g_hwnd);
    MSG msg;
    while (GetMessageW(&msg, NULL, 0, 0) > 0) {
        TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
}
