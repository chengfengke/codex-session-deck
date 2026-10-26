use std::io::{self, IsTerminal, Write};

pub const EYES: &str = " ⠇ ⠸ ";
pub const SMILE: &str = " ⠢⣀⠔ ";
const MASCOT: &str = include_str!("../assets/startup-logo-unicode.txt");
const COMPACT: &str = include_str!("../assets/startup-logo-compact.txt");
const ASCII: &str = include_str!("../assets/startup-logo.txt");

pub fn ascii_mode() -> bool {
    std::env::var("CODEX_DECK_LOGO").as_deref() == Ok("ascii")
        || std::env::var("TERM").as_deref() == Ok("dumb")
}

pub fn eyes() -> &'static str {
    if ascii_mode() {
        " o o "
    } else {
        EYES
    }
}

pub fn smile() -> &'static str {
    if ascii_mode() {
        " \\_/ "
    } else {
        SMILE
    }
}

/// Print text only, before entering the alternate screen. Braille cells are
/// fixed-width 2x4-dot tiles, giving curves more detail than ASCII punctuation.
pub fn print_startup() -> io::Result<()> {
    let mut out = io::stdout();
    if !out.is_terminal() {
        return Ok(());
    }
    let color =
        std::env::var_os("NO_COLOR").is_none() && std::env::var("TERM").as_deref() != Ok("dumb");
    if color {
        write!(out, "\x1b[38;2;168;237;201m")?;
    }
    let (width, height) = crossterm::terminal::size().unwrap_or((80, 24));
    let art = if ascii_mode() {
        ASCII
    } else if height >= 32 && width >= 48 {
        MASCOT
    } else {
        COMPACT
    };
    let lines: Vec<_> = art.lines().collect();
    let art_width = lines
        .iter()
        .map(|line| line.chars().count())
        .max()
        .unwrap_or(0);
    let fits = width as usize >= art_width + 2 && height as usize >= lines.len() + 4;
    let side = fits && width as usize >= art_width + 46;
    if fits {
        for (i, line) in lines.iter().enumerate() {
            write!(out, "  {line}")?;
            if side {
                let label = match i {
                    8 => "Codex Deck",
                    10 => "Your sessions, one workspace.",
                    13 => "Starting workspace...",
                    15 => "Connecting to local Codex...",
                    _ => "",
                };
                if !label.is_empty() {
                    write!(
                        out,
                        "{}{label}",
                        " ".repeat(art_width + 5 - line.chars().count())
                    )?;
                }
            }
            writeln!(out)?;
        }
    }
    if !side {
        writeln!(out, "Codex Deck\nYour sessions, one workspace.")?;
        writeln!(out, "Starting workspace...\nConnecting to local Codex...")?;
    }
    if color {
        write!(out, "\x1b[0m")?;
    }
    out.flush()
}
