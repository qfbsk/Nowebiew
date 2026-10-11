// Nowebiew M0 - 词法分析器
// 将 TS 子集源码切分为 token 流。支持模板字符串（记录 ${} 内表达式原文，
// 交给 parser 子解析）。

#[derive(Debug, Clone, PartialEq)]
pub enum TplPart {
    Lit(String),
    ExprSrc(String, u32, u32), // 表达式原文, 行, 列
}

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    Ident(String),
    NumLit(f64),
    StrLit(String),
    Tpl(Vec<TplPart>),
    // 关键字
    Function, Let, Const, If, Else, While, For, Return, Break, Continue, Try, New,
    True, False, TNumber, TString, TBoolean, TVoid,
    // 符号
    Plus, Minus, Star, Slash, Percent, Bang,
    AmpAmp, PipePipe,
    Eq, EqEq, EqEqEq, BangEq, BangEqEq, Arrow,
    Lt, Le, Gt, Ge,
    PlusEq, MinusEq, StarEq, SlashEq,
    PlusPlus, MinusMinus,
    Question, Colon,
    LParen, RParen, LBrace, RBrace, LBracket, RBracket, Comma, Semi, Dot,
    RegexLit(String),   /* /pattern/flags —— 正则字面量（模式含原始反斜杠序列） */
    Eof,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub tok: Tok,
    pub line: u32,
    pub col: u32,
}

pub struct LexError {
    pub msg: String,
    pub line: u32,
    pub col: u32,
}

struct Lexer<'a> {
    src: &'a [u8],
    pos: usize,
    line: u32,
    col: u32,
    regex_allowed: bool, /* JS 词法规则：主位置（前一个 token 不是值结尾）的 / 是正则 */
}

pub fn lex(src: &str) -> Result<Vec<Token>, LexError> {
    let mut lx = Lexer { src: src.as_bytes(), pos: 0, line: 1, col: 1, regex_allowed: true };
    let mut out = Vec::new();
    loop {
        let t = lx.next_token()?;
        /* 值结尾的位置（标识符/字面量/右括号/后缀运算）后，/ 只能是除号；
         * 其余位置（运算符后、括号后、语句开始）的 / 是正则字面量开头 */
        lx.regex_allowed = !matches!(&t.tok,
            Tok::Ident(_) | Tok::NumLit(_) | Tok::StrLit(_) | Tok::Tpl(_)
            | Tok::RParen | Tok::RBracket | Tok::True | Tok::False
            | Tok::PlusPlus | Tok::MinusMinus);
        let done = t.tok == Tok::Eof;
        out.push(t);
        if done { break; }
    }
    Ok(out)
}

impl<'a> Lexer<'a> {
    fn peek(&self) -> u8 {
        if self.pos < self.src.len() { self.src[self.pos] } else { 0 }
    }
    fn peek2(&self) -> u8 {
        if self.pos + 1 < self.src.len() { self.src[self.pos + 1] } else { 0 }
    }
    fn bump(&mut self) -> u8 {
        let c = self.peek();
        self.pos += 1;
        if c == b'\n' { self.line += 1; self.col = 1; } else { self.col += 1; }
        c
    }
    fn err(&self, msg: impl Into<String>) -> LexError {
        LexError { msg: msg.into(), line: self.line, col: self.col }
    }

    fn skip_ws_comments(&mut self) -> Result<(), LexError> {
        loop {
            let c = self.peek();
            if c == b' ' || c == b'\t' || c == b'\r' || c == b'\n' {
                self.bump();
            } else if c == b'/' && self.peek2() == b'/' {
                while self.peek() != 0 && self.peek() != b'\n' { self.bump(); }
            } else if c == b'/' && self.peek2() == b'*' {
                let (l, c0) = (self.line, self.col);
                self.bump(); self.bump();
                loop {
                    if self.peek() == 0 {
                        return Err(LexError { msg: "未闭合的块注释".into(), line: l, col: c0 });
                    }
                    if self.peek() == b'*' && self.peek2() == b'/' {
                        self.bump(); self.bump();
                        break;
                    }
                    self.bump();
                }
            } else {
                break;
            }
        }
        Ok(())
    }

    fn next_token(&mut self) -> Result<Token, LexError> {
        self.skip_ws_comments()?;
        let (line, col) = (self.line, self.col);
        let c = self.peek();
        if c == 0 {
            return Ok(Token { tok: Tok::Eof, line, col });
        }
        // 标识符 / 关键字
        if c.is_ascii_alphabetic() || c == b'_' || c >= 0x80 {
            let start = self.pos;
            while {
                let p = self.peek();
                p.is_ascii_alphanumeric() || p == b'_' || p >= 0x80
            } { self.bump(); }
            let word = std::str::from_utf8(&self.src[start..self.pos])
                .map_err(|_| self.err("非法标识符字符"))?.to_string();
            let tok = match word.as_str() {
                "function" => Tok::Function,
                "let" => Tok::Let,
                "var" => Tok::Let, /* var 语义按 let 处理；同作用域同类型重声明在 check 放行（var 允许） */
                "new" => Tok::New,
                "const" => Tok::Const,
                "if" => Tok::If,
                "else" => Tok::Else,
                "while" => Tok::While,
                "for" => Tok::For,
                "return" => Tok::Return,
                "break" => Tok::Break,
                "continue" => Tok::Continue,
                "true" => Tok::True,
                "try" => Tok::Try,
                "false" => Tok::False,
                "number" => Tok::TNumber,
                "string" => Tok::TString,
                "boolean" => Tok::TBoolean,
                "void" => Tok::TVoid,
                _ => Tok::Ident(word),
            };
            return Ok(Token { tok, line, col });
        }
        // 数字
        if c.is_ascii_digit() {
            let start = self.pos;
            while self.peek().is_ascii_digit() { self.bump(); }
            if self.peek() == b'.' && self.peek2().is_ascii_digit() {
                self.bump();
                while self.peek().is_ascii_digit() { self.bump(); }
            }
            if self.peek() == b'e' || self.peek() == b'E' {
                self.bump();
                if self.peek() == b'+' || self.peek() == b'-' { self.bump(); }
                if !self.peek().is_ascii_digit() { return Err(self.err("非法指数")); }
                while self.peek().is_ascii_digit() { self.bump(); }
            }
            let text = std::str::from_utf8(&self.src[start..self.pos]).unwrap();
            let v: f64 = text.parse().map_err(|_| self.err("非法数字字面量"))?;
            return Ok(Token { tok: Tok::NumLit(v), line, col });
        }
        // 字符串
        if c == b'"' || c == b'\'' {
            let quote = self.bump();
            let mut s = String::new();
            loop {
                let ch = self.peek();
                if ch == 0 || ch == b'\n' { return Err(self.err("字符串未闭合")); }
                if ch == quote { self.bump(); break; }
                if ch == b'\\' {
                    self.bump();
                    let e = self.bump();
                    match e {
                        b'n' => s.push('\n'),
                        b't' => s.push('\t'),
                        b'r' => s.push('\r'),
                        b'\\' => s.push('\\'),
                        b'0' => s.push('\0'),
                        b'\'' => s.push('\''),
                        b'"' => s.push('"'),
                        b'`' => s.push('`'),
                        _ => return Err(self.err(format!("不支持的转义字符 \\{}", e as char))),
                    }
                } else {
                    // UTF-8 多字节整体拷贝
                    let start = self.pos;
                    self.bump();
                    while self.peek() >= 0x80 { self.bump(); }
                    let piece = std::str::from_utf8(&self.src[start..self.pos])
                        .map_err(|_| self.err("非法 UTF-8"))?;
                    s.push_str(piece);
                }
            }
            return Ok(Token { tok: Tok::StrLit(s), line, col });
        }
        // 模板字符串
        if c == b'`' {
            self.bump();
            let parts = self.lex_template()?;
            return Ok(Token { tok: Tok::Tpl(parts), line, col });
        }
        // 符号
        let two = |a: u8, b: u8| c == a && self.peek2() == b;
        let tok = if two(b'=', b'=') && self.src.get(self.pos + 2) == Some(&b'=') {
            self.bump(); self.bump(); self.bump(); Tok::EqEqEq
        } else if two(b'!', b'=') && self.src.get(self.pos + 2) == Some(&b'=') {
            self.bump(); self.bump(); self.bump(); Tok::BangEqEq
        } else if two(b'=', b'=') { self.bump(); self.bump(); Tok::EqEq }
        else if two(b'!', b'=') { self.bump(); self.bump(); Tok::BangEq }
        else if two(b'&', b'&') { self.bump(); self.bump(); Tok::AmpAmp }
        else if two(b'|', b'|') { self.bump(); self.bump(); Tok::PipePipe }
        else if two(b'<', b'=') { self.bump(); self.bump(); Tok::Le }
        else if two(b'>', b'=') { self.bump(); self.bump(); Tok::Ge }
        else if two(b'+', b'=') { self.bump(); self.bump(); Tok::PlusEq }
        else if two(b'-', b'=') { self.bump(); self.bump(); Tok::MinusEq }
        else if two(b'*', b'=') { self.bump(); self.bump(); Tok::StarEq }
        else if two(b'/', b'=') { self.bump(); self.bump(); Tok::SlashEq }
        else if two(b'=', b'>') { self.bump(); self.bump(); Tok::Arrow }
        else if two(b'+', b'+') { self.bump(); self.bump(); Tok::PlusPlus }
        else if two(b'-', b'-') { self.bump(); self.bump(); Tok::MinusMinus }
        else if c == b'/' && self.regex_allowed {
            /* 正则字面量：/pattern/flags —— 字符类 [..] 内的 / 不结尾 */
            self.bump(); // 开头 /
            let mut pat = String::new();
            let mut in_class = false;
            loop {
                let ch = self.peek();
                if ch == 0 || ch == b'\n' { return Err(self.err("正则字面量未闭合")); }
                if ch == b'\\' {
                    self.bump();
                    let e = self.bump();
                    if e == 0 { return Err(self.err("正则字面量未闭合")); }
                    pat.push('\\'); pat.push(e as char);
                    continue;
                }
                if ch == b'/' && !in_class { self.bump(); break; }
                if ch == b'[' { in_class = true; }
                if ch == b']' { in_class = false; }
                let st = self.pos;
                self.bump();
                if ch >= 0x80 {
                    while self.peek() >= 0x80 { self.bump(); }
                    match std::str::from_utf8(&self.src[st..self.pos]) {
                        Ok(s) => pat.push_str(s),
                        Err(_) => return Err(self.err("非法 UTF-8")),
                    }
                } else {
                    pat.push(ch as char);
                }
            }
            while self.peek().is_ascii_alphabetic() { self.bump(); } /* flags: g/i/m 忽略 */
            Tok::RegexLit(pat)
        }
        else {
            self.bump();
            match c {
                b'+' => Tok::Plus,
                b'-' => Tok::Minus,
                b'*' => Tok::Star,
                b'/' => Tok::Slash,
                b'%' => Tok::Percent,
                b'!' => Tok::Bang,
                b'=' => Tok::Eq,
                b'<' => Tok::Lt,
                b'>' => Tok::Gt,
                b'?' => Tok::Question,
                b':' => Tok::Colon,
                b'(' => Tok::LParen,
                b')' => Tok::RParen,
                b'[' => Tok::LBracket,
                b']' => Tok::RBracket,
                b'{' => Tok::LBrace,
                b'}' => Tok::RBrace,
                b',' => Tok::Comma,
                b';' => Tok::Semi,
                b'.' => Tok::Dot,
                _ => return Err(self.err(format!("无法识别的字符 '{}'", c as char))),
            }
        };
        Ok(Token { tok, line, col })
    }

    // 已消费开头 ` ；读到结尾 ` 为止
    fn lex_template(&mut self) -> Result<Vec<TplPart>, LexError> {
        let mut parts: Vec<TplPart> = Vec::new();
        let mut lit = String::new();
        loop {
            let ch = self.peek();
            if ch == 0 { return Err(self.err("模板字符串未闭合")); }
            if ch == b'`' { self.bump(); break; }
            if ch == b'\\' {
                self.bump();
                let e = self.bump();
                match e {
                    b'n' => lit.push('\n'),
                    b't' => lit.push('\t'),
                    b'`' => lit.push('`'),
                    b'\\' => lit.push('\\'),
                    b'$' => lit.push('$'),
                    _ => return Err(self.err(format!("不支持的转义 \\{}", e as char))),
                }
                continue;
            }
            if ch == b'$' && self.peek2() == b'{' {
                let (el, ec) = (self.line, self.col);
                self.bump(); self.bump(); // ${
                let start = self.pos;
                let mut depth = 1usize;
                while depth > 0 {
                    let p = self.peek();
                    if p == 0 { return Err(self.err("模板插值未闭合")); }
                    if p == b'{' { depth += 1; }
                    if p == b'}' { depth -= 1; if depth == 0 { break; } }
                    // 字符串/模板嵌套原样跳过
                    if p == b'"' || p == b'\'' || p == b'`' {
                        let q = self.bump();
                        while self.peek() != 0 && self.peek() != q {
                            if self.peek() == b'\\' { self.bump(); }
                            self.bump();
                        }
                        if self.peek() == 0 { return Err(self.err("插值内字符串未闭合")); }
                    }
                    self.bump();
                }
                let expr_src = std::str::from_utf8(&self.src[start..self.pos])
                    .map_err(|_| self.err("非法 UTF-8"))?.trim().to_string();
                self.bump(); // }
                if !lit.is_empty() {
                    parts.push(TplPart::Lit(std::mem::take(&mut lit)));
                }
                parts.push(TplPart::ExprSrc(expr_src, el, ec));
                continue;
            }
            // 普通 UTF-8 字符
            let start = self.pos;
            self.bump();
            while self.peek() >= 0x80 { self.bump(); }
            let piece = std::str::from_utf8(&self.src[start..self.pos])
                .map_err(|_| self.err("非法 UTF-8"))?;
            lit.push_str(piece);
        }
        if !lit.is_empty() {
            parts.push(TplPart::Lit(lit));
        }
        Ok(parts)
    }
}
