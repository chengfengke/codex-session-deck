use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

/// A terminal representation, not a TeX typesetter. Unsupported commands retain
/// their source, and the stored message is never changed by presentation.
pub fn math(source: &str) -> String {
    if source.len() > 256 * 1024 {
        return source.into();
    }
    Math {
        chars: source.chars().collect(),
        pos: 0,
        depth: 0,
    }
    .parse()
}

struct Math {
    chars: Vec<char>,
    pos: usize,
    depth: usize,
}
impl Math {
    fn parse(&mut self) -> String {
        let mut out = String::new();
        while let Some(&ch) = self.chars.get(self.pos) {
            self.pos += 1;
            match ch {
                '\\' => out.push_str(&self.command()),
                '{' => {
                    self.pos -= 1;
                    if let Some(arg) = self.argument() {
                        out.push('(');
                        out.push_str(&self.nested(&arg));
                        out.push(')');
                    } else {
                        self.pos += 1;
                        out.push('{');
                    }
                }
                '^' | '_' => {
                    let Some(arg) = self.argument() else {
                        out.push(ch);
                        continue;
                    };
                    let arg = self.nested(&arg);
                    let translated: Option<String> =
                        arg.chars().map(|c| script(c, ch == '^')).collect();
                    if let Some(s) = translated {
                        out.push_str(&s);
                    } else if arg.chars().count() == 1 {
                        out.push(ch);
                        out.push_str(&arg);
                    } else {
                        out.push(ch);
                        out.push('(');
                        out.push_str(&arg);
                        out.push(')');
                    }
                }
                '&' => out.push(' '),
                _ => out.push(ch),
            }
        }
        out
    }
    fn nested(&self, text: &str) -> String {
        if self.depth >= 24 {
            return format!("{{{text}}}");
        }
        Math {
            chars: text.chars().collect(),
            pos: 0,
            depth: self.depth + 1,
        }
        .parse()
    }
    fn argument(&mut self) -> Option<String> {
        let start = self.pos;
        while self.chars.get(self.pos).is_some_and(|c| c.is_whitespace()) {
            self.pos += 1;
        }
        match self.chars.get(self.pos).copied() {
            Some('{') => {
                self.pos += 1;
                let body = self.pos;
                let mut depth = 1;
                while let Some(&ch) = self.chars.get(self.pos) {
                    if ch == '\\' {
                        self.pos += 2;
                        continue;
                    }
                    if ch == '{' {
                        depth += 1;
                    }
                    if ch == '}' {
                        depth -= 1;
                    }
                    if depth == 0 {
                        let result = self.chars[body..self.pos].iter().collect();
                        self.pos += 1;
                        return Some(result);
                    }
                    self.pos += 1;
                }
                self.pos = start;
                None
            }
            Some('\\') => {
                let body = self.pos;
                self.pos += 1;
                while self
                    .chars
                    .get(self.pos)
                    .is_some_and(char::is_ascii_alphabetic)
                {
                    self.pos += 1;
                }
                if self.pos == body + 1 && self.pos < self.chars.len() {
                    self.pos += 1;
                }
                Some(self.chars[body..self.pos].iter().collect())
            }
            Some(ch) => {
                self.pos += 1;
                Some(ch.to_string())
            }
            None => {
                self.pos = start;
                None
            }
        }
    }
    fn command(&mut self) -> String {
        let start = self.pos;
        while self
            .chars
            .get(self.pos)
            .is_some_and(char::is_ascii_alphabetic)
        {
            self.pos += 1;
        }
        if start == self.pos {
            return match self.chars.get(self.pos).copied() {
                Some(ch) => {
                    self.pos += 1;
                    match ch {
                        ',' | ';' | ':' | ' ' => " ".into(),
                        '!' => String::new(),
                        '\\' => "\n".into(),
                        _ => ch.to_string(),
                    }
                }
                None => "\\".into(),
            };
        }
        let name: String = self.chars[start..self.pos].iter().collect();
        let end = self.pos;
        let symbol = match name.as_str() {
            "alpha" => "α",
            "beta" => "β",
            "gamma" => "γ",
            "delta" => "δ",
            "epsilon" | "varepsilon" => "ε",
            "theta" => "θ",
            "lambda" => "λ",
            "mu" => "μ",
            "nu" => "ν",
            "pi" => "π",
            "rho" => "ρ",
            "sigma" => "σ",
            "tau" => "τ",
            "phi" | "varphi" => "φ",
            "chi" => "χ",
            "psi" => "ψ",
            "omega" => "ω",
            "eta" => "η",
            "zeta" => "ζ",
            "Gamma" => "Γ",
            "Delta" => "Δ",
            "Theta" => "Θ",
            "Lambda" => "Λ",
            "Pi" => "Π",
            "Sigma" => "Σ",
            "Phi" => "Φ",
            "Omega" => "Ω",
            "sum" => "∑",
            "prod" => "∏",
            "int" => "∫",
            "infty" => "∞",
            "partial" => "∂",
            "nabla" => "∇",
            "cdot" => "·",
            "times" => "×",
            "div" => "÷",
            "pm" => "±",
            "mp" => "∓",
            "le" | "leq" => "≤",
            "ge" | "geq" => "≥",
            "ne" | "neq" => "≠",
            "approx" => "≈",
            "equiv" => "≡",
            "in" => "∈",
            "notin" => "∉",
            "subset" => "⊂",
            "subseteq" => "⊆",
            "cup" => "∪",
            "cap" => "∩",
            "emptyset" => "∅",
            "to" | "rightarrow" => "→",
            "leftarrow" => "←",
            "Rightarrow" | "implies" => "⇒",
            "Leftrightarrow" => "⇔",
            "forall" => "∀",
            "exists" => "∃",
            "neg" => "¬",
            "land" => "∧",
            "lor" => "∨",
            "ldots" | "dots" => "…",
            "cdots" => "⋯",
            "langle" => "⟨",
            "rangle" => "⟩",
            "lVert" | "rVert" | "Vert" => "‖",
            "quad" => " ",
            "qquad" => "  ",
            "left" | "right" | "displaystyle" => "",
            "cos" | "sin" | "tan" | "log" | "ln" | "exp" | "min" | "max" | "lim" | "det" => {
                return name
            }
            _ => {
                let formatted = match name.as_str() {
                    "frac" | "dfrac" | "tfrac" => match (self.argument(), self.argument()) {
                        (Some(a), Some(b)) => {
                            Some(format!("({})/({})", self.nested(&a), self.nested(&b)))
                        }
                        _ => None,
                    },
                    "sqrt" => {
                        if self.chars.get(self.pos) == Some(&'[') {
                            let start = self.pos + 1;
                            self.chars[start..]
                                .iter()
                                .position(|&c| c == ']')
                                .and_then(|len| {
                                    let index: String =
                                        self.chars[start..start + len].iter().collect();
                                    self.pos = start + len + 1;
                                    self.argument().map(|a| {
                                        format!(
                                            "root({}, {})",
                                            self.nested(&index),
                                            self.nested(&a)
                                        )
                                    })
                                })
                        } else {
                            self.argument().map(|a| format!("√({})", self.nested(&a)))
                        }
                    }
                    "boxed" | "mathrm" | "mathbf" | "mathit" | "mathsf" | "operatorname" => {
                        self.argument().map(|a| self.nested(&a))
                    }
                    "text" | "textrm" | "textbf" => self.argument(),
                    "hat" | "bar" | "vec" | "dot" | "tilde" => self
                        .argument()
                        .map(|a| format!("{}({})", name, self.nested(&a))),
                    "begin" | "end" => self
                        .argument()
                        .filter(|a| {
                            matches!(a.as_str(), "aligned" | "align" | "equation" | "gathered")
                        })
                        .map(|_| String::new()),
                    _ => None,
                };
                if let Some(s) = formatted {
                    return s;
                }
                self.pos = end;
                let mut raw = format!("\\{name}");
                // Preserve all groups attached to an unsupported command.
                while self.chars.get(self.pos) == Some(&'{') {
                    let start = self.pos;
                    if self.argument().is_some() {
                        raw.extend(self.chars[start..self.pos].iter());
                    } else {
                        raw.extend(self.chars[self.pos..].iter());
                        self.pos = self.chars.len();
                    }
                }
                return raw;
            }
        };
        symbol.into()
    }
}

fn script(ch: char, superscript: bool) -> Option<char> {
    let (from, to) = if superscript {
        ("0123456789+-=()in", "⁰¹²³⁴⁵⁶⁷⁸⁹⁺⁻⁼⁽⁾ⁱⁿ")
    } else {
        (
            "0123456789+-=()aehijklmnoprstuvx",
            "₀₁₂₃₄₅₆₇₈₉₊₋₌₍₎ₐₑₕᵢⱼₖₗₘₙₒₚᵣₛₜᵤᵥₓ",
        )
    };
    from.chars()
        .position(|c| c == ch)
        .and_then(|i| to.chars().nth(i))
}

/// Remove terminal control sequences; tools may emit ANSI even over stdio.
pub fn clean(text: &str) -> String {
    let mut chars = text.chars().peekable();
    let mut out = String::new();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            match chars.next() {
                Some('[') => {
                    for c in chars.by_ref() {
                        if ('@'..='~').contains(&c) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    while let Some(c) = chars.next() {
                        if c == '\u{7}' {
                            break;
                        }
                        if c == '\u{1b}' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                }
                _ => {}
            }
        } else if ch == '\r' {
            if chars.peek() != Some(&'\n') {
                out.push('\n');
            }
        } else if ch == '\t' {
            out.push_str("    ");
        } else if !ch.is_control() || ch == '\n' {
            out.push(ch);
        }
    }
    out
}

fn math_delimiters(text: &str) -> String {
    let mut result = String::new();
    let mut at = 0;
    while at < text.len() {
        let rest = &text[at..];
        if rest.starts_with('`') {
            let ticks = rest.bytes().take_while(|&c| c == b'`').count();
            let marker = "`".repeat(ticks);
            if let Some(end) = rest[ticks..].find(&marker) {
                let len = ticks + end + ticks;
                result.push_str(&rest[..len]);
                at += len;
                continue;
            }
            result.push_str(rest);
            break;
        }
        if rest.starts_with("\\$") {
            result.push('$');
            at += 2;
            continue;
        }
        let delimiter = if rest.starts_with("\\[") {
            Some(("\\[", "\\]"))
        } else if rest.starts_with("\\(") {
            Some(("\\(", "\\)"))
        } else if rest.starts_with("$$") {
            Some(("$$", "$$"))
        } else if rest.starts_with('$')
            && rest[1..].chars().next().is_some_and(|c| !c.is_whitespace())
        {
            Some(("$", "$"))
        } else {
            None
        };
        if let Some((open, close)) = delimiter {
            if let Some(end) = rest[open.len()..].find(close) {
                let tail = open.len() + end + close.len();
                let body = &rest[open.len()..open.len() + end];
                if open != "$"
                    || (body.chars().last().is_some_and(|c| !c.is_whitespace())
                        && !rest[tail..]
                            .chars()
                            .next()
                            .is_some_and(|c| c.is_ascii_digit()))
                {
                    result.push_str(&math(rest[open.len()..open.len() + end].trim()));
                    at += tail;
                    continue;
                }
            }
        }
        let ch = rest.chars().next().unwrap();
        result.push(ch);
        at += ch.len_utf8();
    }
    result
}

fn inline(text: &str, style: Style) -> Vec<Span<'static>> {
    let mut spans = vec![];
    let mut rest = text;
    while !rest.is_empty() {
        let marker = if rest.starts_with('`') {
            "`"
        } else if rest.starts_with("**") {
            "**"
        } else {
            ""
        };
        if !marker.is_empty() {
            let count = if marker == "`" {
                rest.bytes().take_while(|&b| b == b'`').count()
            } else {
                2
            };
            let marker = marker.chars().next().unwrap().to_string().repeat(count);
            if let Some(end) = rest[count..].find(&marker) {
                let body = rest[count..count + end].to_owned();
                let s = if marker.starts_with('`') {
                    style.fg(Color::Cyan)
                } else {
                    style.add_modifier(Modifier::BOLD)
                };
                spans.push(Span::styled(body, s));
                rest = &rest[count + end + count..];
                continue;
            }
        }
        let start = rest
            .char_indices()
            .skip(1)
            .find(|(_, c)| matches!(c, '`' | '*'))
            .map(|(i, _)| i)
            .unwrap_or(rest.len());
        spans.push(Span::styled(rest[..start].to_owned(), style));
        rest = &rest[start..];
    }
    spans
}

pub fn markdown(text: &str, style: Style) -> Vec<Line<'static>> {
    let text = clean(text);
    let mut lines = vec![];
    let mut prose = String::new();
    let mut fence: Option<(char, usize)> = None;
    fn flush(prose: &mut String, lines: &mut Vec<Line<'static>>, style: Style) {
        if prose.is_empty() {
            return;
        }
        for line in math_delimiters(prose).lines() {
            let trimmed = line.trim_start();
            let hashes = trimmed.chars().take_while(|&c| c == '#').count();
            if (1..=6).contains(&hashes) && trimmed.as_bytes().get(hashes) == Some(&b' ') {
                lines.push(Line::from(inline(
                    &trimmed[hashes + 1..],
                    style.add_modifier(Modifier::BOLD),
                )));
            } else {
                lines.push(Line::from(inline(line, style)));
            }
        }
        prose.clear();
    }
    for line in text.lines() {
        let trimmed = line.trim_start();
        let first = trimmed.chars().next();
        let count = first
            .map(|c| trimmed.chars().take_while(|&x| x == c).count())
            .unwrap_or_default();
        let marker = first.is_some_and(|c| c == '`' || c == '~') && count >= 3;
        if let Some((ch, len)) = fence {
            if marker && first == Some(ch) && count >= len && trimmed[count..].trim().is_empty() {
                fence = None;
            } else {
                lines.push(Line::styled(line.to_owned(), style.fg(Color::Cyan)));
            }
        } else if marker {
            flush(&mut prose, &mut lines, style);
            fence = Some((first.unwrap(), count));
            let language = trimmed[count..].trim();
            if !language.is_empty() {
                lines.push(Line::styled(
                    format!("┌ {language}"),
                    style.add_modifier(Modifier::DIM),
                ));
            }
        } else {
            prose.push_str(line);
            prose.push('\n');
        }
    }
    flush(&mut prose, &mut lines, style);
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    fn display(text: &str) -> String {
        markdown(text, Style::default())
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
    #[test]
    fn renders_reported_dtw_equation_and_boxed_range() {
        let text = r"归一化 DTW：
\[
d_{ij}=\operatorname{clip}(1-\cos(e_i,e_j),\,0,\,2)
\]
值域：\(\boxed{[0,2]}\)";
        let rendered = display(text);
        assert!(
            rendered.contains("dᵢⱼ=clip(1-cos(eᵢ,eⱼ), 0, 2)"),
            "{rendered}"
        );
        assert!(rendered.contains("值域：[0,2]"));
        assert!(!rendered.contains("\\["));
    }
    #[test]
    fn preserves_code_unknown_macros_and_incomplete_streams() {
        let input =
            "**Result** `x_i`\n```python\nprint(r'\\boxed{x}')\n```\n$\\unknown{x}$\n\\[unfinished";
        let rendered = display(input);
        assert!(rendered.contains("Result x_i"));
        assert!(rendered.contains("print(r'\\boxed{x}')"));
        assert!(rendered.contains("\\unknown{x}"));
        assert!(rendered.contains("\\[unfinished"));
        assert_eq!(math(r"\frac{a"), r"\frac{a");
        assert_eq!(display("Costs $5 and $10; $x^2$"), "Costs $5 and $10; x²");
    }
    #[test]
    fn handles_nested_math_unicode_and_terminal_control_codes() {
        assert_eq!(math(r"{a+b}c + \sqrt[3]{x}"), "(a+b)c + root(3, x)");
        assert_eq!(
            math(r"\frac{\alpha+\beta}{\sqrt{x_i}} \leq n^2"),
            "(α+β)/(√(xᵢ)) ≤ n²"
        );
        assert_eq!(clean("\u{1b}[31mred\u{1b}[0m\r\nready"), "red\nready");
        assert_eq!(clean("\u{1b}]0;title\u{7}text"), "text");
        for end in "中文 \\[x_i+\\frac{1}{2}\\]".char_indices().map(|(i, _)| i) {
            let _ = display(&"中文 \\[x_i+\\frac{1}{2}\\]"[..end]);
        }
    }
}
