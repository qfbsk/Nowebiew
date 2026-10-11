/* Nowebiew M0 运行时
   原则：极薄的一层，只做 JS 语义到 C 的粘合，不做任何权限/安全拦截。
   M0 字符串不回收；M1 换引用计数或 arena。 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <math.h>
#ifdef _WIN32
#include <windows.h>
#endif
#include "nw_runtime.h"

NWStr nw_slit(const char* s) {
    NWStr r;
    r.len = strlen(s);
    r.data = s;
    return r;
}

/* wchar_t* -> NWStr（UTF-8，malloc；NULL/失败 -> 空串）。M2 平台 API 边界转换 */
NWStr nw_wide_to_str(const wchar_t* w) {
    NWStr r;
    if (!w || !*w) { r.len = 0; r.data = ""; return r; }
    int n = WideCharToMultiByte(CP_UTF8, 0, w, -1, NULL, 0, NULL, NULL);
    if (n <= 1) { r.len = 0; r.data = ""; return r; }
    char* buf = (char*)malloc((size_t)n);
    if (!buf) { fputs("nowebiew: out of memory\n", stderr); abort(); }
    WideCharToMultiByte(CP_UTF8, 0, w, -1, buf, n, NULL, NULL);
    r.len = (size_t)n - 1; /* 不含结尾 0 */
    r.data = buf;
    return r;
}

NWStr nw_cat2(NWStr a, NWStr b) {
    size_t n = a.len + b.len;
    char* buf = (char*)malloc(n + 1);
    if (!buf) { fputs("nowebiew: out of memory\n", stderr); abort(); }
    memcpy(buf, a.data, a.len);
    memcpy(buf + a.len, b.data, b.len);
    buf[n] = '\0';
    NWStr r;
    r.len = n;
    r.data = buf;
    return r;
}

/* number -> string，尽量贴近 JS 的显示习惯：
   整数值不带小数点；其余 %.17g（JS 用最短表示，M0 用 17 位精度，略有差异） */
NWStr nw_num_str(double v) {
    char buf[64];
    if (v == floor(v) && fabs(v) < 1e15) {
        snprintf(buf, sizeof buf, "%.0f", v);
    } else {
        snprintf(buf, sizeof buf, "%.17g", v);
    }
    size_t n = strlen(buf);
    char* out = (char*)malloc(n + 1);
    if (!out) { fputs("nowebiew: out of memory\n", stderr); abort(); }
    memcpy(out, buf, n + 1);
    NWStr r;
    r.len = n;
    r.data = out;
    return r;
}

NWStr nw_bool_str(int b) {
    return b ? nw_slit("true") : nw_slit("false");
}

/* string -> number：JS ToNumber 语义（浏览器对字符串做数值比较/算术时的转换）。
   UTF-8 数字字面量即 ASCII，strtod 直接解析；前导空白合法，失败返回 0（JS NaN 参与比较恒 false 的
   常见场景由调用方语义承担，M0 用 0.0 近似） */
double nw_str_num(NWStr s) {
    return strtod(s.data, NULL);
}

int nw_seq(NWStr a, NWStr b) {
    if (a.len != b.len) return 0;
    return memcmp(a.data, b.data, a.len) == 0;
}

int nw_slt(NWStr a, NWStr b) {
    size_t n = a.len < b.len ? a.len : b.len;
    int c = memcmp(a.data, b.data, n);
    if (c != 0) return c < 0;
    return a.len < b.len;
}

int nw_sgt(NWStr a, NWStr b) {
    return nw_slt(b, a);
}

void nw_pnum(double v) {
    if (v == floor(v) && fabs(v) < 1e15) {
        printf("%.0f", v);
    } else {
        printf("%.17g", v);
    }
}

void nw_pbool(int b) {
    fputs(b ? "true" : "false", stdout);
}

void nw_pstr(NWStr s) {
    fwrite(s.data, 1, s.len, stdout);
}

void nw_init(void) {
#ifdef _WIN32
    SetConsoleOutputCP(65001); /* UTF-8，中文输出不乱码 */
#endif
}
