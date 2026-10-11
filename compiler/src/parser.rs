// Nowebiew M0 - 递归下降解析器
// 语法子集：函数声明、let/const、if/else、while、for、return、break/continue、
// 表达式（算术/比较/逻辑/三元/调用/模板串/.length）、console.log、Math.*、String()

use crate::ast::*;
use crate::lexer::{lex, Tok, Token, TplPart};

pub struct ParseError {
    pub msg: String,
    pub line: u32,
    pub col: u32,
}

struct Parser {
    toks: Vec<Token>,
    pos: usize,
    lambdas: u32,           // 箭头函数/IIFE 匿名函数计数（生成隐藏名 nw_lambda_N）
    pending: Vec<FnDecl>,   // 表达式位置产生的函数（箭头、IIFE），解析结束并入 Program.fns
}

pub fn parse(src: &str) -> Result<Program, ParseError> {
    let toks = lex(src).map_err(|e| ParseError { msg: e.msg, line: e.line, col: e.col })?;
    let mut p = Parser { toks, pos: 0, lambdas: 0, pending: Vec::new() };
    p.parse_program()
}

type PResult<T> = Result<T, ParseError>;

impl Parser {
    fn peek(&self) -> &Tok { &self.toks[self.pos].tok }
    fn line(&self) -> u32 { self.toks[self.pos].line }
    fn col(&self) -> u32 { self.toks[self.pos].col }

    fn bump(&mut self) -> Token { let t = self.toks[self.pos].clone(); if self.pos < self.toks.len() - 1 { self.pos += 1; } t }

    fn eat(&mut self, t: &Tok) -> bool {
        if self.peek() == t { self.bump(); true } else { false }
    }

    fn expect(&mut self, t: &Tok, what: &str) -> PResult<Token> {
        if self.peek() == t { Ok(self.bump()) }
        else {
            Err(ParseError {
                msg: format!(" expected '{}' ,{}", what, self.describe()),
                line: self.line(), col: self.col(),
            })
        }
    }

    fn describe(&self) -> String {
        match self.peek() {
            Tok::Eof => "但已到文件末尾".to_string(),
            Tok::Ident(s) => format!("但看到标识符 '{}'", s),
            Tok::NumLit(v) => format!("但看到数字 {}", v),
            Tok::StrLit(s) => format!("但看到字符串 \"{}\"", s),
            other => format!("但看到符号 {:?}", other),
        }
    }

    fn err_here(&self, msg: impl Into<String>) -> ParseError {
        ParseError { msg: msg.into(), line: self.line(), col: self.col() }
    }

    fn parse_program(&mut self) -> PResult<Program> {
        let mut fns = Vec::new();
        let mut top = Vec::new();
        loop {
            match self.peek() {
                Tok::Eof => break,
                Tok::Function => fns.push(self.parse_fn()?),
                _ => top.push(self.parse_stmt()?),
            }
        }
        // 表达式位置的箭头函数 / IIFE 匿名函数并入
        fns.extend(std::mem::take(&mut self.pending));
        Ok(Program { fns, top })
    }

    fn parse_fn(&mut self) -> PResult<FnDecl> {
        let (line, col) = (self.line(), self.col());
        self.expect(&Tok::Function, "'function'")?;
        let name = match self.bump().tok {
            Tok::Ident(s) => s,
            _ => return Err(self.err_here("函数名必须是标识符")),
        };
        self.parse_fn_rest(name, line, col)
    }

    /// 函数表达式（IIFE 用）：function 关键字未消费，名字可选
    fn parse_fn_expr(&mut self) -> PResult<FnDecl> {
        let (line, col) = (self.line(), self.col());
        self.expect(&Tok::Function, "'function'")?;
        let id = self.lambdas;
        let name = if let Tok::Ident(s) = self.peek().clone() {
            self.bump();
            s
        } else {
            self.lambdas += 1;
            format!("nw_lambda_{}", id)
        };
        self.parse_fn_rest(name, line, col)
    }

    /// 函数剩余部分：(参数)[: 返回类型]{ 体 }——parse_fn/parse_fn_expr 共用
    fn parse_fn_rest(&mut self, name: String, line: u32, col: u32) -> PResult<FnDecl> {
        self.expect(&Tok::LParen, "'('")?;
        let mut params = Vec::new();
        if self.peek() != &Tok::RParen {
            loop {
                let pname = match self.bump().tok {
                    Tok::Ident(s) => s,
                    _ => return Err(self.err_here("参数名必须是标识符")),
                };
                let pty = if self.eat(&Tok::Colon) { Some(self.parse_type()?) } else { None };
                params.push((pname, pty));
                if !self.eat(&Tok::Comma) { break; }
            }
        }
        self.expect(&Tok::RParen, "')'")?;
        let ret = if self.eat(&Tok::Colon) { Some(self.parse_type()?) } else { None };
        let body = self.parse_block()?;
        Ok(FnDecl { name, params, ret, body, line, col })
    }

    fn parse_type(&mut self) -> PResult<Ty> {
        let t = self.bump();
        match t.tok {
            Tok::TNumber => Ok(Ty::Num),
            Tok::TString => Ok(Ty::Str),
            Tok::TBoolean => Ok(Ty::Bool),
            Tok::TVoid => Ok(Ty::Void),
            Tok::Ident(s) if s == "Event" => Ok(Ty::Event),
            _ => Err(ParseError { msg: format!("应为类型（number/string/boolean/void/Event），{}", self.describe_prev(&t)), line: t.line, col: t.col }),
        }
    }

    fn describe_prev(&self, t: &Token) -> String {
        match &t.tok {
            Tok::Ident(s) => format!("但看到 '{}'", s),
            _ => format!("但看到 {:?}", t.tok),
        }
    }

    fn parse_block(&mut self) -> PResult<Vec<Stmt>> {
        self.expect(&Tok::LBrace, "'{'")?;
        let mut out = Vec::new();
        loop {
            if self.eat(&Tok::RBrace) { break; }
            if self.peek() == &Tok::Eof { return Err(self.err_here("缺少 '}'")); }
            out.push(self.parse_stmt()?);
        }
        Ok(out)
    }

    /// ASI（自动分号插入，子集）：'}' 或文件尾前的分号可省略
    fn expect_semi_asi(&mut self, what: &str) -> PResult<()> {
        if self.eat(&Tok::Semi) { return Ok(()); }
        if matches!(self.peek(), Tok::RBrace | Tok::Eof) { return Ok(()); }
        Err(self.err_here(format!("expected '{}'", what)))
    }

    fn parse_stmt(&mut self) -> PResult<Stmt> {
        match self.peek().clone() {
            Tok::LBrace => Ok(Stmt::Block(self.parse_block()?)),
            Tok::Semi => { self.bump(); Ok(Stmt::Empty) }
            Tok::Let | Tok::Const => self.parse_let(true),
            Tok::If => self.parse_if(),
            Tok::While => {
                self.bump();
                self.expect(&Tok::LParen, "'(' after 'while'")?;
                let cond = self.parse_expr()?;
                self.expect(&Tok::RParen, "')'")?;
                let body = self.parse_stmt()?;
                Ok(Stmt::While { cond, body: Box::new(body) })
            }
            Tok::Try => {
                // try { body } catch (e) { ... } —— 子集内无抛出来源，catch 分支剥离
                let (line, col) = (self.line(), self.col());
                self.bump();
                let body = self.parse_block()?;
                if !(self.eat(&Tok::Ident("catch".into()))) {
                    return Err(self.err_here("try 后必须跟 catch (e) { ... }"));
                }
                self.expect(&Tok::LParen, "'(' after 'catch'")?;
                match self.bump().tok {
                    Tok::Ident(_) => {} // 捕获变量忽略（body 不会抛出）
                    _ => return Err(self.err_here("catch 后必须有捕获变量名")),
                }
                self.expect(&Tok::RParen, "')'")?;
                let _catch_body = self.parse_block()?; // 剥离（编译期警告由 check 发出）
                Ok(Stmt::Try { body, line, col })
            }
            Tok::For => self.parse_for(),
            Tok::Return => {
                let (line, col) = (self.line(), self.col());
                self.bump();
                let e = if self.peek() == &Tok::Semi { None } else { Some(self.parse_expr()?) };
                self.expect_semi_asi("';' after 'return'")?;
                Ok(Stmt::Return(e, line, col))
            }
            Tok::Break => {
                let (line, col) = (self.line(), self.col());
                self.bump();
                self.expect_semi_asi("';' after 'break'")?;
                Ok(Stmt::Break(line, col))
            }
            Tok::Continue => {
                let (line, col) = (self.line(), self.col());
                self.bump();
                self.expect_semi_asi("';' after 'continue'")?;
                Ok(Stmt::Continue(line, col))
            }
            _ => {
                let s = self.parse_simple_stmt()?;
                self.expect_semi_asi("';'")?;
                Ok(s)
            }
        }
    }

    // let/const、赋值、后缀自增/自减、普通表达式 —— 不消费结尾分号
    fn parse_simple_stmt(&mut self) -> PResult<Stmt> {
        match self.peek().clone() {
            Tok::Let | Tok::Const => self.parse_let(false),
            Tok::Ident(_) => {
                // 可能是赋值 / 成员赋值 / 下标成员赋值 / 后缀自增 / 表达式
                let save = self.pos;
                let (line, col) = (self.line(), self.col());
                let name = match self.bump().tok {
                    Tok::Ident(s) => s,
                    _ => unreachable!(),
                };
                /* name[idx].prop = v / name[idx].style.sub = v —— 下标赋值目标 */
                if self.peek() == &Tok::LBracket {
                    let idx_pos = self.pos;
                    self.bump(); // [
                    let idx_ok = self.parse_expr().is_ok() && self.expect(&Tok::RBracket, "']'").is_ok();
                    if idx_ok && self.peek() == &Tok::Dot {
                        self.bump(); // .
                        let prop = match self.bump().tok {
                            Tok::Ident(p) => p,
                            _ => { self.pos = save; return Ok(Stmt::ExprStmt(self.parse_expr()?)); }
                        };
                        let (prop, idx_src) = if prop == "style" && self.peek() == &Tok::Dot {
                            self.bump();
                            match self.bump().tok {
                                Tok::Ident(p) => (format!("style.{}", p), None::<String>),
                                _ => { self.pos = save; return Ok(Stmt::ExprStmt(self.parse_expr()?)); }
                            }
                        } else {
                            (prop, None::<String>)
                        };
                        if !["textContent", "scrollTop"].contains(&prop.as_str()) && !prop.starts_with("style.") {
                            self.pos = save;
                            return Ok(Stmt::ExprStmt(self.parse_expr()?));
                        }
                        if self.peek() != &Tok::Eq {
                            self.pos = save;
                            return Ok(Stmt::ExprStmt(self.parse_expr()?));
                        }
                        self.bump(); // Eq
                        let v = self.parse_expr()?;
                        /* 还原下标表达式原文区间内的 token（重建 Index） */
                        let idx_toks: Vec<Token> = self.toks[idx_pos + 1..].iter()
                            .take_while(|t| !matches!(t.tok, Tok::RBracket))
                            .cloned().collect();
                        let mut sub = Parser {
                            toks: {
                                let mut ts = idx_toks;
                                ts.push(Token { tok: Tok::Eof, line: 0, col: 0 });
                                ts
                            },
                            pos: 0,
                            lambdas: 0,
                            pending: Vec::new(),
                        };
                        let idx_expr = sub.parse_expr().map_err(|e| ParseError {
                            msg: format!("下标表达式错误: {}", e.msg), line, col,
                        })?;
                        return Ok(Stmt::ElemMemberAssign {
                            base: Box::new(Expr::Index { obj: name, index: Box::new(idx_expr), line, col }),
                            prop, value: v, line, col,
                        });
                    }
                    self.pos = save;
                    return Ok(Stmt::ExprStmt(self.parse_expr()?));
                }
                match self.peek() {
                    Tok::Eq => {
                        self.bump();
                        let v = self.parse_expr()?;
                        Ok(Stmt::Assign { name, op: AssignOp::Eq, value: v, line, col })
                    }
                    Tok::Dot => {
                        // 多级链：name.prop = v / name.style.sub = v / location.hash = v /
                        // document.body.style.sub = v / document.getElementById("id").prop = v
                        self.bump(); // Dot
                        let mut chain: Vec<String> = Vec::new();
                        chain.push(match self.bump().tok {
                            Tok::Ident(p) => p,
                            _ => { self.pos = save; return Ok(Stmt::ExprStmt(self.parse_expr()?)); }
                        });
                        let mut byid: Option<String> = None;
                        if chain[0] == "getElementById" && name == "document" {
                            // document.getElementById("id") 的调用形参
                            self.expect(&Tok::LParen, "'('")?;
                            let id = match self.bump().tok {
                                Tok::StrLit(s) => s,
                                _ => { self.pos = save; return Ok(Stmt::ExprStmt(self.parse_expr()?)); }
                            };
                            self.expect(&Tok::RParen, "')'")?;
                            byid = Some(id);
                        }
                        while self.peek() == &Tok::Dot {
                            self.bump();
                            match self.bump().tok {
                                Tok::Ident(p) => chain.push(p),
                                _ => { self.pos = save; return Ok(Stmt::ExprStmt(self.parse_expr()?)); }
                            }
                        }
                        if self.peek() != &Tok::Eq {
                            self.pos = save;
                            return Ok(Stmt::ExprStmt(self.parse_expr()?));
                        }
                        self.bump(); // Eq
                        let v = self.parse_expr()?;
                        /* 目标分派 */
                        if let Some(id) = byid {
                            // 链首是 getElementById：[getElementById, prop] 或 [getElementById, style, sub]
                            let mut prop = match chain.len() {
                                2 => chain[1].clone(),
                                3 if chain[1] == "style" => format!("style.{}", chain[2]),
                                _ => { self.pos = save; return Ok(Stmt::ExprStmt(self.parse_expr()?)); }
                            };
                            /* innerHTML：运行时无 HTML 解析器（章程），按纯文本语义处理 */
                            if prop == "innerHTML" { prop = "textContent".to_string(); }
                            if !["textContent", "scrollTop", "onclick"].contains(&prop.as_str()) && !prop.starts_with("style.") {
                                return Err(ParseError { msg: format!("getElementById(...) 目标仅支持 textContent/scrollTop/style.*，不支持 '{}'", prop), line, col });
                            }
                            return Ok(Stmt::ElemMemberAssign {
                                base: Box::new(Expr::GetById { id, line, col }),
                                prop, value: v, line, col,
                            });
                        }
                        if chain.len() == 1 {
                            return Ok(Stmt::MemberAssign { obj: name, prop: chain[0].clone(), value: v, line, col });
                        }
                        /* name.style.<prop> = v（el.style.width 等运行时样式） */
                        if chain.len() == 2 && chain[0] == "style" {
                            return Ok(Stmt::MemberAssign { obj: format!("{}@style", name), prop: chain[1].clone(), value: v, line, col });
                        }
                        /* document.body.style.<prop> = v */
                        if name == "document" && chain.len() == 3 && chain[0] == "body" && chain[1] == "style" {
                            return Ok(Stmt::MemberAssign { obj: "document.body@style".into(), prop: chain[2].clone(), value: v, line, col });
                        }
                        self.pos = save;
                        Ok(Stmt::ExprStmt(self.parse_expr()?))
                    }
                    Tok::PlusEq => { self.bump(); let v = self.parse_expr()?; Ok(Stmt::Assign { name, op: AssignOp::AddEq, value: v, line, col }) }
                    Tok::MinusEq => { self.bump(); let v = self.parse_expr()?; Ok(Stmt::Assign { name, op: AssignOp::SubEq, value: v, line, col }) }
                    Tok::StarEq => { self.bump(); let v = self.parse_expr()?; Ok(Stmt::Assign { name, op: AssignOp::MulEq, value: v, line, col }) }
                    Tok::SlashEq => { self.bump(); let v = self.parse_expr()?; Ok(Stmt::Assign { name, op: AssignOp::DivEq, value: v, line, col }) }
                    Tok::PlusPlus => {
                        self.bump();
                        Ok(Stmt::PostAssign { name, op: PostOp::Inc, line, col })
                    }
                    Tok::MinusMinus => {
                        self.bump();
                        Ok(Stmt::PostAssign { name, op: PostOp::Dec, line, col })
                    }
                    _ => {
                        self.pos = save;
                        Ok(Stmt::ExprStmt(self.parse_expr()?))
                    }
                }
            }
            _ => Ok(Stmt::ExprStmt(self.parse_expr()?)),
        }
    }

    fn parse_let(&mut self, need_semi: bool) -> PResult<Stmt> {
        let (line, col) = (self.line(), self.col());
        let is_const = self.peek() == &Tok::Const;
        self.bump(); // let / const
        let name = match self.bump().tok {
            Tok::Ident(s) => s,
            _ => return Err(self.err_here("变量名必须是标识符")),
        };
        let explicit = if self.eat(&Tok::Colon) { Some(self.parse_type()?) } else { None };
        self.expect(&Tok::Eq, "'='（M0 要求声明即初始化）")?;
        let init = self.parse_expr()?;
        if need_semi { self.expect_semi_asi("';'")?; }
        Ok(Stmt::Let { name, explicit, init, is_const, line, col })
    }

    fn parse_if(&mut self) -> PResult<Stmt> {
        self.bump(); // if
        self.expect(&Tok::LParen, "'(' after 'if'")?;
        let cond = self.parse_expr()?;
        self.expect(&Tok::RParen, "')'")?;
        let then = Box::new(self.parse_stmt()?);
        let els = if self.eat(&Tok::Else) { Some(Box::new(self.parse_stmt()?)) } else { None };
        Ok(Stmt::If { cond, then, els })
    }

    fn parse_for(&mut self) -> PResult<Stmt> {
        self.bump(); // for
        self.expect(&Tok::LParen, "'(' after 'for'")?;
        let init = if self.peek() == &Tok::Semi {
            self.bump();
            None
        } else {
            Some(Box::new(self.parse_simple_stmt()?))
        };
        self.expect(&Tok::Semi, "';'")?;
        let cond = if self.peek() == &Tok::Semi { None } else { Some(self.parse_expr()?) };
        self.expect(&Tok::Semi, "';'")?;
        let step = if self.peek() == &Tok::RParen { None } else { Some(Box::new(self.parse_simple_stmt()?)) };
        self.expect(&Tok::RParen, "')'")?;
        let body = self.parse_stmt()?;
        Ok(Stmt::For { init, cond, step, body: Box::new(body) })
    }

    // ============ 表达式 ============

    pub fn parse_expr(&mut self) -> PResult<Expr> {
        self.parse_ternary()
    }

    fn parse_ternary(&mut self) -> PResult<Expr> {
        let cond = self.parse_or()?;
        if self.eat(&Tok::Question) {
            let a = self.parse_ternary()?;
            self.expect(&Tok::Colon, "':' in ternary")?;
            let b = self.parse_ternary()?;
            Ok(Expr::Ternary(Box::new(cond), Box::new(a), Box::new(b)))
        } else {
            Ok(cond)
        }
    }

    fn parse_or(&mut self) -> PResult<Expr> {
        let mut l = self.parse_and()?;
        while self.eat(&Tok::PipePipe) {
            let r = self.parse_and()?;
            let (line, col) = (self.line(), self.col());
            l = Expr::Binary(BinOp::Or, Box::new(l), Box::new(r), line, col);
        }
        Ok(l)
    }

    fn parse_and(&mut self) -> PResult<Expr> {
        let mut l = self.parse_equality()?;
        while self.eat(&Tok::AmpAmp) {
            let r = self.parse_equality()?;
            let (line, col) = (self.line(), self.col());
            l = Expr::Binary(BinOp::And, Box::new(l), Box::new(r), line, col);
        }
        Ok(l)
    }

    fn parse_equality(&mut self) -> PResult<Expr> {
        let mut l = self.parse_comparison()?;
        loop {
            let op = match self.peek() {
                Tok::EqEq | Tok::EqEqEq => BinOp::Eq,
                Tok::BangEq | Tok::BangEqEq => BinOp::Neq,
                _ => break,
            };
            self.bump();
            let r = self.parse_comparison()?;
            let (line, col) = (self.line(), self.col());
            l = Expr::Binary(op, Box::new(l), Box::new(r), line, col);
        }
        Ok(l)
    }

    fn parse_comparison(&mut self) -> PResult<Expr> {
        let mut l = self.parse_additive()?;
        loop {
            let op = match self.peek() {
                Tok::Lt => BinOp::Lt,
                Tok::Gt => BinOp::Gt,
                Tok::Le => BinOp::Le,
                Tok::Ge => BinOp::Ge,
                _ => break,
            };
            self.bump();
            let r = self.parse_additive()?;
            let (line, col) = (self.line(), self.col());
            l = Expr::Binary(op, Box::new(l), Box::new(r), line, col);
        }
        Ok(l)
    }

    fn parse_additive(&mut self) -> PResult<Expr> {
        let mut l = self.parse_multiplicative()?;
        loop {
            let op = match self.peek() {
                Tok::Plus => BinOp::Add,
                Tok::Minus => BinOp::Sub,
                _ => break,
            };
            self.bump();
            let r = self.parse_multiplicative()?;
            let (line, col) = (self.line(), self.col());
            l = Expr::Binary(op, Box::new(l), Box::new(r), line, col);
        }
        Ok(l)
    }

    fn parse_multiplicative(&mut self) -> PResult<Expr> {
        let mut l = self.parse_unary()?;
        loop {
            let op = match self.peek() {
                Tok::Star => BinOp::Mul,
                Tok::Slash => BinOp::Div,
                Tok::Percent => BinOp::Mod,
                _ => break,
            };
            self.bump();
            let r = self.parse_unary()?;
            let (line, col) = (self.line(), self.col());
            l = Expr::Binary(op, Box::new(l), Box::new(r), line, col);
        }
        Ok(l)
    }

    fn parse_unary(&mut self) -> PResult<Expr> {
        match self.peek() {
            Tok::Minus => {
                self.bump();
                let e = self.parse_unary()?;
                Ok(Expr::Unary(UnOp::Neg, Box::new(e)))
            }
            Tok::Bang => {
                self.bump();
                let e = self.parse_unary()?;
                Ok(Expr::Unary(UnOp::Not, Box::new(e)))
            }
            _ => self.parse_postfix(),
        }
    }

    fn parse_postfix(&mut self) -> PResult<Expr> {
        let mut e = self.parse_primary()?;
        let mut dataset_base: Option<Box<Expr>> = None; // x.dataset.<key> 的中间态
        loop {
            match self.peek() {
                Tok::Dot => {
                    self.bump();
                    let (line, col) = (self.line(), self.col());
                    match self.bump().tok {
                        Tok::Ident(m) if m == "getHours" || m == "getMinutes" || m == "getSeconds" || m == "getTime" => {
                            // Date 实例方法：Date() 产生 epoch 毫秒（number），方法折叠为
                            // 内置调用 Date.<m>(ms)。任何 number 表达式都可用（Date 即数值）。
                            self.expect(&Tok::LParen, "'('")?;
                            self.expect(&Tok::RParen, "')'")?;
                            e = Expr::Call { callee: format!("Date.{}", m), args: vec![e], line, col };
                        }
                        Tok::Ident(m) if m == "length" => {
                            e = Expr::Length(Box::new(e), line, col);
                        }
                        Tok::Ident(m) if m == "getElementById" => {
                            // document.getElementById("id")
                            let base = match &e {
                                Expr::Ident(b, _, _) => b.clone(),
                                _ => return Err(self.err_here("getElementById 只能通过 document 调用")),
                            };
                            if base != "document" {
                                return Err(self.err_here(format!("未知的属性或方法 '{}.{}'", base, m)));
                            }
                            self.expect(&Tok::LParen, "'('")?;
                            let id = match self.bump().tok {
                                Tok::StrLit(s) => s,
                                _ => return Err(self.err_here("M1: getElementById 的参数必须是字符串字面量")),
                            };
                            self.expect(&Tok::RParen, "')'")?;
                            e = Expr::GetById { id, line, col };
                        }
                        Tok::Ident(m) if m == "value" => {
                            // <textarea>.value 读取（对象必须是 Elem 变量，check 阶段验证）
                            let base = match &e {
                                Expr::Ident(b, _, _) => b.clone(),
                                _ => return Err(self.err_here(".value 只能用于 Element 变量")),
                            };
                            e = Expr::ElemValue { obj: base, line, col };
                        }
                        Tok::Ident(m) if m == "touches" || m == "changedTouches" => {
                            // 触摸事件成员：e.touches[0].clientX / e.changedTouches[0].clientX
                            // （子集仅支持主触点 [0]，折叠为 clientX/clientY；check 按事件种类验证）
                            let base = match &e {
                                Expr::Ident(b, _, _) => b.clone(),
                                _ => return Err(self.err_here(format!(".{} 只能用于 Event 变量", m))),
                            };
                            self.expect(&Tok::LBracket, "'['")?;
                            let _idx = self.parse_expr()?; // 仅支持主触点，索引值忽略
                            self.expect(&Tok::RBracket, "']'")?;
                            self.expect(&Tok::Dot, "'.'")?;
                            let pt = match self.bump().tok {
                                Tok::Ident(p) if p == "clientX" || p == "clientY" => p,
                                _ => return Err(self.err_here("触点仅支持 .clientX / .clientY")),
                            };
                            e = Expr::PropRead { obj: base, prop: pt, line, col };
                        }
                        Tok::Ident(m) if m == "closest" => {
                            // e.target.closest("a, button") -> boolean（self+祖先链选择器匹配）
                            self.expect(&Tok::LParen, "'('")?;
                            let sel = match self.bump().tok {
                                Tok::StrLit(s) => s,
                                _ => return Err(self.err_here("closest 的参数必须是字符串字面量")),
                            };
                            self.expect(&Tok::RParen, "')'")?;
                            e = Expr::Closest { base: Box::new(e), sel, line, col };
                        }
                        Tok::Ident(m) if m == "forEach" => {
                            // list.forEach(fn)：fn 为函数名或内联箭头（已提升为隐藏函数）
                            let list = match &e {
                                Expr::Ident(b, _, _) => b.clone(),
                                _ => return Err(self.err_here(".forEach 只能用于数组变量（querySelectorAll 结果）")),
                            };
                            self.expect(&Tok::LParen, "'('")?;
                            let fncall = match self.bump().tok {
                                Tok::Ident(f) => {
                                    /* 无括号单参箭头：s => ... */
                                    if self.peek() == &Tok::Arrow {
                                        self.bump(); // 吃掉 =>
                                        match self.finish_arrow(vec![(f, None)], line, col)? {
                                            Expr::Ident(n, _, _) => n,
                                            _ => unreachable!("finish_arrow 必返回 Ident"),
                                        }
                                    } else {
                                        f
                                    }
                                }
                                Tok::LParen => {
                                    /* 内联多参箭头：(a, b) => ... —— bump 已吃掉箭头自己的 '('，
                                     * 状态与 parse_primary 的 LParen 臂入口一致，复用同一解析路径 */
                                    if !self.paren_arrow_ahead() {
                                        return Err(self.err_here("forEach 的参数必须是函数名或箭头函数"));
                                    }
                                    let mut params = Vec::new();
                                    if self.peek() != &Tok::RParen {
                                        loop {
                                            let pname = match self.bump().tok {
                                                Tok::Ident(s) => s,
                                                _ => return Err(self.err_here("箭头函数参数名必须是标识符")),
                                            };
                                            let pty = if self.eat(&Tok::Colon) { Some(self.parse_type()?) } else { None };
                                            params.push((pname, pty));
                                            if !self.eat(&Tok::Comma) { break; }
                                        }
                                    }
                                    self.expect(&Tok::RParen, "')'")?;
                                    self.expect(&Tok::Arrow, "'=>'")?;
                                    match self.finish_arrow(params, line, col)? {
                                        Expr::Ident(n, _, _) => n,
                                        _ => unreachable!("finish_arrow 必返回 Ident"),
                                    }
                                }
                                _ => return Err(self.err_here("forEach 的参数必须是函数名或箭头函数")),
                            };
                            self.expect(&Tok::RParen, "')'")?;
                            e = Expr::ForEach { list, fncall, line, col };
                        }
                        Tok::Ident(m) if m == "dataset" => {
                            match &e {
                                Expr::Ident(..) | Expr::Index { .. } => {
                                    dataset_base = Some(Box::new(e.clone()));
                                    continue; /* 下一个 Dot 处理 key */
                                }
                                _ => return Err(self.err_here(".dataset 只能用于 Element")),
                            }
                        }
                        Tok::Ident(m) if m == "clientX" || m == "clientY" || m == "target" || m == "key" => {
                            // Event 参数成员读：e.clientX / e.clientY / e.target / e.key
                            // （对象必须是 Event 变量，check 阶段验证种类与上下文）
                            let base = match &e {
                                Expr::Ident(b, _, _) => b.clone(),
                                _ => return Err(self.err_here(format!(".{} 只能用于 Event 变量", m))),
                            };
                            e = Expr::PropRead { obj: base, prop: m, line, col };
                        }
                        Tok::Ident(m) if m == "preventDefault" => {
                            // e.preventDefault()：事件对象方法，零参
                            let base = match &e {
                                Expr::Ident(b, _, _) => b.clone(),
                                _ => return Err(self.err_here(".preventDefault 只能用于 Event 变量")),
                            };
                            self.expect(&Tok::LParen, "'('")?;
                            self.expect(&Tok::RParen, "')'")?;
                            e = Expr::Call { callee: format!("{}.preventDefault", base), args: vec![], line, col };
                        }
                        Tok::Ident(m) => {
                            /* x.dataset.key —— 两段链的收尾（base 为 Ident/Index 均可） */
                            if let Some(b) = dataset_base.take() {
                                e = Expr::DatasetRead { base: b, key: m, line, col };
                                continue;
                            }
                            // el.classList.add/remove/toggle/contains —— classList 属性链
                            if m == "classList" {
                                match &e {
                                    Expr::Ident(..) => { continue; /* 下一个 Dot 处理 add 等 */ }
                                    _ => return Err(self.err_here(".classList 只能用于 Element 变量")),
                                }
                            }
                            // el.previousElementSibling —— 无括号属性读 → 0 参内置
                            if m == "previousElementSibling" {
                                let pb = match &e {
                                    Expr::Ident(b, _, _) => b.clone(),
                                    _ => return Err(self.err_here("previousElementSibling 只能用于 Element 变量")),
                                };
                                e = Expr::Call { callee: "elem.prevSibling".into(),
                                    args: vec![Expr::Ident(pb, line, col)], line, col };
                                continue;
                            }
                            // window.innerWidth / window.innerHeight：无括号属性读
                            // location.hash：无括号属性读（0 参内置 Call）
                            let base = match &e {
                                Expr::Ident(b, _, _) => b.clone(),
                                _ => return Err(self.err_here("M0 仅支持 .length 与内置对象调用")),
                            };
                            if base == "window" && (m == "innerWidth" || m == "innerHeight") {
                                e = Expr::Call { callee: format!("window.{}", m), args: vec![], line, col };
                                continue;
                            }
                            if base == "location" && m == "hash" {
                                e = Expr::Call { callee: "location.hash".to_string(), args: vec![], line, col };
                                continue;
                            }
                            // document.documentElement / document.body：伪对象名（后续 Dot 再接方法）
                            if base == "document" && (m == "documentElement" || m == "body") {
                                e = Expr::Ident(format!("document.{}", m), line, col);
                                continue;
                            }
                            // 仅支持内置对象的点号调用
                            let full = format!("{}.{}", base, m);
                            let known_obj = matches!(base.as_str(),
                                "console" | "Math" | "fs" | "clipboard" | "document"
                                | "Array" | "window" | "localStorage");
                            let elem_listener = m == "addEventListener" && !known_obj;
                            let classmethod = ["add", "remove", "toggle", "contains"].contains(&m.as_str());
                            let builtin = (base == "console" && m == "log")
                                || (base == "Math" && ["floor", "abs", "sqrt", "max", "min"].contains(&m.as_str()))
                                || (base == "fs" && ["readText", "writeText"].contains(&m.as_str()))
                                || (base == "clipboard" && ["setText", "getText"].contains(&m.as_str()))
                                || (base == "document" && ["queryCount", "showOnly", "setTheme", "addEventListener", "querySelectorAll"].contains(&m.as_str()))
                                || (base == "document.documentElement" && ["setAttribute", "getAttribute"].contains(&m.as_str()))
                                || (base == "Array" && m == "from")
                                || (base == "window" && ["print", "matchMedia"].contains(&m.as_str()))
                                || (base == "localStorage" && ["getItem", "setItem"].contains(&m.as_str()))
                                || classmethod
                                || elem_listener;
                            if !builtin {
                                return Err(self.err_here(format!("未知的属性或方法 '{}'", full)));
                            }
                            /* window.matchMedia 裸引用（`matchMedia && ...` 存在性判断）：
                             * 内置在 Nowebiew 中必存在，折叠为 true */
                            if base == "window" && m == "matchMedia" && self.peek() != &Tok::LParen {
                                e = Expr::Bool(true);
                                continue;
                            }
                            self.expect(&Tok::LParen, "'('")?;
                            let mut args = Vec::new();
                            if classmethod {
                                /* 首参 = 元素本身（base 即元素变量名） */
                                let (l0, c0) = (self.line(), self.col());
                                args.push(Expr::Ident(base.clone(), l0, c0));
                            }
                            /* parse_call_args 统一处理对象字面量选项袋（{passive:true} 等） */
                            args.extend(self.parse_call_args()?);
                            let callee = if classmethod { format!("classList.{}", m) } else { full };
                            // window.matchMedia(q).matches -> 布尔：折叠为一个内置 Call
                            if callee == "window.matchMedia" {
                                if !(self.eat(&Tok::Dot) && matches!(self.bump().tok, Tok::Ident(ref k) if k == "matches")) {
                                    return Err(self.err_here("window.matchMedia 后必须紧跟 .matches"));
                                }
                                return Ok(Expr::Call { callee: "window.matchMedia.matches".into(), args, line, col });
                            }
                            e = Expr::Call { callee, args, line, col };
                        }
                        _ => return Err(self.err_here("M0 仅支持属性 .length")),
                    }
                }
                Tok::LBracket => {
                    // listVar[i]：下标读（ElemList -> Element / StrList -> string）
                    let (eline, ecol) = (self.line(), self.col());
                    let obj = match &e {
                        Expr::Ident(b, _, _) => b.clone(),
                        _ => return Err(self.err_here("仅支持对数组变量下标访问")),
                    };
                    self.bump(); // [
                    let idx = self.parse_expr()?;
                    self.expect(&Tok::RBracket, "']'")?;
                    e = Expr::Index { obj, index: Box::new(idx), line: eline, col: ecol };
                }
                Tok::PlusPlus => {
                    let (line, col) = (self.line(), self.col());
                    self.bump();
                    e = Expr::PostOp(PostOp::Inc, Box::new(e), line, col);
                }
                Tok::MinusMinus => {
                    let (line, col) = (self.line(), self.col());
                    self.bump();
                    e = Expr::PostOp(PostOp::Dec, Box::new(e), line, col);
                }
                _ => break,
            }
        }
        Ok(e)
    }

    fn parse_primary(&mut self) -> PResult<Expr> {
        let t = self.bump();
        match t.tok {
            Tok::NumLit(v) => Ok(Expr::Num(v)),
            Tok::StrLit(s) => Ok(Expr::Str(s)),
            Tok::True => Ok(Expr::Bool(true)),
            Tok::False => Ok(Expr::Bool(false)),
            Tok::Tpl(parts) => self.build_tpl(parts, t.line, t.col),
            Tok::Ident(name) => {
                if name == "null" {
                    return Ok(Expr::Null(t.line, t.col));
                }
                // 单参箭头：e => ... / e => expr（参数无类型注解）
                if self.peek() == &Tok::Arrow {
                    self.bump();
                    return self.finish_arrow(vec![(name, None)], t.line, t.col);
                }
                if self.peek() == &Tok::LParen {
                    self.bump(); // (
                    let args = self.parse_call_args()?;
                    Ok(Expr::Call { callee: name, args, line: t.line, col: t.col })
                } else {
                    Ok(Expr::Ident(name, t.line, t.col))
                }
            }
            Tok::RegexLit(pat) => {
                // /re/.exec(arg) -> StrList|null   /re/.test(arg) -> boolean（主位置正则）
                if self.eat(&Tok::Dot) {
                    let m = match self.bump().tok {
                        Tok::Ident(m) => m,
                        _ => return Err(self.err_here("正则字面量后仅支持 .exec() / .test()")),
                    };
                    self.expect(&Tok::LParen, "'('")?;
                    let arg = self.parse_expr()?;
                    self.expect(&Tok::RParen, "')'")?;
                    return match m.as_str() {
                        "exec" => Ok(Expr::RegexExec { pat, arg: Box::new(arg), line: t.line, col: t.col }),
                        "test" => Ok(Expr::RegexTest { pat, arg: Box::new(arg), line: t.line, col: t.col }),
                        _ => Err(self.err_here(format!("正则字面量后仅支持 .exec() / .test()，不支持 '.{}'", m))),
                    };
                }
                Err(self.err_here("正则字面量必须紧跟 .exec(...) 或 .test(...)"))
            }
            Tok::Function => {
                /* 函数表达式作为值（回调实参等）：提升为顶层函数，返回其名。
                 * 关键字已在 parse_primary 入口消费，直接进函数体解析。 */
                let id = self.lambdas;
                let name = if let Tok::Ident(s) = self.peek().clone() {
                    self.bump();
                    s
                } else {
                    self.lambdas += 1;
                    format!("nw_lambda_{}", id)
                };
                let fd = self.parse_fn_rest(name, t.line, t.col)?;
                self.pending.push(fd.clone());
                Ok(Expr::Ident(fd.name, t.line, t.col))
            }
            Tok::New => {
                /* new Date()：折叠为内置调用 Date()（epoch 毫秒，number） */
                let name = match self.bump().tok {
                    Tok::Ident(s) => s,
                    _ => return Err(self.err_here("new 后仅支持 Date()")),
                };
                if name != "Date" {
                    return Err(self.err_here(format!("仅支持 new Date()，不支持 new {}", name)));
                }
                self.expect(&Tok::LParen, "'('")?;
                self.expect(&Tok::RParen, "')'")?;
                Ok(Expr::Call { callee: "Date".to_string(), args: Vec::new(), line: t.line, col: t.col })
            }
            Tok::LParen => {
                // IIFE：(function {...})(args) —— 函数表达式仅支持立即调用
                if self.peek() == &Tok::Function {
                    let fd = self.parse_fn_expr()?;
                    self.pending.push(fd.clone());
                    self.expect(&Tok::RParen, "')'")?;
                    self.expect(&Tok::LParen, "'('（function 表达式仅支持立即调用 IIFE）")?;
                    let args = self.parse_call_args()?;
                    return Ok(Expr::Call { callee: fd.name, args, line: t.line, col: t.col });
                }
                // 多参箭头：(a, b) => ...（LParen 已在 parse_primary 入口被消费）
                if self.paren_arrow_ahead() {
                    let mut params = Vec::new();
                    if self.peek() != &Tok::RParen {
                        loop {
                            let pname = match self.bump().tok {
                                Tok::Ident(s) => s,
                                _ => return Err(self.err_here("箭头函数参数名必须是标识符")),
                            };
                            let pty = if self.eat(&Tok::Colon) { Some(self.parse_type()?) } else { None };
                            params.push((pname, pty));
                            if !self.eat(&Tok::Comma) { break; }
                        }
                    }
                    self.expect(&Tok::RParen, "')'")?;
                    self.expect(&Tok::Arrow, "'=>'")?;
                    return self.finish_arrow(params, t.line, t.col);
                }
                let e = self.parse_expr()?;
                self.expect(&Tok::RParen, "')'")?;
                Ok(e)
            }
            _ => Err(ParseError {
                msg: format!("意外的 token（应为表达式）{}", self.describe_prev(&t)),
                line: t.line, col: t.col,
            }),
        }
    }

    /// 箭头函数收尾：'=>' 已消费，解析体（块或表达式），登记为隐藏函数，返回其 Ident
    fn finish_arrow(&mut self, params: Vec<(String, Option<Ty>)>, line: u32, col: u32) -> PResult<Expr> {
        let id = self.lambdas;
        self.lambdas += 1;
        let name = format!("nw_lambda_{}", id);
        let body: Vec<Stmt> = if self.peek() == &Tok::LBrace {
            self.parse_block()?
        } else {
            let e = self.parse_expr()?;
            vec![Stmt::Return(Some(e), line, col)]
        };
        self.pending.push(FnDecl { name: name.clone(), params, ret: None, body, line, col });
        Ok(Expr::Ident(name, line, col))
    }

    /// 多参箭头前瞻：调用点已消费 LParen，从其后扫描到匹配 RParen，看下一个是否 =>
    fn paren_arrow_ahead(&self) -> bool {
        let mut depth = 1usize; // 当前这个 ( 已被吃掉
        let mut i = self.pos;
        while i < self.toks.len() {
            match self.toks[i].tok {
                Tok::LParen => depth += 1,
                Tok::RParen => {
                    depth -= 1;
                    if depth == 0 {
                        return i + 1 < self.toks.len() && self.toks[i + 1].tok == Tok::Arrow;
                    }
                }
                Tok::Eof => return false,
                _ => {}
            }
            i += 1;
        }
        false
    }

    /// 调用实参：'(' 已消费，吃 args 直到 ')'
    fn parse_call_args(&mut self) -> PResult<Vec<Expr>> {
        let mut args = Vec::new();
        if self.peek() != &Tok::RParen {
            loop {
                /* 对象字面量实参（如 {passive:true} 选项袋）：跳过并折叠为 Null（忽略） */
                if self.peek() == &Tok::LBrace {
                    let (l, c) = (self.line(), self.col());
                    let mut depth = 0usize;
                    loop {
                        match self.bump().tok {
                            Tok::LBrace => depth += 1,
                            Tok::RBrace => { depth -= 1; if depth == 0 { break; } }
                            Tok::Eof => return Err(self.err_here("对象字面量未闭合")),
                            _ => {}
                        }
                    }
                    args.push(Expr::Null(l, c));
                } else {
                    args.push(self.parse_expr()?);
                }
                if !self.eat(&Tok::Comma) { break; }
            }
        }
        self.expect(&Tok::RParen, "')'")?;
        Ok(args)
    }

    fn build_tpl(&mut self, parts: Vec<TplPart>, line: u32, col: u32) -> PResult<Expr> {        let mut nodes = Vec::new();
        for p in parts {
            match p {
                TplPart::Lit(s) => nodes.push(TplNode::Lit(s)),
                TplPart::ExprSrc(src, el, ec) => {
                    // 子解析器解析插值表达式
                    let mut sub = Parser {
                        toks: lex(&src).map_err(|e| ParseError { msg: format!("插值表达式错误: {}", e.msg), line: el, col: ec })?,
                        pos: 0,
                        lambdas: 0,
                        pending: Vec::new(),
                    };
                    let e = sub.parse_expr()?;
                    if sub.peek() != &Tok::Eof {
                        return Err(ParseError { msg: "插值表达式有多余内容".into(), line: el, col: ec });
                    }
                    nodes.push(TplNode::Expr(e));
                }
            }
        }
        let _ = (line, col);
        Ok(Expr::Tpl(nodes))
    }
}
