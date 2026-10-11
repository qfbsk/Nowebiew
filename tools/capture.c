/* Nowebiew 辅助工具：抓取 NowebiewApp 窗口客户区为 BMP（零依赖，GDI） */
#include <windows.h>
#include <stdio.h>
#include <stdlib.h>

int main(int argc, char** argv) {
    if (argc < 2) { fprintf(stderr, "usage: capture.exe out.bmp\n"); return 2; }
    /* 与被截窗口同等的 DPI 感知，坐标系一致 */
    HMODULE u32 = GetModuleHandleW(L"user32.dll");
    BOOL (WINAPI *sdpi)(HANDLE) = (BOOL (WINAPI *)(HANDLE))GetProcAddress(u32, "SetProcessDpiAwarenessContext");
    if (sdpi) sdpi((HANDLE)-4);
    HWND hwnd = FindWindowExW(NULL, NULL, L"NowebiewApp", NULL);
    if (!hwnd) { fprintf(stderr, "找不到 NowebiewApp 窗口\n"); return 1; }
    RECT r; GetClientRect(hwnd, &r);
    int w = r.right, h = r.bottom;
    if (w <= 0 || h <= 0) { fprintf(stderr, "窗口客户区无效\n"); return 1; }
    HDC wdc = GetWindowDC(hwnd);
    HDC mem = CreateCompatibleDC(wdc);
    HBITMAP bmp = CreateCompatibleBitmap(wdc, w, h);
    HGDIOBJ old = SelectObject(mem, bmp);
    /* 客户区在屏幕上的绝对位置，直接从屏幕 DC 抓（D2D 硬件表面 PrintWindow 会出伪影） */
    POINT po = { 0, 0 };
    ClientToScreen(hwnd, &po);
    HDC sdc = GetDC(NULL);
    if (!BitBlt(mem, 0, 0, w, h, sdc, po.x, po.y, SRCCOPY)) {
        PrintWindow(hwnd, mem, 1 | 2);
    }
    ReleaseDC(NULL, sdc);
    int row = ((w * 3 + 3) / 4) * 4;
    int datasize = row * h;
    unsigned char* buf = (unsigned char*)malloc(datasize);
    BITMAPINFOHEADER bh; memset(&bh, 0, sizeof(bh));
    bh.biSize = sizeof(bh); bh.biWidth = w; bh.biHeight = -h;
    bh.biPlanes = 1; bh.biBitCount = 24; bh.biCompression = BI_RGB;
    GetDIBits(mem, bmp, 0, h, buf, (BITMAPINFO*)&bh, DIB_RGB_COLORS);

    FILE* f = fopen(argv[1], "wb");
    if (!f) { fprintf(stderr, "无法创建输出文件\n"); return 1; }
    int fsize = 54 + datasize;
    unsigned char hdr[54] = {0};
    hdr[0]='B'; hdr[1]='M';
    *(int*)(hdr+2) = fsize; *(int*)(hdr+10) = 54;
    *(int*)(hdr+14) = 40; *(int*)(hdr+18) = w; *(int*)(hdr+22) = h;
    *(short*)(hdr+26) = 1; *(short*)(hdr+28) = 24;
    *(int*)(hdr+34) = datasize;
    fwrite(hdr, 1, 54, f);
    /* buf 是自上而下，BMP 标准自下而上，翻转写出 */
    for (int y = h - 1; y >= 0; y--) fwrite(buf + y * row, 1, row, f);
    fclose(f);
    fprintf(stderr, "已保存 %dx%d -> %s\n", w, h, argv[1]);
    SelectObject(mem, old);
    DeleteObject(bmp); DeleteDC(mem); ReleaseDC(hwnd, wdc);
    free(buf);
    return 0;
}
