/* MSVC CRT shim：让 rust-lld 用 MinGW-w64 的 ucrt 导入库链接 Rust(msvc ABI)。
   仅用于构建 nowebiewc 编译器本身；Nowebiew 编译出的程序是纯 C，由 gcc 原生链接。 */

#include <stdlib.h>

int _fltused = 1;          /* MSVC 浮点标记 */
/* _tls_index 由 mingw tlssup.o 提供真实现（PE TLS directory，loader 填真值），此处不可桩化 */

/* MSVC x64 栈探针：RCX=字节数，探页不改栈。MinGW 的 ___chkstk_ms 名字不同，这里自实现。 */
__asm__(
    ".text\n"
    ".globl __chkstk\n"
    "__chkstk:\n"
    "    push %rcx\n"
    "    push %rax\n"
    "    mov  %rsp, %rax\n"
    "    add  $16, %rax\n"          /* rax = 调用者 rsp */
    "    cmp  $0x1000, %rcx\n"
    "    jb   2f\n"
    "1:  sub  $0x1000, %rax\n"
    "    test %eax, (%rax)\n"
    "    sub  $0x1000, %rcx\n"
    "    cmp  $0x1000, %rcx\n"
    "    jae  1b\n"
    "2:  test %rcx, %rcx\n"
    "    jz   3f\n"
    "    sub  %rcx, %rax\n"
    "    test %eax, (%rax)\n"
    "3:  pop  %rax\n"
    "    pop  %rcx\n"
    "    ret\n"
);

/* C++ RTTI vftable 占位：Rust panic 元数据只引用符号，M0 不跨 C++ 异常 */
__asm__(
    ".section .rdata,\"dr\"\n"
    ".globl \"??_7type_info@@6B@\"\n"
    "\"??_7type_info@@6B@\":\n"
    "    .quad 0\n"
    "    .quad 0\n"
    ".text\n"
);

/* ucrt 导出 _crt_atexit，不导出 atexit；补一个转发 */
int _crt_atexit(void (*f)(void));
int atexit(void (*f)(void)) {
    return _crt_atexit(f);
}

/* mingw TLS 体系引用的辅助符号 */
int _CRT_MT = 1;                /* 线程安全 CRT */
int __mingw_module_is_dll = 0;  /* 主模块是 exe */
void _fpreset(void) { __asm__("fninit"); }
void _assert(const char* msg, const char* file, unsigned line) {
    (void)msg; (void)file; (void)line;
    abort();
}

/* 入口点：无需完整 CRT 初始化，ucrt 流按需初始化；exit 负责刷新 stdio */
extern int main(void);
void mainCRTStartup(void) {
    exit(main());
}
