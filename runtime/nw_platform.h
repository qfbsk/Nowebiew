/* Nowebiew M2 平台运行时 —— 文件读写 + 通用对话框 + 剪贴板 */
#ifndef NW_PLATFORM_H
#define NW_PLATFORM_H

#include <wchar.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ===== 文件（磁盘 UTF-8，内存 wchar_t） =====
 * 成功返回 malloc 缓冲（调用方 free），失败返回 NULL */
wchar_t* nw_fs_read(const wchar_t* path);
/* 成功返回 0，失败返回 -1（内容以 UTF-8 写出） */
int      nw_fs_write(const wchar_t* path, const wchar_t* content);

/* ===== 通用对话框（阻塞模态） =====
 * 返回 malloc 的完整路径，用户取消返回 NULL。
 * filter 形如 L"文本文件 (*.txt);所有文件 (*.*)"，';' 分段自动转嵌入 0 */
wchar_t* nw_dialog_open(const wchar_t* title, const wchar_t* filter);
wchar_t* nw_dialog_save(const wchar_t* title, const wchar_t* filter);

/* ===== 剪贴板（CF_UNICODETEXT） ===== */
void     nw_clip_set(const wchar_t* text);   /* 空串允许，NULL 忽略 */
wchar_t* nw_clip_get(void);                  /* malloc 或 NULL（无文本） */

/* ===== 由 nw_ui.c 提供：主窗口句柄（对话框模态归属） ===== */
void* nw_platform_hwnd(void);

#ifdef __cplusplus
}
#endif
#endif
