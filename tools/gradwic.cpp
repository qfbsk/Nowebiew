// gradwic.cpp — WIC 位图 RT 对照：完全脱离 GDI interop，直接软渲染到位图
#include <windows.h>
#include <d2d1.h>
#include <d2d1_1.h>
#include <wincodec.h>
#include <stdio.h>
#include <string.h>

int main() {
    ID2D1Factory* f = NULL;
    D2D1_FACTORY_OPTIONS opt = { D2D1_DEBUG_LEVEL_NONE };
    if (FAILED(D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, opt, &f))) { printf("factory fail\n"); return 1; }

    IWICImagingFactory* wic = NULL;
    if (FAILED(CoInitialize(NULL))) { printf("com fail\n"); return 1; }
    if (FAILED(CoCreateInstance(CLSID_WICImagingFactory, NULL, CLSCTX_INPROC_SERVER,
            IID_PPV_ARGS(&wic)))) { printf("wic fail\n"); return 1; }

    const int W = 400, H = 40;
    IWICBitmap* bmp = NULL;
    if (FAILED(wic->CreateBitmap(W, H, GUID_WICPixelFormat32bppPBGRA,
            WICBitmapCacheOnDemand, &bmp))) { printf("bitmap fail\n"); return 1; }

    ID2D1RenderTarget* rt = NULL;
    if (FAILED(f->CreateWicBitmapRenderTarget(bmp, D2D1::RenderTargetProperties(
            D2D1_RENDER_TARGET_TYPE_SOFTWARE,
            D2D1::PixelFormat(DXGI_FORMAT_B8G8R8A8_UNORM, D2D1_ALPHA_MODE_PREMULTIPLIED)),
            &rt))) { printf("rt fail\n"); return 1; }
    rt->SetDpi(96.0f, 96.0f);

    D2D1_GRADIENT_STOP gs[2] = {
        { 0.0f, { 16/255.0f, 185/255.0f, 129/255.0f, 1.0f } },
        { 1.0f, {  6/255.0f, 182/255.0f, 212/255.0f, 1.0f } },
    };
    ID2D1GradientStopCollection* sc = NULL;
    rt->CreateGradientStopCollection(gs, 2, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP, &sc);
    D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES lp = {};
    lp.startPoint = { 0, 20 }; lp.endPoint = { 400, 20 };
    D2D1_BRUSH_PROPERTIES bp = {}; bp.opacity = 1.0f;
    ID2D1LinearGradientBrush* gb = NULL;
    rt->CreateLinearGradientBrush(&lp, &bp, sc, &gb);

    rt->BeginDraw();
    D2D1_RECT_F bar = { 0, 10, 400, 30 };
    rt->FillRectangle(&bar, gb);
    HRESULT hr = rt->EndDraw(0, 0);

    // 锁定 WIC 位图读像素
    IWICBitmapLock* lock = NULL;
    if (SUCCEEDED(bmp->Lock(NULL, WICBitmapLockRead, &lock))) {
        UINT sz = 0; BYTE* data = NULL;
        lock->GetDataPointer(&sz, &data);
        if (data) {
            auto px = [&](int x, int y) {
                BYTE* p = data + y * W * 4 + x * 4;
                return (p[2] << 16) | (p[1] << 8) | p[0];
            };
            printf("WIC SW RT  hr=0x%08lx  x=10:%06x x=100:%06x x=200:%06x x=300:%06x x=390:%06x\n",
                (unsigned long)hr, px(10,20), px(100,20), px(200,20), px(300,20), px(390,20));
            printf("expected    x=10:~10b981(绿) x=390:~06b6d4(青)\n");
        }
        lock->Release();
    }
    fflush(stdout);
    return 0;
}
