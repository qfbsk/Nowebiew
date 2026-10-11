// Nowebiew M0 - AST 定义与类型

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Ty {
    Num,   // number -> double
    Bool,  // boolean -> int
    Str,   // string -> NWStr（值语义 struct）
    Void,
    Elem,  // Element -> NWNode*（M1：来自 document.getElementById）
    Event, // Event -> 不落地为 C 值；仅作为事件处理器参数存在，
           // 展开为 (clientX, clientY, target) 三个 C 参数（见 codegen）
    ElemList,  // Element[] -> NWList*（querySelectorAll 结果；堆对象，现阶段不回收）
    StrList,   // string[] -> NWStrList*（正则 exec 的捕获组结果）
    Null,      // null 字面量的流类型：变量经首次真实赋值定型；终态仍为 Null 则报错
}

impl Ty {
    pub fn name(&self) -> &'static str {
        match self {
            Ty::Num => "number",
            Ty::Bool => "boolean",
            Ty::Str => "string",
            Ty::Void => "void",
            Ty::Elem => "Element",
            Ty::Event => "Event",
            Ty::ElemList => "Element[]",
            Ty::StrList => "string[]",
            Ty::Null => "null",
        }
    }
    pub fn mangle(&self) -> &'static str {
        match self {
            Ty::Num => "n",
            Ty::Bool => "b",
            Ty::Str => "s",
            Ty::Void => "v",
            Ty::Elem => "e",
            Ty::Event => "E",
            Ty::ElemList => "L",
            Ty::StrList => "G",
            Ty::Null => "z",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add, Sub, Mul, Div, Mod,
    Eq, Neq, Lt, Gt, Le, Ge,
    And, Or,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp { Neg, Not }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostOp { Inc, Dec }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssignOp { Eq, AddEq, SubEq, MulEq, DivEq }

#[derive(Debug, Clone)]
pub enum Expr {
    Num(f64),
    Bool(bool),
    Str(String),
    Tpl(Vec<TplNode>),
    Ident(String, u32, u32),          // name, line, col
    Unary(UnOp, Box<Expr>),
    PostOp(PostOp, Box<Expr>, u32, u32),
    Binary(BinOp, Box<Expr>, Box<Expr>, u32, u32),
    Ternary(Box<Expr>, Box<Expr>, Box<Expr>),
    Call { callee: String, args: Vec<Expr>, line: u32, col: u32 },
    Length(Box<Expr>, u32, u32),      // e.length（M0 仅 string）
    GetById { id: String, line: u32, col: u32 },  // document.getElementById("x")
    ElemValue { obj: String, line: u32, col: u32 }, // <textarea>.value 读取（M2）
    PropRead { obj: String, prop: String, line: u32, col: u32 }, // e.clientX / e.clientY / e.target（Event 参数成员）
    Null(u32, u32),                    // null 字面量（流类型：Num=NaN / Str="" / 指针=NULL）
    Zero { ty: Ty },                   // check 定型后的零值（null 变量声明初始化：NaN/""/NULL）
    Index { obj: String, index: Box<Expr>, line: u32, col: u32 }, // listVar[i]（ElemList/StrList 变量下标）
    RegexExec { pat: String, arg: Box<Expr>, line: u32, col: u32 }, // /re/.exec(s) -> StrList | null
    RegexTest { pat: String, arg: Box<Expr>, line: u32, col: u32 }, // /re/.test(s) -> boolean
    DatasetRead { base: Box<Expr>, key: String, line: u32, col: u32 }, // x.dataset.key -> string（读 data-key 属性）
    Closest { base: Box<Expr>, sel: String, line: u32, col: u32 },     // x.closest("a, button") -> boolean
    ForEach { list: String, fncall: String, line: u32, col: u32 },     // list.forEach(fn / (el,i)=>{})
}

#[derive(Debug, Clone)]
pub enum TplNode {
    Lit(String),
    Expr(Expr),
}

#[derive(Debug, Clone)]
pub enum Stmt {
    Let {
        name: String,
        explicit: Option<Ty>,
        init: Expr,
        is_const: bool,
        line: u32, col: u32,
    },
    Assign { name: String, op: AssignOp, value: Expr, line: u32, col: u32 },
    // el.textContent = expr / el.onclick = fn（M1：对象必须是 Element 变量）
    MemberAssign { obj: String, prop: String, value: Expr, line: u32, col: u32 },
    // 元素成员赋值，对象为任意 Elem 表达式（getElementById(...) / listVar[i]）：
    // prop = "textContent" | "scrollTop" | "style.<子属性>"
    ElemMemberAssign { base: Box<Expr>, prop: String, value: Expr, line: u32, col: u32 },
    // try { body } catch (e) {...}：子集内无抛出来源，catch 分支编译期剥离（body 照常编译）
    Try { body: Vec<Stmt>, line: u32, col: u32 },
    PostAssign { name: String, op: PostOp, line: u32, col: u32 },
    ExprStmt(Expr),
    If { cond: Expr, then: Box<Stmt>, els: Option<Box<Stmt>> },
    While { cond: Expr, body: Box<Stmt> },
    For {
        init: Option<Box<Stmt>>,
        cond: Option<Expr>,
        step: Option<Box<Stmt>>, // Assign / PostAssign / ExprStmt
        body: Box<Stmt>,
    },
    Return(Option<Expr>, u32, u32),
    Break(u32, u32),
    Continue(u32, u32),
    Block(Vec<Stmt>),
    Empty,
}

#[derive(Debug, Clone)]
pub struct FnDecl {
    pub name: String,
    pub params: Vec<(String, Option<Ty>)>,
    pub ret: Option<Ty>,
    pub body: Vec<Stmt>,
    pub line: u32,
    pub col: u32,
}

#[derive(Debug, Clone)]
pub struct Program {
    pub fns: Vec<FnDecl>,
    pub top: Vec<Stmt>,
}
