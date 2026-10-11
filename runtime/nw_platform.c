/* Nowebiew M2 平台运行时实现 —— 文件 + 对话框 + 剪贴板 */
#define UNICODE
#define _UNICODE
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <commdlg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "nw_platform.h"

/* ===== UTF-8 <-> UTF-16 转换（返回 malloc 缓冲） ===== */
static wchar_t* utf8_to_wide(const char* s) {
    int n = MultiByteToWideChar(CP_UTF8, 0, s, -1, NULL, 0);
    if (n <= 0) return NULL;
    wchar_t* w = (wchar_t*)malloc((size_t)n * sizeof(wchar_t));
    if (!w) return NULL;
    MultiByteToWideChar(CP_UTF8, 0, s, -1, w, n);
    return w;
}
static char* wide_to_utf8(const wchar_t* w) {
    int n = WideCharToMultiByte(CP_UTF8, 0, w, -1, NULL, 0, NULL, NULL);
    if (n <= 0) return NULL;
    char* s = (char*)malloc((size_t)n);
    if (!s) return NULL;
    WideCharToMultiByte(CP_UTF8, 0, w, -1, s, n, NULL, NULL);
    return s;
}

/* ===== 文件 ===== */
wchar_t* nw_fs_read(const wchar_t* path) {
    HANDLE h = CreateFileW(path, GENERIC_READ, FILE_SHARE_READ | FILE_SHARE_WRITE,
                           NULL, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, NULL);
    if (h == INVALID_HANDLE_VALUE) return NULL;
    LARGE_INTEGER sz;
    if (!GetFileSizeEx(h, &sz) || sz.QuadPart > 0x40000000) { CloseHandle(h); return NULL; }
    DWORD bytes = (DWORD)sz.QuadPart;
    char* buf = (char*)malloc((size_t)bytes + 1);
    if (!buf) { CloseHandle(h); return NULL; }
    DWORD got = 0;
    if (!ReadFile(h, buf, bytes, &got, NULL)) { free(buf); CloseHandle(h); return NULL; }
    CloseHandle(h);
    buf[got] = 0;
    /* 跳过 UTF-8 BOM */
    const char* p = buf;
    if (got >= 3 && (unsigned char)p[0] == 0xEF && (unsigned char)p[1] == 0xBB && (unsigned char)p[2] == 0xBF) p += 3;
    wchar_t* w = utf8_to_wide(p);
    free(buf);
    return w;
}

int nw_fs_write(const wchar_t* path, const wchar_t* content) {
    char* u8 = wide_to_utf8(content ? content : L"");
    if (!u8) return -1;
    size_t len = strlen(u8); /* 写出不含结尾 0 */
    HANDLE h = CreateFileW(path, GENERIC_WRITE, 0, NULL, CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, NULL);
    if (h == INVALID_HANDLE_VALUE) { free(u8); return -1; }
    DWORD put = 0;
    int ok = WriteFile(h, u8, (DWORD)len, &put, NULL);
    CloseHandle(h);
    free(u8);
    return ok ? 0 : -1;
}

/* ===== 对话框 ===== */
/* "A;B" -> "A\0B\0\0"（malloc），失败 NULL */
static wchar_t* make_filter(const wchar_t* spec) {
    if (!spec || !*spec) {
        const wchar_t* all = L"所有文件 (*.*)\0*.*\0\0";
        size_t n = wcslen(all);
        wchar_t* out = (wchar_t*)malloc((n + 1) * sizeof(wchar_t));
        if (out) memcpy(out, all, (n + 1) * sizeof(wchar_t));
        return out;
    }
    /* ';' 换成嵌入 0；末尾补双 0 */
    size_t n = wcslen(spec);
    wchar_t* out = (wchar_t*)malloc((n + 3) * sizeof(wchar_t));
    if (!out) return NULL;
    memcpy(out, spec, (n + 1) * sizeof(wchar_t));
    for (size_t i = 0; i < n; i++) if (out[i] == L';') out[i] = 0;
    out[n] = 0; out[n + 1] = 0;
    return out;
}

static wchar_t* run_dialog(HWND owner, const wchar_t* title, const wchar_t* filter, int save) {
    wchar_t* flt = make_filter(filter);
    if (!flt) return NULL;
    wchar_t file[MAX_PATH];
    file[0] = 0;
    OPENFILENAMEW o;
    memset(&o, 0, sizeof(o));
    o.lStructSize = sizeof(o);
    o.hwndOwner = owner;
    o.lpstrFilter = flt;
    o.lpstrFile = file;
    o.nMaxFile = MAX_PATH;
    o.lpstrTitle = title;
    o.Flags = OFN_HIDEREADONLY | (save ? OFN_OVERWRITEPROMPT : OFN_FILEMUSTEXIST);
    BOOL ok = save ? GetSaveFileNameW(&o) : GetOpenFileNameW(&o);
    free(flt);
    if (!ok) return NULL;
    size_t n = wcslen(file);
    wchar_t* out = (wchar_t*)malloc((n + 1) * sizeof(wchar_t));
    if (out) memcpy(out, file, (n + 1) * sizeof(wchar_t));
    return out;
}

/* 对话框归属主窗口（nw_ui.c 实现） */
wchar_t* nw_dialog_open(const wchar_t* title, const wchar_t* filter) {
    return run_dialog((HWND)nw_platform_hwnd(), title, filter, 0);
}
wchar_t* nw_dialog_save(const wchar_t* title, const wchar_t* filter) {
    return run_dialog((HWND)nw_platform_hwnd(), title, filter, 1);
}

/* ===== 剪贴板 ===== */
void nw_clip_set(const wchar_t* text) {
    if (!text) return;
    size_t n = wcslen(text);
    if (!OpenClipboard(NULL)) return;
    EmptyClipboard();
    HGLOBAL hg = GlobalAlloc(GMEM_MOVEABLE, (n + 1) * sizeof(wchar_t));
    if (hg) {
        void* p = GlobalLock(hg);
        if (p) { memcpy(p, text, (n + 1) * sizeof(wchar_t)); GlobalUnlock(hg); }
        SetClipboardData(CF_UNICODETEXT, hg);
    }
    CloseClipboard();
}

wchar_t* nw_clip_get(void) {
    if (!OpenClipboard(NULL)) return NULL;
    HANDLE h = GetClipboardData(CF_UNICODETEXT);
    wchar_t* out = NULL;
    if (h) {
        const wchar_t* p = (const wchar_t*)GlobalLock(h);
        if (p) {
            size_t n = wcslen(p);
            out = (wchar_t*)malloc((n + 1) * sizeof(wchar_t));
            if (out) memcpy(out, p, (n + 1) * sizeof(wchar_t));
            GlobalUnlock(h);
        }
    }
    CloseClipboard();
    return out;
}
