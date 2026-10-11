/* realclick.c — 真实鼠标点击（SendInput）：找到 NowebiewApp 窗口，点客户区指定坐标
 * 用法: realclick.exe x y   （客户区坐标，物理像素）
 */
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <stdio.h>

int main(int argc, char** argv) {
    if (argc < 3) { fprintf(stderr, "usage: realclick x y\n"); return 1; }
    int cx = atoi(argv[1]), cy = atoi(argv[2]);

    /* DPI 感知，避免坐标虚拟化 */
    HMODULE u32 = GetModuleHandleW(L"user32.dll");
    BOOL (WINAPI *setCtx)(HANDLE) = (BOOL (WINAPI *)(HANDLE))GetProcAddress(u32, "SetProcessDpiAwarenessContext");
    if (setCtx) setCtx((HANDLE)-4);

    HWND h = FindWindowW(L"NowebiewApp", NULL);
    if (!h) { fprintf(stderr, "window not found\n"); return 1; }

    RECT r; GetClientRect(h, &r);
    POINT origin = {0, 0}; ClientToScreen(h, &origin);
    printf("client %ldx%ld, origin(%ld,%ld)\n", r.right, r.bottom, origin.x, origin.y);

    POINT p = { origin.x + cx, origin.y + cy };
    SetCursorPos(p.x, p.y);
    Sleep(80);

    INPUT in[4]; memset(in, 0, sizeof(in));
    /* 鼠标移动（绝对坐标，0..65535 归一化到虚拟屏幕） */
    int vx = GetSystemMetrics(SM_XVIRTUALSCREEN), vy = GetSystemMetrics(SM_YVIRTUALSCREEN);
    int vw = GetSystemMetrics(SM_CXVIRTUALSCREEN), vh = GetSystemMetrics(SM_CYVIRTUALSCREEN);
    in[0].type = INPUT_MOUSE;
    in[0].mi.dx = (DWORD)((p.x - vx) * 65535 / (vw - 1));
    in[0].mi.dy = (DWORD)((p.y - vy) * 65535 / (vh - 1));
    in[0].mi.dwFlags = MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE;
    in[1].type = INPUT_MOUSE;
    in[1].mi.dwFlags = MOUSEEVENTF_LEFTDOWN;
    in[2].type = INPUT_MOUSE;
    in[2].mi.dwFlags = MOUSEEVENTF_LEFTUP;
    if (SendInput(3, in, sizeof(INPUT)) != 3) { fprintf(stderr, "SendInput failed\n"); return 1; }
    printf("clicked screen(%ld,%ld)\n", p.x, p.y);
    return 0;
}
