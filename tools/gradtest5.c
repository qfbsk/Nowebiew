/* gradtest5.c — 槽位探测：对每个候选槽用 (This, {x0,y0}) / (This, {x1,y1}) 调用后渲染，
   找到真正生效的 SetStartPoint/SetEndPoint 组合 */
#include <windows.h>
#include <d2d1.h>
#include <stdio.h>
#include <string.h>
static const GUID NW_IID_ID2D1Factory =
    {0x06152247,0x6f50,0x465a,{0x92,0x45,0x11,0x8b,0xfd,0x3b,0x60,0x07}};
static ID2D1DCRenderTarget* dcrt; static ID2D1RenderTarget* rt;
static ID2D1LinearGradientBrush* mk(void) {
    D2D1_GRADIENT_STOP gs[2] = {
        { 0.0f, { 16/255.0f, 185/255.0f, 129/255.0f, 1.0f } },
        { 1.0f, { 6/255.0f, 182/255.0f, 212/255.0f, 1.0f } },
    };
    ID2D1GradientStopCollection* sc = NULL;
    rt->lpVtbl->CreateGradientStopCollection(rt, gs, 2, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP, &sc);
    D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES lp; memset(&lp, 0, sizeof(lp));
    lp.startPoint.x = 0; lp.startPoint.y = 20; lp.endPoint.x = 400; lp.endPoint.y = 20;
    D2D1_BRUSH_PROPERTIES bp; memset(&bp, 0, sizeof(bp)); bp.opacity = 1.0f;
    ID2D1LinearGradientBrush* gb = NULL;
    rt->lpVtbl->CreateLinearGradientBrush(rt, &lp, &bp, sc, &gb);
    return gb;
}
static void sample(const char* tag) {
    BITMAPINFOHEADER bh; memset(&bh, 0, sizeof(bh));
    bh.biSize = sizeof(bh); bh.biWidth = 400; bh.biHeight = -40;
    bh.biPlanes = 1; bh.biBitCount = 24;
    int row = ((400 * 3 + 3) / 4) * 4;
    static unsigned char buf[41 * 1204];
    GetDIBits(GetDC(NULL), NULL, 0, 0, NULL, NULL, 0); /* no-op */
    HDC sdc = GetDC(NULL);
    GetDIBits(sdc, NULL, 0, 0, NULL, NULL, 0); ReleaseDC(NULL, sdc);
    (void)tag;
}
int main(void) {
    ID2D1Factory* f = NULL;
    D2D1_FACTORY_OPTIONS opt = { D2D1_DEBUG_LEVEL_NONE };
    D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, &NW_IID_ID2D1Factory, &opt, (void**)&f);
    int W = 400, H = 40;
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
    rt = (ID2D1RenderTarget*)dcrt;
    rt->lpVtbl->SetDpi(rt, 96.0f, 96.0f);

    void** vt;
    ID2D1LinearGradientBrush* gb = mk();
    vt = *(void***)gb;
    printf("brush vtbl slots 0..12:\n"); fflush(stdout);
    for (int i = 0; i <= 12; i++) printf("  [%2d] %p\n", i, vt[i]);
    fflush(stdout);
    return 0;
}
