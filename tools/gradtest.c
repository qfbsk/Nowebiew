/* gradtest.c — 最小 D2D 线性渐变复现：SOFTWARE DC RT 上画水平渐变条，输出 BMP。
 * 复现运行时快照路径：DCRenderTarget + temp gradient brush + FillRectangle。 */
#include <windows.h>
#include <d2d1.h>
#include <stdio.h>
#include <string.h>

static const GUID NW_IID_ID2D1Factory =
    {0x06152247,0x6f50,0x465a,{0x92,0x45,0x11,0x8b,0xfd,0x3b,0x60,0x07}};

#define CHK(msg, hr) printf("%s hr=0x%08lx\n", msg, (unsigned long)(hr))

int main(void) {
    ID2D1Factory* f = NULL;
    D2D1_FACTORY_OPTIONS opt = { D2D1_DEBUG_LEVEL_NONE };
    HRESULT hr = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, &NW_IID_ID2D1Factory, &opt, (void**)&f);
    CHK("factory", hr);
    if (hr != S_OK) return 1;

    int W = 400, H = 40;
    ID2D1DCRenderTarget* dcrt = NULL;
    D2D1_RENDER_TARGET_PROPERTIES rtp; memset(&rtp, 0, sizeof(rtp));
    rtp.type = D2D1_RENDER_TARGET_TYPE_SOFTWARE;
    rtp.pixelFormat.format = DXGI_FORMAT_B8G8R8A8_UNORM;
    rtp.pixelFormat.alphaMode = D2D1_ALPHA_MODE_PREMULTIPLIED;
    rtp.usage = D2D1_RENDER_TARGET_USAGE_GDI_COMPATIBLE;
    hr = f->lpVtbl->CreateDCRenderTarget(f, &rtp, &dcrt);
    CHK("CreateDCRenderTarget", hr);
    if (hr != S_OK) return 1;

    HDC screendc = GetDC(NULL);
    HDC memdc = CreateCompatibleDC(screendc);
    HBITMAP bmp = CreateCompatibleBitmap(screendc, W, H);
    ReleaseDC(NULL, screendc);
    HGDIOBJ old = SelectObject(memdc, bmp);
    RECT bounds = { 0, 0, W, H };
    hr = dcrt->lpVtbl->BindDC(dcrt, memdc, &bounds);
    CHK("BindDC", hr);
    ID2D1RenderTarget* rt = (ID2D1RenderTarget*)dcrt;
    rt->lpVtbl->SetDpi(rt, 96.0f, 96.0f);

    D2D1_GRADIENT_STOP gs[2] = {
        { 0.0f, { 16/255.0f, 185/255.0f, 129/255.0f, 1.0f } },   /* green */
        { 1.0f, { 6/255.0f, 182/255.0f, 212/255.0f, 1.0f } },    /* cyan */
    };
    ID2D1GradientStopCollection* sc = NULL;
    hr = rt->lpVtbl->CreateGradientStopCollection(rt, gs, 2, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP, &sc);
    CHK("stops", hr);
    D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES lprop; memset(&lprop, 0, sizeof(lprop));
    D2D1_BRUSH_PROPERTIES bprop; memset(&bprop, 0, sizeof(bprop));
    bprop.opacity = 1.0f;
    ID2D1LinearGradientBrush* gb = NULL;
    hr = rt->lpVtbl->CreateLinearGradientBrush(rt, &lprop, &bprop, sc, &gb);
    CHK("brush", hr);

    rt->lpVtbl->BeginDraw(rt);
    D2D1_RECT_F full = { 0, 0, (float)W, (float)H };
    ID2D1SolidColorBrush* bg = NULL;
    rt->lpVtbl->CreateSolidColorBrush(rt, &(D2D1_COLOR_F){0,0,0,1}, NULL, &bg);
    rt->lpVtbl->FillRectangle(rt, &full, (ID2D1Brush*)bg);

    /* 与运行时一致：创建后 SetStartPoint/SetEndPoint 再填充 */
    D2D1_POINT_2F p0 = { 0.0f, (float)H / 2.0f };
    D2D1_POINT_2F p1 = { (float)W, (float)H / 2.0f };
    gb->lpVtbl->SetStartPoint(gb, p0);
    gb->lpVtbl->SetEndPoint(gb, p1);
    D2D1_RECT_F bar = { 0, 10.0f, (float)W, 30.0f };
    rt->lpVtbl->FillRectangle(rt, &bar, (ID2D1Brush*)gb);
    hr = rt->lpVtbl->EndDraw(rt, NULL, NULL);
    CHK("EndDraw", hr);

    /* 读回像素：x=10 应偏绿，x=W-10 应偏青 */
    BITMAPINFOHEADER bh; memset(&bh, 0, sizeof(bh));
    bh.biSize = sizeof(bh); bh.biWidth = W; bh.biHeight = -H;
    bh.biPlanes = 1; bh.biBitCount = 24; bh.biCompression = BI_RGB;
    int row = ((W * 3 + 3) / 4) * 4;
    unsigned char* buf = (unsigned char*)malloc((size_t)row * H);
    if (GetDIBits(memdc, bmp, 0, H, buf, (BITMAPINFO*)&bh, DIB_RGB_COLORS)) {
        int y = 20;
        int xs[] = { 10, 100, 200, 300, 390 };
        for (int k = 0; k < 5; k++) {
            unsigned char* p = buf + (size_t)y * row + xs[k] * 3;
            printf("x=%3d BGR=%3d,%3d,%3d\n", xs[k], p[0], p[1], p[2]);
        }
        /* 写 BMP 供人工查看 */
        BITMAPFILEHEADER fh; memset(&fh, 0, sizeof(fh));
        fh.bfType = 0x4D42; fh.bfOffBits = 54;
        fh.bfSize = 54 + row * H;
        FILE* fp = fopen("gradtest.bmp", "wb");
        fwrite(&fh, 1, 14, fp);
        fwrite(&bh, 1, 40, fp);
        fwrite(buf, 1, (size_t)row * H, fp);
        fclose(fp);
        printf("gradtest.bmp written\n");
    }
    return 0;
}
