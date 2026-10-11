// gradcpp.cpp — C++ 模式基准：正确的 COM 继承 vtbl 下线性渐变是否正常
// 用 CreateDIBSection 直接读像素，排除 GetDIBits 干扰
#include <windows.h>
#include <d2d1.h>
#include <stdio.h>
#include <string.h>

static const GUID NW_IID_ID2D1Factory =
    {0x06152247,0x6f50,0x465a,{0x92,0x45,0x11,0x8b,0xfd,0x3b,0x60,0x07}};

static void run(int sw, int set_at_create, const char* tag) {
    ID2D1Factory* f = NULL;
    D2D1_FACTORY_OPTIONS opt = { D2D1_DEBUG_LEVEL_NONE };
    if (FAILED(D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, opt, &f))) {
        printf("%s: factory fail\n", tag); return;
    }
    const int W = 400, H = 40;
    ID2D1DCRenderTarget* dcrt = NULL;
    D2D1_RENDER_TARGET_PROPERTIES rtp;
    memset(&rtp, 0, sizeof(rtp));
    rtp.type = sw ? D2D1_RENDER_TARGET_TYPE_SOFTWARE : D2D1_RENDER_TARGET_TYPE_DEFAULT;
    rtp.pixelFormat.format = DXGI_FORMAT_B8G8R8A8_UNORM;
    rtp.pixelFormat.alphaMode = D2D1_ALPHA_MODE_PREMULTIPLIED;
    if (FAILED(f->CreateDCRenderTarget(&rtp, &dcrt))) { printf("%s: rt fail\n", tag); return; }

    HDC sdc = GetDC(NULL);
    BITMAPINFO bmi; memset(&bmi, 0, sizeof(bmi));
    bmi.bmiHeader.biSize = sizeof(BITMAPINFOHEADER);
    bmi.bmiHeader.biWidth = W; bmi.bmiHeader.biHeight = -H;
    bmi.bmiHeader.biPlanes = 1; bmi.bmiHeader.biBitCount = 32;
    bmi.bmiHeader.biCompression = BI_RGB;
    void* bits = NULL;
    HBITMAP bmp = CreateDIBSection(sdc, &bmi, DIB_RGB_COLORS, &bits, NULL, 0);
    ReleaseDC(NULL, sdc);
    HDC mdc = CreateCompatibleDC(NULL);
    HGDIOBJ old = SelectObject(mdc, bmp);
    RECT b = { 0, 0, W, H };
    dcrt->BindDC(mdc, &b);
    ID2D1RenderTarget* rt = dcrt;
    rt->SetDpi(96.0f, 96.0f);

    D2D1_GRADIENT_STOP gs[2] = {
        { 0.0f, { 16/255.0f, 185/255.0f, 129/255.0f, 1.0f } },  // emerald
        { 1.0f, {  6/255.0f, 182/255.0f, 212/255.0f, 1.0f } },  // cyan
    };
    ID2D1GradientStopCollection* sc = NULL;
    rt->CreateGradientStopCollection(gs, 2, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP, &sc);
    D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES lp;
    memset(&lp, 0, sizeof(lp));
    if (set_at_create) { lp.startPoint.x = 0; lp.startPoint.y = 20; lp.endPoint.x = 400; lp.endPoint.y = 20; }
    D2D1_BRUSH_PROPERTIES bp; memset(&bp, 0, sizeof(bp)); bp.opacity = 1.0f;
    ID2D1LinearGradientBrush* gb = NULL;
    rt->CreateLinearGradientBrush(&lp, &bp, sc, &gb);
    if (!set_at_create) {
        gb->SetStartPoint(D2D1::Point2F(0, 20));
        gb->SetEndPoint(D2D1::Point2F(400, 20));
    }
    D2D1_POINT_2F g0 = gb->GetStartPoint();
    D2D1_POINT_2F g1 = gb->GetEndPoint();
    rt->BeginDraw();
    D2D1_RECT_F bar = { 0, 10, 400, 30 };
    rt->FillRectangle(&bar, gb);
    HRESULT hr = rt->EndDraw(0, 0);

    unsigned char* p8 = (unsigned char*)bits;
    auto px = [&](int x, int y) {
        unsigned char* p = p8 + y * W * 4 + x * 4;
        return (p[2] << 16) | (p[1] << 8) | p[0]; // RRGGBB
    };
    printf("%-26s hr=0x%08lx start=(%.0f,%.0f) end=(%.0f,%.0f)  x=10:%06x x=200:%06x x=390:%06x\n",
        tag, (unsigned long)hr, g0.x, g0.y, g1.x, g1.y, px(10,20), px(200,20), px(390,20));
    fflush(stdout);
    SelectObject(mdc, old);
    DeleteDC(mdc); DeleteObject(bmp);
}

int main() {
    printf("expected: 10a9b981(绿) -> 06b6d4(青)\n");
    run(1, 1, "SW  + set at create");
    run(1, 0, "SW  + set after create");
    run(0, 1, "HW  + set at create");
    run(0, 0, "HW  + set after create");
    return 0;
}
