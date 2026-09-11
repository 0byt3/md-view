//! Small TeX-math subset used by `example.md` display/inline `$$...$$`.
//!
//! Covers `\frac`, `\sum`/`\lim` limits, super/subscripts, and a handful of
//! symbols (`\partial`, `\to`). Anything else is emitted as literal text so
//! unknown markup never drops the source.

/// One laid-out math fragment.
#[derive(Debug, Clone, PartialEq)]
pub enum Atom {
    Text(String),
    Row(Vec<Atom>),
    Frac(Box<Atom>, Box<Atom>),
    Scripts {
        base: Box<Atom>,
        sub: Option<Box<Atom>>,
        sup: Option<Box<Atom>>,
        /// Display-style operators put limits above/below instead of beside.
        limits: bool,
    },
}

/// Parse a TeX math fragment into an [`Atom`] tree.
pub fn parse_tex(src: &str) -> Atom {
    let chars: Vec<char> = src.chars().collect();
    let mut i = 0;
    let atom = parse_row(&chars, &mut i, false);
    skip_space(&chars, &mut i);
    atom
}

/// Flatten to a searchable approximation (symbols, no braces).
pub fn flatten_tex(src: &str) -> String {
    flatten_atom(&parse_tex(src))
}

fn flatten_atom(atom: &Atom) -> String {
    match atom {
        Atom::Text(text) => text.clone(),
        Atom::Row(items) => items.iter().map(flatten_atom).collect(),
        Atom::Frac(num, den) => format!("({}/{})", flatten_atom(num), flatten_atom(den)),
        Atom::Scripts { base, sub, sup, .. } => {
            let mut out = flatten_atom(base);
            if let Some(sub) = sub {
                out.push_str(&format!("_{}", flatten_atom(sub)));
            }
            if let Some(sup) = sup {
                out.push_str(&format!("^{}", flatten_atom(sup)));
            }
            out
        }
    }
}

fn parse_row(chars: &[char], i: &mut usize, stop_brace: bool) -> Atom {
    let mut items = Vec::new();
    loop {
        if *i >= chars.len() {
            break;
        }
        if stop_brace && chars[*i] == '}' {
            break;
        }
        if chars[*i].is_whitespace() {
            skip_space(chars, i);
            items.push(Atom::Text(" ".to_string()));
            continue;
        }
        items.push(parse_scripted(chars, i));
    }
    match items.len() {
        0 => Atom::Text(String::new()),
        1 => items.pop().unwrap(),
        _ => Atom::Row(items),
    }
}

fn parse_scripted(chars: &[char], i: &mut usize) -> Atom {
    let mut atom = parse_primary(chars, i);
    loop {
        let before_space = *i;
        skip_space(chars, i);
        if *i >= chars.len() {
            *i = before_space;
            break;
        }
        match chars[*i] {
            '^' | '_' => {
                let mut sub = None;
                let mut sup = None;
                while *i < chars.len() && (chars[*i] == '^' || chars[*i] == '_') {
                    let is_sup = chars[*i] == '^';
                    *i += 1;
                    skip_space(chars, i);
                    let script = parse_primary(chars, i);
                    if is_sup {
                        sup = Some(Box::new(script));
                    } else {
                        sub = Some(Box::new(script));
                    }
                }
                let limits = matches!(&atom, Atom::Text(t) if t == "∑" || t == "lim");
                atom = Atom::Scripts {
                    base: Box::new(atom),
                    sub,
                    sup,
                    limits,
                };
            }
            _ => {
                *i = before_space;
                break;
            }
        }
    }
    atom
}

fn parse_primary(chars: &[char], i: &mut usize) -> Atom {
    skip_space(chars, i);
    if *i >= chars.len() {
        return Atom::Text(String::new());
    }
    match chars[*i] {
        '{' => {
            *i += 1;
            let inner = parse_row(chars, i, true);
            if *i < chars.len() && chars[*i] == '}' {
                *i += 1;
            }
            inner
        }
        '\\' => parse_command(chars, i),
        c => {
            *i += 1;
            Atom::Text(c.to_string())
        }
    }
}

fn parse_command(chars: &[char], i: &mut usize) -> Atom {
    *i += 1;
    let mut name = String::new();
    while *i < chars.len() && chars[*i].is_ascii_alphabetic() {
        name.push(chars[*i]);
        *i += 1;
    }
    match name.as_str() {
        "frac" => {
            let num = parse_primary(chars, i);
            let den = parse_primary(chars, i);
            Atom::Frac(Box::new(num), Box::new(den))
        }
        "partial" => Atom::Text("∂".to_string()),
        "sum" => Atom::Text("∑".to_string()),
        "lim" => Atom::Text("lim".to_string()),
        "to" => Atom::Text("→".to_string()),
        "infty" => Atom::Text("∞".to_string()),
        "cdot" => Atom::Text("·".to_string()),
        "left" | "right" => parse_primary(chars, i),
        other => {
            if other.is_empty() && *i < chars.len() {
                let c = chars[*i];
                *i += 1;
                Atom::Text(c.to_string())
            } else {
                Atom::Text(other.to_string())
            }
        }
    }
}

fn skip_space(chars: &[char], i: &mut usize) {
    while *i < chars.len() && chars[*i].is_whitespace() {
        *i += 1;
    }
}

/// Map a run of characters to Unicode super/subscripts when possible.
pub fn unicode_script(text: &str, superscript: bool) -> String {
    text.chars()
        .map(|c| script_char(c, superscript).unwrap_or(c))
        .collect()
}

fn script_char(c: char, superscript: bool) -> Option<char> {
    if superscript {
        Some(match c {
            '0' => '⁰',
            '1' => '¹',
            '2' => '²',
            '3' => '³',
            '4' => '⁴',
            '5' => '⁵',
            '6' => '⁶',
            '7' => '⁷',
            '8' => '⁸',
            '9' => '⁹',
            '+' => '⁺',
            '-' => '⁻',
            '=' => '⁼',
            '(' => '⁽',
            ')' => '⁾',
            'n' => 'ⁿ',
            'i' => 'ⁱ',
            _ => return None,
        })
    } else {
        Some(match c {
            '0' => '₀',
            '1' => '₁',
            '2' => '₂',
            '3' => '₃',
            '4' => '₄',
            '5' => '₅',
            '6' => '₆',
            '7' => '₇',
            '8' => '₈',
            '9' => '₉',
            '+' => '₊',
            '-' => '₋',
            '=' => '₌',
            '(' => '₍',
            ')' => '₎',
            'n' => 'ₙ',
            'i' => 'ᵢ',
            'a' => 'ₐ',
            'e' => 'ₑ',
            'o' => 'ₒ',
            'x' => 'ₓ',
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn e_equals_mc_squared() {
        let atom = parse_tex("E = mc^2");
        assert_eq!(flatten_atom(&atom), "E = mc^2");
        match atom {
            Atom::Row(items) => {
                assert!(matches!(&items[0], Atom::Text(t) if t == "E"));
                assert!(
                    items.iter().any(|item| matches!(
                        item,
                        Atom::Scripts { sup, .. } if matches!(sup.as_deref(), Some(Atom::Text(t)) if t == "2")
                    )),
                    "{items:?}"
                );
            }
            other => panic!("expected row, got {other:?}"),
        }
    }

    #[test]
    fn derivative_and_sum() {
        let deriv =
            parse_tex(r"\frac{\partial f}{\partial x} = \lim_{h \to 0} \frac{f(x+h) - f(x)}{h}");
        let flat = flatten_atom(&deriv);
        assert!(flat.contains('∂'), "{flat}");
        assert!(flat.contains("lim"), "{flat}");
        assert!(flat.contains('→'), "{flat}");

        let sum = parse_tex(r"\sum_{i=1}^{n} i^2 = \frac{n(n+1)(2n+1)}{6}");
        let flat = flatten_atom(&sum);
        assert!(flat.contains("∑"), "{flat}");
        assert!(flat.contains("_i=1"), "{flat}");
        assert!(flat.contains("^n"), "{flat}");
        assert!(flat.contains("(n(n+1)(2n+1)/6)"), "{flat}");
    }

    #[test]
    fn unicode_sub_and_sup() {
        assert_eq!(unicode_script("2", false), "₂");
        assert_eq!(unicode_script("2", true), "²");
        assert_eq!(unicode_script("iπ", true), "ⁱπ");
    }
}
