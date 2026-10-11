// 暗色模式切换
const themeBtn = document.getElementById('themeBtn');
themeBtn.addEventListener('click', () => {
    document.body.classList.toggle('dark');
    if(document.body.classList.contains('dark')){
        themeBtn.textContent = "切换浅色";
    }else{
        themeBtn.textContent = "切换暗色";
    }
});

// 文章展开/收起
const readMoreBtns = document.querySelectorAll('.read-more');
readMoreBtns.forEach(btn => {
    btn.addEventListener('click', () => {
        const content = btn.previousElementSibling;
        content.classList.toggle('open');
        if(content.classList.contains('open')){
            btn.textContent = "收起";
        }else{
            btn.textContent = "阅读全文";
        }
    })
})
