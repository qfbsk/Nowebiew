/* baseline.c — 最小 Win32 空窗口，测系统内存地板 */
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
int WINAPI WinMain(HINSTANCE hi, HINSTANCE prev, LPSTR cmd, int show) {
    (void)prev; (void)cmd;
    WNDCLASSW wc; memset(&wc, 0, sizeof(wc));
    wc.lpfnWndProc = DefWindowProcW;
    wc.hInstance = hi; wc.lpszClassName = L"Baseline";
    RegisterClassW(&wc);
    HWND h = CreateWindowExW(0, L"Baseline", L"baseline", WS_OVERLAPPEDWINDOW,
        CW_USEDEFAULT, CW_USEDEFAULT, 900, 640, NULL, NULL, hi, NULL);
    (void)h;
    ShowWindow(h, show);
    MSG msg;
    while (GetMessageW(&msg, NULL, 0, 0) > 0) { TranslateMessage(&msg); DispatchMessageW(&msg); }
    return 0;
}
