use lorum_ai_contract::ToolDefinition;
use lorum_runtime::ToolCallSummary;
use serde_json::{json, Value};

use crate::ToolOutput;

pub fn definition() -> ToolDefinition {
    ToolDefinition {
        name: "calculator".to_string(),
        description: "Evaluate a mathematical expression. Supports basic arithmetic (+, -, *, /), \
            parentheses, and common math functions (sqrt, pow, abs, sin, cos, tan, log, ln, \
            floor, ceil, round)."
            .to_string(),
        parameters: json!({
            "type": "object",
            "properties": {
                "expression": {
                    "type": "string",
                    "description": "Mathematical expression to evaluate"
                }
            },
            "required": ["expression"],
            "additionalProperties": false
        }),
    }
}

pub fn format_call(args: &Value) -> ToolCallSummary {
    let expr = args
        .get("expression")
        .and_then(|v| v.as_str())
        .unwrap_or("<unknown>");
    ToolCallSummary {
        headline: "calc".to_string(),
        detail: Some(crate::display_preview(expr, 60)),
        body: None,
    }
}

pub fn format_result(is_error: bool, result: &Value) -> String {
    let text = result.as_str().unwrap_or("");
    if is_error {
        crate::display_preview(text, 200)
    } else {
        text.to_string()
    }
}

pub async fn execute(args: Value) -> ToolOutput {
    let expression = match args.get("expression").and_then(Value::as_str) {
        Some(e) => e,
        None => return ToolOutput::err("missing required parameter: expression"),
    };

    match evaluate(expression) {
        Ok(result) => {
            // Format nicely: if it's a whole number, show without decimal
            if result.fract() == 0.0 && result.abs() < 1e15 {
                ToolOutput::ok(format!("{}", result as i64))
            } else {
                ToolOutput::ok(format!("{result}"))
            }
        }
        Err(err) => ToolOutput::err(err),
    }
}

// ---------------------------------------------------------------------------
// Recursive descent parser / evaluator
// ---------------------------------------------------------------------------

fn evaluate(input: &str) -> Result<f64, String> {
    let tokens = tokenize(input)?;
    let mut parser = Parser { tokens, pos: 0 };
    let result = parser.parse_expr()?;
    if parser.pos < parser.tokens.len() {
        return Err(format!(
            "unexpected token: {:?}",
            parser.tokens[parser.pos]
        ));
    }
    Ok(result)
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Number(f64),
    Ident(String),
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    LParen,
    RParen,
    Comma,
}

fn tokenize(input: &str) -> Result<Vec<Token>, String> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let ch = chars[i];

        if ch.is_whitespace() {
            i += 1;
            continue;
        }

        match ch {
            '+' => { tokens.push(Token::Plus); i += 1; }
            '-' => { tokens.push(Token::Minus); i += 1; }
            '*' => { tokens.push(Token::Star); i += 1; }
            '/' => { tokens.push(Token::Slash); i += 1; }
            '%' => { tokens.push(Token::Percent); i += 1; }
            '(' => { tokens.push(Token::LParen); i += 1; }
            ')' => { tokens.push(Token::RParen); i += 1; }
            ',' => { tokens.push(Token::Comma); i += 1; }
            _ if ch.is_ascii_digit() || ch == '.' => {
                let start = i;
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                    i += 1;
                }
                let s: String = chars[start..i].iter().collect();
                let n = s.parse::<f64>().map_err(|_| format!("invalid number: {s}"))?;
                tokens.push(Token::Number(n));
            }
            _ if ch.is_ascii_alphabetic() || ch == '_' => {
                let start = i;
                while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                let name: String = chars[start..i].iter().collect();
                tokens.push(Token::Ident(name));
            }
            _ => return Err(format!("unexpected character: '{ch}'")),
        }
    }

    Ok(tokens)
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn advance(&mut self) -> Option<Token> {
        if self.pos < self.tokens.len() {
            let tok = self.tokens[self.pos].clone();
            self.pos += 1;
            Some(tok)
        } else {
            None
        }
    }

    fn expect(&mut self, expected: &Token) -> Result<(), String> {
        match self.advance() {
            Some(ref tok) if tok == expected => Ok(()),
            Some(tok) => Err(format!("expected {expected:?}, got {tok:?}")),
            None => Err(format!("expected {expected:?}, got end of input")),
        }
    }

    // expr = term (('+' | '-') term)*
    fn parse_expr(&mut self) -> Result<f64, String> {
        let mut left = self.parse_term()?;
        loop {
            match self.peek() {
                Some(Token::Plus) => {
                    self.advance();
                    left += self.parse_term()?;
                }
                Some(Token::Minus) => {
                    self.advance();
                    left -= self.parse_term()?;
                }
                _ => break,
            }
        }
        Ok(left)
    }

    // term = unary (('*' | '/' | '%') unary)*
    fn parse_term(&mut self) -> Result<f64, String> {
        let mut left = self.parse_unary()?;
        loop {
            match self.peek() {
                Some(Token::Star) => {
                    self.advance();
                    left *= self.parse_unary()?;
                }
                Some(Token::Slash) => {
                    self.advance();
                    let right = self.parse_unary()?;
                    if right == 0.0 {
                        return Err("division by zero".to_string());
                    }
                    left /= right;
                }
                Some(Token::Percent) => {
                    self.advance();
                    let right = self.parse_unary()?;
                    if right == 0.0 {
                        return Err("division by zero".to_string());
                    }
                    left %= right;
                }
                _ => break,
            }
        }
        Ok(left)
    }

    // unary = '-' unary | primary
    fn parse_unary(&mut self) -> Result<f64, String> {
        if let Some(Token::Minus) = self.peek() {
            self.advance();
            let val = self.parse_unary()?;
            return Ok(-val);
        }
        if let Some(Token::Plus) = self.peek() {
            self.advance();
            return self.parse_unary();
        }
        self.parse_primary()
    }

    // primary = number | '(' expr ')' | ident '(' args ')' | constant
    fn parse_primary(&mut self) -> Result<f64, String> {
        match self.advance() {
            Some(Token::Number(n)) => Ok(n),
            Some(Token::LParen) => {
                let val = self.parse_expr()?;
                self.expect(&Token::RParen)?;
                Ok(val)
            }
            Some(Token::Ident(name)) => {
                let lower = name.to_lowercase();
                // Check if it's a function call
                if let Some(Token::LParen) = self.peek() {
                    self.advance(); // consume '('
                    let args = self.parse_arg_list()?;
                    self.expect(&Token::RParen)?;
                    call_function(&lower, &args)
                } else {
                    // Must be a constant
                    match lower.as_str() {
                        "pi" => Ok(std::f64::consts::PI),
                        "e" => Ok(std::f64::consts::E),
                        _ => Err(format!("unknown constant: {name}")),
                    }
                }
            }
            Some(tok) => Err(format!("unexpected token: {tok:?}")),
            None => Err("unexpected end of expression".to_string()),
        }
    }

    fn parse_arg_list(&mut self) -> Result<Vec<f64>, String> {
        let mut args = Vec::new();
        // Handle empty arg list
        if let Some(Token::RParen) = self.peek() {
            return Ok(args);
        }
        args.push(self.parse_expr()?);
        while let Some(Token::Comma) = self.peek() {
            self.advance();
            args.push(self.parse_expr()?);
        }
        Ok(args)
    }
}

fn call_function(name: &str, args: &[f64]) -> Result<f64, String> {
    match name {
        "sqrt" => {
            ensure_args(name, args, 1)?;
            if args[0] < 0.0 {
                return Err("sqrt of negative number".to_string());
            }
            Ok(args[0].sqrt())
        }
        "pow" => {
            ensure_args(name, args, 2)?;
            Ok(args[0].powf(args[1]))
        }
        "abs" => {
            ensure_args(name, args, 1)?;
            Ok(args[0].abs())
        }
        "sin" => {
            ensure_args(name, args, 1)?;
            Ok(args[0].sin())
        }
        "cos" => {
            ensure_args(name, args, 1)?;
            Ok(args[0].cos())
        }
        "tan" => {
            ensure_args(name, args, 1)?;
            Ok(args[0].tan())
        }
        "log" => {
            ensure_args(name, args, 1)?;
            if args[0] <= 0.0 {
                return Err("log of non-positive number".to_string());
            }
            Ok(args[0].log10())
        }
        "ln" => {
            ensure_args(name, args, 1)?;
            if args[0] <= 0.0 {
                return Err("ln of non-positive number".to_string());
            }
            Ok(args[0].ln())
        }
        "floor" => {
            ensure_args(name, args, 1)?;
            Ok(args[0].floor())
        }
        "ceil" => {
            ensure_args(name, args, 1)?;
            Ok(args[0].ceil())
        }
        "round" => {
            ensure_args(name, args, 1)?;
            Ok(args[0].round())
        }
        "min" => {
            ensure_args(name, args, 2)?;
            Ok(args[0].min(args[1]))
        }
        "max" => {
            ensure_args(name, args, 2)?;
            Ok(args[0].max(args[1]))
        }
        _ => Err(format!("unknown function: {name}")),
    }
}

fn ensure_args(name: &str, args: &[f64], expected: usize) -> Result<(), String> {
    if args.len() != expected {
        Err(format!(
            "{name} expects {expected} argument(s), got {}",
            args.len()
        ))
    } else {
        Ok(())
    }
}
