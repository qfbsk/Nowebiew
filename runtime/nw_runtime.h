/* Nowebiew M0 运行时 —— 头文件 */
#ifndef NW_RUNTIME_H
#define NW_RUNTIME_H

#include <stddef.h>
/* 注意：不反向包含 nw_ui.h。包含方向单向化：
 *   nw_runtime.h（NWStr 定义）← nw_ui.h（UI 声明，J2 起用到 NWStr）。
 * nw_ui.h 自行包含本头文件，只含 nw_runtime.h 的文件可传递拿到 UI 声明。 */

/* 字符串：值语义 struct（指针 + 长度），不可变。
   M0 内存模型：nw_cat2/nw_num_str/nw_bool_str 分配的新串不回收（原型阶段）；
   nw_slit 指向静态数据，零分配。 */
typedef struct {
    size_t len;
    const char* data;
} NWStr;

NWStr nw_slit(const char* s);
NWStr nw_cat2(NWStr a, NWStr b);
NWStr nw_num_str(double v);
NWStr nw_bool_str(int b);
double nw_str_num(NWStr s);   /* JS ToNumber：字符串数值化（比较/算术隐式转换） */
int   nw_seq(NWStr a, NWStr b);
int   nw_slt(NWStr a, NWStr b);
int   nw_sgt(NWStr a, NWStr b);
void  nw_pnum(double v);
void  nw_pbool(int b);
void  nw_pstr(NWStr s);
void  nw_init(void);   // Windows 控制台切 UTF-8，中文输出不乱码
const wchar_t* nw_str_to_wide(NWStr s);   /* M1: 结果不回收 */
NWStr nw_wide_to_str(const wchar_t* w);   /* M2: 结果 malloc，NULL -> 空串 */

#endif
