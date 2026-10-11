/* d2dtest.c — 定位 CreateHwndRenderTarget E_INVALIDARG 的参数组合 */
#include <windows.h>
#include <d2d1.h>
#include <dwrite.h>
#include <stdio.h>

static const GUID NW_IID_ID2D1Factory =
    {0x06152247,0x6f50,0x465a,{0x92,0x45,0x11,0x8b,0xfd,0x3b,0x60,0x07}};

static LRESULT CALLBACK p(HWND h, UINT m, WPARAM w, LPARAM l) { return DefWindowProcW(h, m, w, l); }

int main(void) {
    HMODULE u32 = GetModuleHandleW(L"user32.dll");
    BOOL (WINAPI *sdpi)(HANDLE) = (BOOL (WINAPI *)(HANDLE))GetProcAddress(u32, "SetProcessDpiAwarenessContext");
    if (sdpi) sdpi((HANDLE)-4);
    WNDCLASSW wc = {0};
    wc.lpfnWndProc = p; wc.hInstance = GetModuleHandleW(NULL); wc.lpszClassName = L"D2DTest";
    RegisterClassW(&wc);
    HWND hwnd = CreateWindowExW(0, L"D2DTest", L"t", WS_OVERLAPPEDWINDOW,
        50, 50, 400, 300, NULL, NULL, wc.hInstance, NULL);
    RECT r; GetClientRect(hwnd, &r);
    printf("client %dx%d\n", r.right, r.bottom);

    ID2D1Factory* f = NULL;
    D2D1_FACTORY_OPTIONS opt = { D2D1_DEBUG_LEVEL_NONE };
    HRESULT hr = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, &NW_IID_ID2D1Factory, &opt, (void**)&f);
    printf("factory hr=0x%08lx\n", (unsigned long)hr);
    if (hr != S_OK) return 1;

    struct { int type; int fmt; int alpha; const char* name; } cases[] = {
        { 0, 0, 1, "DEFAULT + UNKNOWN + PREMULTIPLIED" },
        { 0, 87, 1, "DEFAULT + B8G8R8A8_UNORM + PREMULTIPLIED" },
        { 1, 0, 1, "SOFTWARE + UNKNOWN + PREMULTIPLIED" },
        { 1, 87, 1, "SOFTWARE + B8G8R8A8_UNORM + PREMULTIPLIED" },
        { 0, 87, 0, "DEFAULT + B8G8R8A8_UNORM + IGNORE" },
    };
    for (int i = 0; i < 5; i++) {
        D2D1_RENDER_TARGET_PROPERTIES rtp; memset(&rtp, 0, sizeof(rtp));
        rtp.type = (D2D1_RENDER_TARGET_TYPE)cases[i].type;
        rtp.pixelFormat.format = (DXGI_FORMAT)cases[i].fmt;   /* 87 = B8G8R8A8_UNORM */
        rtp.pixelFormat.alphaMode = (D2D1_ALPHA_MODE)cases[i].alpha;
        rtp.usage = D2D1_RENDER_TARGET_USAGE_NONE;
        rtp.minLevel = D2D1_FEATURE_LEVEL_DEFAULT;
        D2D1_HWND_RENDER_TARGET_PROPERTIES hp; memset(&hp, 0, sizeof(hp));
        hp.hwnd = hwnd;
        hp.pixelSize.width = r.right; hp.pixelSize.height = r.bottom;
        hp.presentOptions = D2D1_PRESENT_OPTIONS_NONE;
        ID2D1HwndRenderTarget* rt = NULL;
        hr = f->lpVtbl->CreateHwndRenderTarget(f, &rtp, &hp, &rt);
        printf("%-45s hr=0x%08lx %s\n", cases[i].name, (unsigned long)hr, hr == S_OK ? "OK" : "");
        if (rt) ((IUnknown*)rt)->lpVtbl->Release((IUnknown*)rt);
    }
    return 0;
}
