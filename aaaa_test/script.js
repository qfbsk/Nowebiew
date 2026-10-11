// 暗色切换（机制验证版：forEach 参数闭包捕获属子集边界，见任务清单 T3）
const themeBtn = document.getElementById('themeBtn');
themeBtn.addEventListener('click', () => {
    document.body.classList.toggle('dark');
    if (document.body.classList.contains('dark')) {
        themeBtn.textContent = "切换浅色";
    } else {
        themeBtn.textContent = "切换暗色";
    }
});

// 文章展开/收起（按钮加 id，避开 forEach 闭包）
const more1 = document.getElementById('more1');
more1.addEventListener('click', () => {
    const content = more1.previousElementSibling;
    content.classList.toggle('open');
    if (content.classList.contains('open')) {
        more1.textContent = "收起";
    } else {
        more1.textContent = "阅读全文";
    }
});
