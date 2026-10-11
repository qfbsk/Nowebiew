const like1 = document.getElementById("like1");
const like2 = document.getElementById("like2");
const like3 = document.getElementById("like3");

let c1 = 0;
let c2 = 0;
let c3 = 0;

function doLike1() {
  c1 = c1 + 1;
  like1.textContent = "已赞 · " + String(c1);
}

function doLike2() {
  c2 = c2 + 1;
  like2.textContent = "已赞 · " + String(c2);
}

function doLike3() {
  c3 = c3 + 1;
  like3.textContent = "已赞 · " + String(c3);
}

like1.onclick = doLike1;
like2.onclick = doLike2;
like3.onclick = doLike3;
