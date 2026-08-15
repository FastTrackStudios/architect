//! Calculator provider.
//!
//! Evaluates simple math expressions inline.
//! Triggered by the '=' prefix or when the query looks like math.

use architect_launcher_core::{ActivationResult, Item, Provider, ProviderConfig};

pub struct CalcProvider {
    config: ProviderConfig,
}

impl CalcProvider {
    pub fn new() -> Self {
        Self {
            config: ProviderConfig {
                name: "calc".into(),
                icon: "accessories-calculator".into(),
                prefix: Some('='),
                ..Default::default()
            },
        }
    }
}

impl Default for CalcProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl Provider for CalcProvider {
    fn name(&self) -> &str {
        "calc"
    }

    fn config(&self) -> &ProviderConfig {
        &self.config
    }

    fn config_mut(&mut self) -> &mut ProviderConfig {
        &mut self.config
    }

    fn query(
        &self,
        query: &str,
        _exact: bool,
    ) -> Result<Vec<Item>, Box<dyn std::error::Error + Send + Sync>> {
        if query.is_empty() {
            return Ok(vec![]);
        }

        // Simple expression evaluator
        match eval_expr(query) {
            Some(result) => {
                let label = format!("{result}");
                let mut item =
                    Item::new("calc-result", &label, "calc").with_tags(&["tools/calculator"]);
                item.sub = format!("{query} = {result}");
                item.icon = "accessories-calculator".into();
                item.score = 1000.0; // Always show calc results at top when prefix is used
                item.search_fields = vec![]; // Don't fuzzy match calc results
                Ok(vec![item])
            }
            None => Ok(vec![]),
        }
    }

    fn activate(
        &self,
        item: &Item,
        _action: &str,
    ) -> Result<ActivationResult, Box<dyn std::error::Error + Send + Sync>> {
        // Copy result to clipboard (best-effort)
        let _ = std::process::Command::new("wl-copy")
            .arg(&item.label)
            .spawn();
        Ok(ActivationResult::Close)
    }
}

/// Minimal recursive-descent expression evaluator.
/// Supports: +, -, *, /, parentheses, decimals, negative numbers.
fn eval_expr(input: &str) -> Option<f64> {
    let tokens = tokenize(input)?;
    let mut pos = 0;
    let result = parse_addition(&tokens, &mut pos)?;
    if pos == tokens.len() {
        // Round to avoid floating point display noise
        let rounded = (result * 1_000_000_000.0).round() / 1_000_000_000.0;
        Some(rounded)
    } else {
        None
    }
}

#[derive(Debug, Clone)]
enum Token {
    Num(f64),
    Op(char),
    LParen,
    RParen,
}

fn tokenize(input: &str) -> Option<Vec<Token>> {
    let mut tokens = Vec::new();
    let mut chars = input.chars().peekable();

    while let Some(&ch) = chars.peek() {
        match ch {
            ' ' | '\t' => {
                chars.next();
            }
            '0'..='9' | '.' => {
                let mut num_str = String::new();
                while let Some(&c) = chars.peek() {
                    if c.is_ascii_digit() || c == '.' {
                        num_str.push(c);
                        chars.next();
                    } else {
                        break;
                    }
                }
                let n: f64 = num_str.parse().ok()?;
                tokens.push(Token::Num(n));
            }
            '+' | '-' | '*' | '/' | '^' | '%' => {
                tokens.push(Token::Op(ch));
                chars.next();
            }
            '(' => {
                tokens.push(Token::LParen);
                chars.next();
            }
            ')' => {
                tokens.push(Token::RParen);
                chars.next();
            }
            _ => return None,
        }
    }

    Some(tokens)
}

fn parse_addition(tokens: &[Token], pos: &mut usize) -> Option<f64> {
    let mut left = parse_multiplication(tokens, pos)?;
    while *pos < tokens.len() {
        match tokens[*pos] {
            Token::Op('+') => {
                *pos += 1;
                left += parse_multiplication(tokens, pos)?;
            }
            Token::Op('-') => {
                *pos += 1;
                left -= parse_multiplication(tokens, pos)?;
            }
            _ => break,
        }
    }
    Some(left)
}

fn parse_multiplication(tokens: &[Token], pos: &mut usize) -> Option<f64> {
    let mut left = parse_power(tokens, pos)?;
    while *pos < tokens.len() {
        match tokens[*pos] {
            Token::Op('*') => {
                *pos += 1;
                left *= parse_power(tokens, pos)?;
            }
            Token::Op('/') => {
                *pos += 1;
                let right = parse_power(tokens, pos)?;
                if right == 0.0 {
                    return None;
                }
                left /= right;
            }
            Token::Op('%') => {
                *pos += 1;
                let right = parse_power(tokens, pos)?;
                if right == 0.0 {
                    return None;
                }
                left %= right;
            }
            _ => break,
        }
    }
    Some(left)
}

fn parse_power(tokens: &[Token], pos: &mut usize) -> Option<f64> {
    let base = parse_unary(tokens, pos)?;
    if *pos < tokens.len()
        && let Token::Op('^') = tokens[*pos]
    {
        *pos += 1;
        let exp = parse_power(tokens, pos)?; // right-associative
        return Some(base.powf(exp));
    }
    Some(base)
}

fn parse_unary(tokens: &[Token], pos: &mut usize) -> Option<f64> {
    if *pos < tokens.len() {
        if let Token::Op('-') = tokens[*pos] {
            *pos += 1;
            let val = parse_atom(tokens, pos)?;
            return Some(-val);
        }
        if let Token::Op('+') = tokens[*pos] {
            *pos += 1;
            return parse_atom(tokens, pos);
        }
    }
    parse_atom(tokens, pos)
}

fn parse_atom(tokens: &[Token], pos: &mut usize) -> Option<f64> {
    if *pos >= tokens.len() {
        return None;
    }
    match &tokens[*pos] {
        Token::Num(n) => {
            let v = *n;
            *pos += 1;
            Some(v)
        }
        Token::LParen => {
            *pos += 1;
            let val = parse_addition(tokens, pos)?;
            if *pos < tokens.len()
                && let Token::RParen = tokens[*pos]
            {
                *pos += 1;
                return Some(val);
            }
            None
        }
        _ => None,
    }
}
