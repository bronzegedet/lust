use crate::token::{SpannedToken, Token};
use std::iter::Peekable;
use std::str::Chars;

pub struct Lexer<'a> {
    chars: Peekable<Chars<'a>>,
    line: usize,
}

impl<'a> Lexer<'a> {
    pub fn new(input: &'a str) -> Self {
        Self {
            chars: input.chars().peekable(),
            line: 1,
        }
    }

    pub fn next_token(&mut self) -> Option<SpannedToken> {
        self.skip_whitespace();
        let line = self.line;
        let c = self.chars.next()?;

        let kind = match c {
            '(' => Token::LParen,
            ')' => Token::RParen,
            '{' => Token::LBrace,
            '}' => Token::RBrace,
            '[' => Token::LBracket,
            ']' => Token::RBracket,
            ':' => Token::Colon,
            '.' => {
                if let Some(&'.') = self.chars.peek() {
                    self.chars.next();
                    if let Some(&'=') = self.chars.peek() {
                        self.chars.next();
                        Token::DotDotEq
                    } else {
                        Token::DotDot
                    }
                } else {
                    Token::Dot
                }
            }
            ',' => Token::Comma,
            '|' => {
                if let Some(&'>') = self.chars.peek() {
                    self.chars.next();
                    Token::PipeForward
                } else {
                    Token::Pipe
                }
            }
            '+' => {
                if let Some(&'=') = self.chars.peek() {
                    self.chars.next();
                    Token::PlusEq
                } else {
                    Token::Plus
                }
            }
            '*' => {
                if let Some(&'=') = self.chars.peek() {
                    self.chars.next();
                    Token::StarEq
                } else {
                    Token::Star
                }
            }
            '/' => {
                if let Some(&'/') = self.chars.peek() {
                    while let Some(c) = self.chars.next() {
                        if c == '\n' { self.line += 1; break; }
                    }
                    self.next_token()
                        .map(|tok| tok.kind)?
                } else if let Some(&'=') = self.chars.peek() {
                    self.chars.next();
                    Token::SlashEq
                } else {
                    Token::Slash
                }
            }
            '-' => {
                if let Some(&'-') = self.chars.peek() {
                    self.chars.next(); // consume second '-'
                    while let Some(c) = self.chars.next() {
                        if c == '\n' { self.line += 1; break; }
                    }
                    self.next_token()
                        .map(|tok| tok.kind)?
                } else if let Some(&'=') = self.chars.peek() {
                    self.chars.next();
                    Token::MinusEq
                } else {
                    Token::Minus
                }
            }
            '=' => {
                if let Some(&'=') = self.chars.peek() {
                    self.chars.next();
                    Token::Eq
                } else if let Some(&'>') = self.chars.peek() {
                    self.chars.next();
                    Token::FatArrow
                } else {
                    Token::Equals
                }
            }
            '!' => {
                if let Some(&'=') = self.chars.peek() {
                    self.chars.next();
                    Token::Ne
                } else {
                    return None;
                }
            }
            '%' => {
                if let Some(&'=') = self.chars.peek() {
                    self.chars.next();
                    Token::PercentEq
                } else {
                    Token::Percent
                }
            }
            '>' => {
                if let Some(&'=') = self.chars.peek() {
                    self.chars.next();
                    Token::Ge
                } else {
                    Token::Gt
                }
            }
            '<' => {
                if let Some(&'=') = self.chars.peek() {
                    self.chars.next();
                    Token::Le
                } else {
                    Token::Lt
                }
            }
            '"' => {
                let mut s = String::new();
                while let Some(c) = self.chars.next() {
                    if c == '\\' {
                        if let Some(nc) = self.chars.next() {
                            match nc {
                                '"' => s.push('"'),
                                'n' => s.push('\n'),
                                't' => s.push('\t'),
                                '\\' => s.push('\\'),
                                _ => { s.push('\\'); s.push(nc); }
                            }
                        }
                        continue;
                    }
                    if c == '\n' { self.line += 1; }
                    if c == '"' { break; }
                    s.push(c);
                }
                Token::StringLit(s)
            }
            'r' => {
                if matches!(self.chars.peek(), Some('"') | Some('#')) {
                    self.lex_raw_string().unwrap_or_else(|| self.lex_identifier_from_start('r'))
                } else {
                    self.lex_identifier_from_start('r')
                }
            }
            c if c.is_digit(10) => {
                let mut s = String::from(c);
                let mut has_dot = false;
                while let Some(&c) = self.chars.peek() {
                    if c.is_ascii_digit() {
                        s.push(self.chars.next().unwrap());
                    } else if c == '.' && !has_dot {
                        let mut lookahead = self.chars.clone();
                        lookahead.next();
                        if matches!(lookahead.next(), Some(next) if next.is_ascii_digit()) {
                            has_dot = true;
                            s.push(self.chars.next().unwrap());
                        } else {
                            break;
                        }
                    } else {
                        break;
                    }
                }
                Token::Number(s.parse().unwrap_or(0.0))
            }
            c if c.is_alphabetic() || c == '_' => {
                self.lex_identifier_from_start(c)
            }
            _ => return self.next_token(),
        };

        Some(SpannedToken { kind, line })
    }

    fn skip_whitespace(&mut self) {
        while let Some(&c) = self.chars.peek() {
            if c.is_whitespace() {
                if c == '\n' {
                    self.line += 1;
                }
                self.chars.next();
            } else {
                break;
            }
        }
    }

    fn lex_identifier_from_start(&mut self, first: char) -> Token {
        let mut s = String::from(first);
        while let Some(&c) = self.chars.peek() {
            if c.is_alphanumeric() || c == '_' {
                s.push(self.chars.next().unwrap());
            } else {
                break;
            }
        }
        match s.as_str() {
            "fn" => Token::Fn,
            "print" => Token::Print,
            "let" => Token::Let,
            "pass" => Token::Pass,
            "return" => Token::Return,
            "break" => Token::Break,
            "continue" => Token::Continue,
            "if" => Token::If,
            "match" => Token::Match,
            "case" => Token::Case,
            "then" => Token::Then,
            "else" => Token::Else,
            "while" => Token::While,
            "for" => Token::For,
            "in" => Token::In,
            "spawn" => Token::Spawn,
            "do" => Token::Do,
            "type" => Token::Type,
            "enum" => Token::Enum,
            "struct" => Token::Struct,
            "import" => Token::Import,
            "self" => Token::Self_,
            "end" => Token::End,
            "and" => Token::And,
            "or" => Token::Or,
            "not" => Token::Not,
            _ => Token::Ident(s),
        }
    }

    fn lex_raw_string(&mut self) -> Option<Token> {
        let mut hash_count = 0usize;
        while matches!(self.chars.peek(), Some('#')) {
            self.chars.next();
            hash_count += 1;
        }

        if !matches!(self.chars.next(), Some('"')) {
            return None;
        }

        let mut s = String::new();
        loop {
            let c = self.chars.next()?;
            if c == '\n' {
                self.line += 1;
            }

            if c != '"' {
                s.push(c);
                continue;
            }

            let mut lookahead = self.chars.clone();
            let mut matched = true;
            for _ in 0..hash_count {
                if !matches!(lookahead.next(), Some('#')) {
                    matched = false;
                    break;
                }
            }

            if matched {
                for _ in 0..hash_count {
                    self.chars.next();
                }
                break;
            }

            s.push('"');
        }

        Some(Token::RawStringLit(s))
    }
}

#[cfg(test)]
mod tests {
    use super::Lexer;
    use crate::token::Token;

    #[test]
    fn lexes_rust_style_raw_strings() {
        let mut lexer =
            Lexer::new("r\"plain\" r#\"with \\\"quotes\\\"\"# r##\"with # in body\"##");
        let first = lexer.next_token().expect("first token");
        let second = lexer.next_token().expect("second token");
        let third = lexer.next_token().expect("third token");

        assert!(matches!(first.kind, Token::RawStringLit(ref text) if text == "plain"));
        assert!(matches!(second.kind, Token::RawStringLit(ref text) if text == "with \\\"quotes\\\""));
        assert!(matches!(third.kind, Token::RawStringLit(ref text) if text == "with # in body"));
    }
}
