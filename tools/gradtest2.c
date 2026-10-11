/* gradtest2.c — 隔离：端点设置时机 x RT 类型 对线性渐变的影响 */
#include <windows.h>
#include <d2d1.h>
#include <stdio.h>
#include <string.h>
static const GUID NW_IID_ID2D1Factory =
    {0x06152247,0x6f50,0x465a,{0x92,0x45,0x11,0x8b,0xfd,0x3b,0x60,0x07}};

static void run(int sw, int set_at_create, const char* tag) {
    ID2D1Factory* f = NULL;
    D2D1_FACTORY_OPTIONS opt = { D2D1_DEBUG_LEVEL_NONE };
    if (FAILED(D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, &NW_IID_ID2D1Factory, &opt, (void**)&f))) return;
    int W = 400, H = 40;
    ID2D1DCRenderTarget* dcrt = NULL;
    D2D1_RENDER_TARGET_PROPERTIES rtp; memset(&rtp, 0, sizeof(rtp));
    rtp.type = sw ? D2D1_RENDER_TARGET_TYPE_SOFTWARE : D2D1_RENDER_TARGET_TYPE_DEFAULT;
    rtp.pixelFormat.format = DXGI_FORMAT_B8G8R8A8_UNORM;
    rtp.pixelFormat.alphaMode = D2D1_ALPHA_MODE_PREMULTIPLIED;
    if (FAILED(f->lpVtbl->CreateDCRenderTarget(f, &rtp, &dcrt))) { printf("%s: create fail\n", tag); return; }
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
    if (set_at_create) { lp.startPoint.x = 0; lp.startPoint.y = 20; lp.endPoint.x = 400; lp.endPoint.y = 20; }
    D2D1_BRUSH_PROPERTIES bp; memset(&bp, 0, sizeof(bp)); bp.opacity = 1.0f;
    ID2D1LinearGradientBrush* gb = NULL;
    rt->lpVtbl->CreateLinearGradientBrush(rt, &lp, &bp, sc, &gb);
    if (!set_at_create) {
        D2D1_POINT_2F p0 = { 0, 20 }, p1 = { 400, 20 };
        gb->lpVtbl->SetStartPoint(gb, p0);
        gb->lpVtbl->SetEndPoint(gb, p1);
    }
    rt->lpVtbl->BeginDraw(rt);
    D2D1_RECT_F bar = { 0, 10, 400, 30 };
    rt->lpVtbl->FillRectangle(rt, &bar, (ID2D1Brush*)gb);
    rt->lpVtbl->EndDraw(rt, NULL, NULL);
    BITMAPINFOHEADER bh; memset(&bh, 0, sizeof(bh));
    bh.biSize = sizeof(bh); bh.biWidth = W; bh.biHeight = -H;
    bh.biPlanes = 1; bh.biBitCount = 24;
    int row = ((W * 3 + 3) / 4) * 4;
    unsigned char* buf = (unsigned char*)malloc((size_t)row * H);
    if (GetDIBits(mdc, bmp, 0, H, buf, (BITMAPINFO*)&bh, DIB_RGB_COLORS)) {
        printf("%-28s", tag);
        int xs[] = { 10, 200, 390 };
        for (int k = 0; k < 3; k++) {
            unsigned char* p = buf + 20 * row + xs[k] * 3;
            printf("  x=%3d BGR=%3d,%3d,%3d", xs[k], p[0], p[1], p[2]);
        }
        printf("\n");
    }
    free(buf);
}
int main(void) {
    run(1, 0, "SW  + set after create");
    run(1, 1, "SW  + set at create");
    run(0, 0, "HW  + set after create");
    run(0, 1, "HW  + set at create");
    return 0;
}
