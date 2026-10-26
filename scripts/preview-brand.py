#!/usr/bin/env python3
"""Print the actual brand text without Rust, daemon access or model requests."""
import os
import pathlib
import re
import shutil
import sys

root = pathlib.Path(__file__).resolve().parents[1]
brand = (root / 'src/brand.rs').read_text()
def constant(name):
    return re.search(rf'pub const {name}: &str = "([^"]*)";', brand)[1]
width, height = shutil.get_terminal_size((100, 40))
ascii_mode = '--ascii' in sys.argv or os.environ.get('CODEX_DECK_LOGO') == 'ascii' or os.environ.get('TERM') == 'dumb'
color = sys.stdout.isatty() and 'NO_COLOR' not in os.environ and os.environ.get('TERM') != 'dumb'
mint = '\033[38;2;168;237;201m' if color else ''
reset = '\033[0m' if color else ''
name = 'startup-logo.txt' if ascii_mode else ('startup-logo-unicode.txt' if height >= 32 and width >= 48 else 'startup-logo-compact.txt')
lines = (root / 'assets' / name).read_text().splitlines()
art_width = max(map(len, lines))
fits = width >= art_width + 2 and height >= len(lines) + 4
side = fits and width >= art_width + 46
print('$ ./codex-deck\n' + mint, end='')
labels = {8: 'Codex Deck', 10: 'Your sessions, one workspace.', 13: 'Starting workspace...', 15: 'Connecting to local Codex...'}
if fits:
    for i, line in enumerate(lines):
        tail = ' ' * (art_width + 5 - len(line)) + labels[i] if side and i in labels else ''
        print('  ' + line + tail)
if not side:
    print('Codex Deck\nYour sessions, one workspace.\nStarting workspace...\nConnecting to local Codex...')
print(reset + '\n--- Workspace header ---\n' + mint, end='')
print((' o o ' if ascii_mode else constant('EYES')) + ' Codex Deck  CONNECTED · 0 running')
print((' \\_/ ' if ascii_mode else constant('SMILE')) + ' [ Tasks ]  [ Find messages ]  [ Show side ]' + reset)
