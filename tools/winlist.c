/* winlist.c — 列出所有可见顶层窗口（类名/标题/矩形/PID） */
#include <windows.h>
#include <stdio.h>
#include <locale.h>

BOOL CALLBACK cb(HWND h, LPARAM lp) {
    if (!IsWindowVisible(h)) return TRUE;
    wchar_t cls[128] = {0}, txt[256] = {0};
    GetClassNameW(h, cls, 128);
    GetWindowTextW(h, txt, 256);
    RECT r; GetWindowRect(h, &r);
    DWORD pid = 0; GetWindowThreadProcessId(h, &pid);
    printf("pid=%lu class=%ls rect=(%ld,%ld %ldx%ld) title=%ls\n",
        (unsigned long)pid, cls, (long)r.left, (long)r.top, (long)(r.right-r.left), (long)(r.bottom-r.top), txt);
    return TRUE;
}

int main(void) {
    setlocale(0, "");
    HMODULE u32 = GetModuleHandleW(L"user32.dll");
    BOOL (WINAPI *sdpi)(HANDLE) = (BOOL (WINAPI *)(HANDLE))GetProcAddress(u32, "SetProcessDpiAwarenessContext");
    if (sdpi) sdpi((HANDLE)-4);
    EnumWindows(cb, 0);
    return 0;
}
