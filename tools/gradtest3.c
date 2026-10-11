/* gradtest3.c — GetStartPoint 回读 + WIC 位图对照（排除 GetDIBits 干扰） */
#include <windows.h>
#include <d2d1.h>
#include <stdio.h>
#include <string.h>
static const GUID NW_IID_ID2D1Factory =
    {0x06152247,0x6f50,0x465a,{0x92,0x45,0x11,0x8b,0xfd,0x3b,0x60,0x07}};
int main(void) {
    ID2D1Factory* f = NULL;
    D2D1_FACTORY_OPTIONS opt = { D2D1_DEBUG_LEVEL_NONE };
    D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, &NW_IID_ID2D1Factory, &opt, (void**)&f);
    int W = 400, H = 40;
    ID2D1DCRenderTarget* dcrt = NULL;
    D2D1_RENDER_TARGET_PROPERTIES rtp; memset(&rtp, 0, sizeof(rtp));
    rtp.type = D2D1_RENDER_TARGET_TYPE_SOFTWARE;
    rtp.pixelFormat.format = DXGI_FORMAT_B8G8R8A8_UNORM;
    rtp.pixelFormat.alphaMode = D2D1_ALPHA_MODE_PREMULTIPLIED;
    f->lpVtbl->CreateDCRenderTarget(f, &rtp, &dcrt);
    HDC sdc = GetDC(NULL); HDC mdc = CreateCompatibleDC(sdc);
    HBITMAP bmp = CreateCompatibleBitmap(sdc, W, H); ReleaseDC(NULL, sdc);
    SelectObject(mdc, bmp);
    RECT b = { 0, 0, W, H };
    dcrt->lpVtbl->BindDC(dcrt, mdc, &b);
    ID2D1RenderTarget* rt = (ID2D1RenderTarget*)dcrt;
    rt->lpVtbl->SetDpi(rt, 96.0f, 96.0f);
    D2D1_GRADIENT_STOP gs[2] = {
        { 0.0f, { 16/255.0f, 185/255.0f, 129/255.0f, 1.0f } },
        { 1.0f, { 6/255.0f, 182/255.0f, 212/255.0f, 1.0f } },
    };
    ID2D1GradientStopCollection* sc = NULL;
    rt->lpVtbl->CreateGradientStopCollection(rt, gs, 2, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP, &sc);
    D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES lp; memset(&lp, 0, sizeof(lp));
    D2D1_BRUSH_PROPERTIES bp; memset(&bp, 0, sizeof(bp)); bp.opacity = 1.0f;
    ID2D1LinearGradientBrush* gb = NULL;
    rt->lpVtbl->CreateLinearGradientBrush(rt, &lp, &bp, sc, &gb);
    D2D1_POINT_2F p0 = { 0, 20 }, p1 = { 400, 20 };
    gb->lpVtbl->SetStartPoint(gb, p0);
    gb->lpVtbl->SetEndPoint(gb, p1);
    D2D1_POINT_2F g0 = gb->lpVtbl->GetStartPoint(gb);
    D2D1_POINT_2F g1 = gb->lpVtbl->GetEndPoint(gb);
    printf("readback start=(%.1f,%.1f) end=(%.1f,%.1f)\n", g0.x, g0.y, g1.x, g1.y);
    FLOAT op = gb->lpVtbl->Base.GetOpacity((ID2D1Brush*)gb);
    printf("opacity=%.2f\n", op);
    /* 另外试 EXTEND_MODE 换 MIRROR（排除 clamp 病态行为） */
    rt->lpVtbl->BeginDraw(rt);
    D2D1_RECT_F bar = { 0, 10, 400, 30 };
    rt->lpVtbl->FillRectangle(rt, &bar, (ID2D1Brush*)gb);
    HRESULT hr = rt->lpVtbl->EndDraw(rt, NULL, NULL);
    printf("EndDraw hr=0x%08lx\n", (unsigned long)hr);
    BITMAPINFOHEADER bh; memset(&bh, 0, sizeof(bh));
    bh.biSize = sizeof(bh); bh.biWidth = W; bh.biHeight = -H;
    bh.biPlanes = 1; bh.biBitCount = 24;
    int row = ((W * 3 + 3) / 4) * 4;
    unsigned char* buf = (unsigned char*)malloc((size_t)row * H);
    GetDIBits(mdc, bmp, 0, H, buf, (BITMAPINFO*)&bh, DIB_RGB_COLORS);
    int xs[] = { 2, 60, 200, 340, 398 };
    for (int k = 0; k < 5; k++) {
        unsigned char* p = buf + 20 * row + xs[k] * 3;
        printf("x=%3d BGR=%3d,%3d,%3d\n", xs[k], p[0], p[1], p[2]);
    }
    return 0;
}
