// Nowebiew M1 验收：计数器
const label = document.getElementById("label");
const incBtn = document.getElementById("inc");
const decBtn = document.getElementById("dec");

let count = 0;

function render() {
    label.textContent = "当前计数：" + String(count);
}

function onInc() {
    count++;
    render();
}

function onDec() {
    count--;
    render();
}

incBtn.onclick = onInc;
decBtn.onclick = onDec;
render();
console.log("Nowebiew 计数器已启动，count =", count);
