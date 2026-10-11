/* sendkey: SendInput 发单个虚拟键码（用于模态对话框自动化测试）
 * 用法: sendkey <vk> [vk2 ...]   例: sendkey 0x1B (ESC) */
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <stdio.h>
#include <stdlib.h>

int main(int argc, char** argv) {
    if (argc < 2) { fprintf(stderr, "usage: sendkey <vk> [vk...]\n"); return 2; }
    for (int i = 1; i < argc; i++) {
        WORD vk = (WORD)strtoul(argv[i], NULL, 0);
        INPUT in[2] = {0};
        in[0].type = INPUT_KEYBOARD;
        in[0].ki.wVk = vk;
        in[1].type = INPUT_KEYBOARD;
        in[1].ki.wVk = vk;
        in[1].ki.dwFlags = KEYEVENTF_KEYUP;
        if (SendInput(2, in, sizeof(INPUT)) != 2) {
            fprintf(stderr, "SendInput failed for vk=0x%02X\n", vk);
            return 1;
        }
        printf("sent vk=0x%02X\n", vk);
    }
    return 0;
}
