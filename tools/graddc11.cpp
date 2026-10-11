// graddc11.cpp — D2D1.1 DeviceContext + D3D11 WARP：现代管线渐变验证
#include <windows.h>
#include <d2d1_1.h>
#include <d3d11.h>
#include <wincodec.h>
#include <stdio.h>
#include <string.h>

int main() {
    HRESULT hr;
    // D2D1.1 factory
    ID2D1Factory1* f1 = NULL;
    D2D1_FACTORY_OPTIONS opt = { D2D1_DEBUG_LEVEL_NONE };
    ID2D1Factory* f = NULL;
    D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, opt, &f);
    hr = f->QueryInterface(IID_PPV_ARGS(&f1));
    printf("QI factory1 hr=0x%08lx\n", (unsigned long)hr); fflush(stdout);

    // D3D11 WARP device
    ID3D11Device* d3d = NULL;
    ID3D11DeviceContext* d3dc = NULL;
    D3D_FEATURE_LEVEL fl;
    UINT flags = D3D11_CREATE_DEVICE_BGRA_SUPPORT;
    hr = D3D11CreateDevice(NULL, D3D_DRIVER_TYPE_WARP, NULL, flags, NULL, 0,
                           D3D11_SDK_VERSION, &d3d, &fl, &d3dc);
    printf("D3D11 WARP hr=0x%08lx fl=0x%04x\n", (unsigned long)hr, fl); fflush(stdout);
    if (FAILED(hr)) return 1;

    IDXGIDevice* dxgi = NULL;
    d3d->QueryInterface(IID_PPV_ARGS(&dxgi));
    ID2D1Device* dev = NULL;
    hr = f1->CreateDevice(dxgi, &dev);
    printf("CreateDevice hr=0x%08lx\n", (unsigned long)hr); fflush(stdout);
    ID2D1DeviceContext* dc = NULL;
    hr = dev->CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE, &dc);
    printf("CreateDeviceContext hr=0x%08lx\n", (unsigned long)hr); fflush(stdout);

    // 目标：D2D1.1 位图（CPU 可读 staging 语义）
    const int W = 400, H = 40;
    D2D1_BITMAP_PROPERTIES1 bp1;
    memset(&bp1, 0, sizeof(bp1));
    bp1.pixelFormat.format = DXGI_FORMAT_B8G8R8A8_UNORM;
    bp1.pixelFormat.alphaMode = D2D1_ALPHA_MODE_PREMULTIPLIED;
    bp1.bitmapOptions = D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW;
    D2D1_SIZE_U size = { (UINT32)W, (UINT32)H };
    ID2D1Bitmap1* target = NULL;
    hr = dc->CreateBitmap(size, NULL, 0, bp1, &target);
    printf("CreateBitmap target hr=0x%08lx\n", (unsigned long)hr); fflush(stdout);
    dc->SetTarget(target);
    dc->SetDpi(96.0f, 96.0f);

    D2D1_GRADIENT_STOP gs[2] = {
        { 0.0f, { 16/255.0f, 185/255.0f, 129/255.0f, 1.0f } },
        { 1.0f, {  6/255.0f, 182/255.0f, 212/255.0f, 1.0f } },
    };
    ID2D1GradientStopCollection* sc = NULL;
    dc->CreateGradientStopCollection(gs, 2, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP, &sc);
    D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES lp = {};
    lp.startPoint = { 0, 20 }; lp.endPoint = { 400, 20 };
    D2D1_BRUSH_PROPERTIES bp; memset(&bp, 0, sizeof(bp)); bp.opacity = 1.0f;
    ID2D1LinearGradientBrush* gb = NULL;
    dc->CreateLinearGradientBrush(&lp, &bp, sc, &gb);

    dc->BeginDraw();
    D2D1_RECT_F bar = { 0, 10, 400, 30 };
    dc->FillRectangle(&bar, gb);
    hr = dc->EndDraw();
    printf("EndDraw hr=0x%08lx\n", (unsigned long)hr); fflush(stdout);

    // 用 staging 位图拷出并 Map 读像素
    D2D1_BITMAP_PROPERTIES1 sp1;
    memset(&sp1, 0, sizeof(sp1));
    sp1.pixelFormat = bp1.pixelFormat;
    sp1.bitmapOptions = D2D1_BITMAP_OPTIONS_CPU_READ | D2D1_BITMAP_OPTIONS_CANNOT_DRAW;
    ID2D1Bitmap1* staging = NULL;
    hr = dc->CreateBitmap(size, NULL, 0, sp1, &staging);
    if (SUCCEEDED(hr)) {
        D2D1_POINT_2U zero = { 0, 0 };
        D2D1_RECT_U rect = { 0, 0, (UINT32)W, (UINT32)H };
        hr = staging->CopyFromBitmap(&zero, target, &rect);
        printf("CopyFromBitmap hr=0x%08lx\n", (unsigned long)hr); fflush(stdout);
        D2D1_MAPPED_RECT map;
        hr = staging->Map(D2D1_MAP_OPTIONS_READ, &map);
        if (SUCCEEDED(hr)) {
            printf("D2D1.1 WARP path:/n");
            for (int y = 12; y <= 28; y += 8) {
                printf("  y=%2d:", y);
                for (int x = 50; x <= 350; x += 75) {
                    BYTE* p = map.bits + y * map.pitch + x * 4;
                    printf(" %02x%02x%02x", p[2], p[1], p[0]);
                }
                printf("\n");
            }
            printf("expected: x=50 近 10b981(绿) -> x=350 近 06b6d4(青)\n");
            staging->Unmap();
        }
        staging->Release();
    }
    fflush(stdout);
    return 0;
}
