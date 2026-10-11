/* winctl.c — 窗口控制：max | restore | move x y | size w h
 * 用途：程序化复现用户的窗口操作 */
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <stdio.h>

int main(int argc, char** argv) {
    if (argc < 2) { fprintf(stderr, "usage: winctl max|restore|move x y|size w h|info\n"); return 2; }
    HMODULE u32 = GetModuleHandleW(L"user32.dll");
    BOOL (WINAPI *setCtx)(HANDLE) = (BOOL (WINAPI *)(HANDLE))GetProcAddress(u32, "SetProcessDpiAwarenessContext");
    if (setCtx) setCtx((HANDLE)-4);
    HWND h = FindWindowW(L"NowebiewApp", NULL);
    if (!h) { fprintf(stderr, "window not found\n"); return 1; }
    if (!strcmp(argv[1], "info")) {
        RECT w, c;
        GetWindowRect(h, &w); GetClientRect(h, &c);
        UINT (WINAPI *gdfw)(HWND) = (UINT (WINAPI *)(HWND))GetProcAddress(u32, "GetDpiForWindow");
        UINT dpi = gdfw ? gdfw(h) : 96;
        printf("win=(%ld,%ld)-(%ld,%ld) client=%ldx%ld dpi=%u\n",
               w.left, w.top, w.right, w.bottom, c.right, c.bottom, dpi);
        return 0;
    }
    if (!strcmp(argv[1], "max")) { PostMessageW(h, WM_SYSCOMMAND, SC_MAXIMIZE, 0); }
    else if (!strcmp(argv[1], "restore")) { PostMessageW(h, WM_SYSCOMMAND, SC_RESTORE, 0); }
    else if (!strcmp(argv[1], "move") && argc >= 4) { SetWindowPos(h, NULL, atoi(argv[2]), atoi(argv[3]), 0, 0, SWP_NOSIZE | SWP_NOZORDER); }
    else if (!strcmp(argv[1], "size") && argc >= 4) {
        RECT r = { 0, 0, atoi(argv[2]), atoi(argv[3]) };
        AdjustWindowRect(&r, WS_OVERLAPPEDWINDOW, FALSE);
        SetWindowPos(h, NULL, 0, 0, r.right - r.left, r.bottom - r.top, SWP_NOMOVE | SWP_NOZORDER);
    } else { fprintf(stderr, "bad args\n"); return 2; }
    printf("ok %s\n", argv[1]);
    return 0;
}
