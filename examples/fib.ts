// Nowebiew M0 验收程序
// 覆盖：递归函数、显式返回类型标注、for 循环、复合赋值、
// 字符串拼接、模板字符串、.length、单态化（greet 同时吃 string 和 number）、
// console.log、Math.max

function fib(n: number): number {
  if (n < 2) {
    return n;
  }
  return fib(n - 1) + fib(n - 2);
}

// 参数不标类型 -> 按调用点单态化，下面会生成两个版本
function greet(name) {
  return "你好, " + name + "!";
}

let total = 0;
for (let i = 0; i < 10; i++) {
  total += fib(i);
}

console.log(total);
console.log(fib(20));
console.log(greet("Nowebiew"));
console.log(greet(42));

const who = "世界";
console.log(`Hello ${who}, fib(10)=${fib(10)}, len=${who.length}`);
console.log(Math.max(total, 100));
