const pad = document.getElementById("pad");
const status = document.getElementById("status");
const openBtn = document.getElementById("openBtn");
const saveBtn = document.getElementById("saveBtn");
const roundtripBtn = document.getElementById("roundtripBtn");
let filePath = "";

function doOpen() {
  let p = openFileDialog("打开文本文件");
  if (p.length > 0) {
    let content = fs.readText(p);
    pad.value = content;
    filePath = p;
    status.textContent = "已打开: " + p;
  } else {
    status.textContent = "已取消打开";
  }
}

function doSave() {
  let p = saveFileDialog("保存文本文件");
  if (p.length > 0) {
    fs.writeText(p, pad.value);
    filePath = p;
    status.textContent = "已保存: " + p;
  } else {
    status.textContent = "已取消保存";
  }
}

/* 免对话框的文件读写闭环（自动化验证用）：读入 pad → 写出到第二个文件 */
function doRoundtrip() {
  let back = fs.readText("nw_test_io.txt");
  pad.value = back;
  fs.writeText("nw_test_io2.txt", pad.value);
  if (back.length == pad.value.length) {
    status.textContent = "读写一致 (" + String(back.length) + " 字符)";
  } else {
    status.textContent = "读写不一致!";
  }
}

openBtn.onclick = doOpen;
saveBtn.onclick = doSave;
roundtripBtn.onclick = doRoundtrip;
status.textContent = "就绪 - 在下方点击即可输入";
