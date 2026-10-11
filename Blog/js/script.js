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
    var changebk=document.getElementById("background");
    changelogo.src="images/logohalfwhite.png";
    changebk.style.backgroundImage= "url('images/bk2.jpg')";
    var changecontainer=document.getElementById("container1");
    changecontainer.style.color="rgba(255,255,255,.62)";
    var changecontainer=document.getElementById("container2");
    changecontainer.style.color="rgba(255,255,255,.62)";
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
    var changebk=document.getElementById("background");
    changelogo.src="images/logohalfblack.png";
    changebk.style.backgroundImage= "url('images/bk1.jpg')";
    var changecontainer=document.getElementById("container1");
    changecontainer.style.color="rgba(0,0,0,.62)";
    var changecontainer=document.getElementById("container2");
    changecontainer.style.color="rgba(0,0,0,.62)";
    m=1;
}
function startTime(){
	var today=new Date();
	var h=today.getHours();
	var m=today.getMinutes();
	var s=today.getSeconds();// 在小于10的数字前加一个‘0’
	h=checkTime(h);
	m=checkTime(m);
	s=checkTime(s);
	document.getElementById('txt').innerHTML=h+":"+m+":"+s;
	t=setTimeout(function(){startTime()},500);
}
function checkTime(i){
	if (i<10){
		i="0" + i;
	}
	return i;
}
