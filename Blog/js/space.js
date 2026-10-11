/* 初始化 加载完成收到广播 获取时间更新主题 */
var m=1;
var date = new Date();     
var hour = date.getHours();       /* 获取小时 */
function initialize() {
    if(( 24>=hour && hour>=18) || ( 0<=hour && hour <6)){           /* 夜间模式 */
        dark();
    }
    else{                                                           /* 白天模式 */
        light();
    } 
}  
function change(){
    if( m==1) {
        dark();      /* 夜间模式 */
    }
    else {
        light();     /* 白天模式 */
    } 
}       
function dark() {           /* 夜间模式 */
    var changelogo=document.getElementById("logo");
    var changenav=document.getElementById("nav4");
    changenav.style.color="rgba(255,255,255,.62)";
    var changenav=document.getElementById("nav3");
    changenav.style.color="rgba(255,255,255,.62)";
    var changenav=document.getElementById("nav2");
    changenav.style.color="rgba(255,255,255,.62)";
    var changenav=document.getElementById("nav1");
    changenav.style.color="rgba(255,255,255,.62)";
    var changem=document.getElementById("m4");
    changem.style.color="rgba(255,255,255,.62)";
    var changem=document.getElementById("m3");
    changem.style.color="rgba(255,255,255,.62)";
    var changem=document.getElementById("m2");
    changem.style.color="rgba(255,255,255,.62)";
    var changem=document.getElementById("m1");
    changem.style.color="rgba(255,255,255,.62)";
    var changedirectory=document.getElementById("directory");
    changedirectory.style.color="rgba(255,255,255,.62)";
    var changetitle=document.getElementById("title1");
    changetitle.style.color="rgba(255,255,255,.62)";
    var changethinking=document.getElementById("thinking1");
    changethinking.style.color="rgba(255,255,255,.62)";
    var changewritedate=document.getElementById("writedate1");
    changewritedate.style.color="rgba(255,255,255,.62)";
    var changetitle=document.getElementById("title2");
    changetitle.style.color="rgba(255,255,255,.62)";
    var changethinking=document.getElementById("thinking2");
    changethinking.style.color="rgba(255,255,255,.62)";
    var changewritedate=document.getElementById("writedate2");
    changewritedate.style.color="rgba(255,255,255,.62)";
    var changetitle=document.getElementById("title3");
    changetitle.style.color="rgba(255,255,255,.62)";
    var changethinking=document.getElementById("thinking3");
    changethinking.style.color="rgba(255,255,255,.62)";
    var changewritedate=document.getElementById("writedate3");
    changewritedate.style.color="rgba(255,255,255,.62)";
    var changebk=document.getElementById("background");
    changelogo.src="images/logohalfwhite.png";
    changebk.style.backgroundImage= "url('images/bk2.jpg')";
    m=2;
}    
function light() {          /* 白天模式 */
    var changelogo=document.getElementById("logo");            
    var changenav=document.getElementById("nav4");
    changenav.style.color="rgba(0,0,0,.62)";
    var changenav=document.getElementById("nav3");
    changenav.style.color="rgba(0,0,0,.62)";
    var changenav=document.getElementById("nav2");
    changenav.style.color="rgba(0,0,0,.62)";
    var changenav=document.getElementById("nav1");
    changenav.style.color="rgba(0,0,0,.62)";
    var changem=document.getElementById("m4");
    changem.style.color="rgba(0,0,0,.62)";
    var changem=document.getElementById("m3");
    changem.style.color="rgba(0,0,0,.62)";
    var changem=document.getElementById("m2");
    changem.style.color="rgba(0,0,0,.62)";
    var changem=document.getElementById("m1");
    changem.style.color="rgba(0,0,0,.62)";
    var changedirectory=document.getElementById("directory");
    changedirectory.style.color="rgba(0,0,0,.62)";
    var changetitle=document.getElementById("title1");
    changetitle.style.color="rgba(0,0,0,.62)";
    var changethinking=document.getElementById("thinking1");
    changethinking.style.color="rgba(0,0,0,.62)";
    var changewritedate=document.getElementById("writedate1");
    changewritedate.style.color="rgba(0,0,0,.62)";
    var changetitle=document.getElementById("title2");
    changetitle.style.color="rgba(0,0,0,.62)";
    var changethinking=document.getElementById("thinking2");
    changethinking.style.color="rgba(0,0,0,.62)";
    var changewritedate=document.getElementById("writedate2");
    changewritedate.style.color="rgba(0,0,0,.62)";
     var changetitle=document.getElementById("title3");
    changetitle.style.color="rgba(0,0,0,.62)";
    var changethinking=document.getElementById("thinking3");
    changethinking.style.color="rgba(0,0,0,.62)";
    var changewritedate=document.getElementById("writedate3");
    changewritedate.style.color="rgba(0,0,0,.62)";
    var changebk=document.getElementById("background");
    changelogo.src="images/logohalfblack.png";
    changebk.style.backgroundImage= "url('images/bk1.jpg')";
    m=1;
}
