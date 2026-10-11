// gradrow.cpp — 多行多列采样：垂直轴逐行、水平轴逐列
#include <windows.h>
#include <d2d1.h>
#include <wincodec.h>
#include <stdio.h>
#include <string.h>

static ID2D1Factory* f; static IWICImagingFactory* wic;

static void probe2(const char* tag, int nstops, D2D1_GRADIENT_STOP* gs,
                   D2D1_POINT_2F p0, D2D1_POINT_2F p1) {
    const int W = 400, H = 40;
    IWICBitmap* bmp = NULL;
    wic->CreateBitmap(W, H, GUID_WICPixelFormat32bppPBGRA, WICBitmapCacheOnDemand, &bmp);
    ID2D1RenderTarget* rt = NULL;
    f->CreateWicBitmapRenderTarget(bmp, D2D1::RenderTargetProperties(
        D2D1_RENDER_TARGET_TYPE_SOFTWARE,
        D2D1::PixelFormat(DXGI_FORMAT_B8G8R8A8_UNORM, D2D1_ALPHA_MODE_PREMULTIPLIED)), &rt);
    rt->SetDpi(96.0f, 96.0f);
    ID2D1GradientStopCollection* sc = NULL;
    rt->CreateGradientStopCollection(gs, nstops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP, &sc);
    D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES lp = {};
    lp.startPoint = p0; lp.endPoint = p1;
    D2D1_BRUSH_PROPERTIES bp; memset(&bp, 0, sizeof(bp)); bp.opacity = 1.0f;
    ID2D1LinearGradientBrush* gb = NULL;
    rt->CreateLinearGradientBrush(&lp, &bp, sc, &gb);
    rt->BeginDraw();
    D2D1_RECT_F bar = { 0, 10, 400, 30 };
    rt->FillRectangle(&bar, gb);
    rt->EndDraw(0, 0);
    IWICBitmapLock* lock = NULL;
    if (SUCCEEDED(bmp->Lock(NULL, WICBitmapLockRead, &lock))) {
        UINT sz = 0; BYTE* data = NULL;
        lock->GetDataPointer(&sz, &data);
        printf("%s\n", tag);
        for (int y = 8; y <= 32; y += 4) {
            printf("  y=%2d:", y);
            for (int x = 50; x <= 350; x += 75) {
                BYTE* p = data + y * W * 4 + x * 4;
                printf(" %02x%02x%02x", p[2], p[1], p[0]);
            }
            printf("\n");
        }
        lock->Release();
    }
    rt->Release(); bmp->Release(); sc->Release(); gb->Release();
}

int main() {
    D2D1_FACTORY_OPTIONS opt = { D2D1_DEBUG_LEVEL_NONE };
    D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, opt, &f);
    CoInitialize(NULL);
    CoCreateInstance(CLSID_WICImagingFactory, NULL, CLSCTX_INPROC_SERVER, IID_PPV_ARGS(&wic));

    D2D1_GRADIENT_STOP gs2[2] = {
        { 0.0f, { 16/255.0f, 185/255.0f, 129/255.0f, 1.0f } },
        { 1.0f, {  6/255.0f, 182/255.0f, 212/255.0f, 1.0f } },
    };
    D2D1_GRADIENT_STOP grb[2] = {
        { 0.0f, { 1.0f, 0.0f, 0.0f, 1.0f } },
        { 1.0f, { 0.0f, 0.0f, 1.0f, 1.0f } },
    };
    probe2("h-axis G->C (0,20)->(400,20)", 2, gs2, {0,20}, {400,20});
    probe2("v-axis R->B (200,0)->(200,40)", 2, grb, {200,0}, {200,40});
    return 0;
}
