#!/usr/bin/env python3
"""CLI help must work without a terminal or backend and match the in-app guide."""
import os
from pathlib import Path
import subprocess
import sys
import tempfile

root = Path(__file__).resolve().parents[1]
binary = Path(sys.argv[1] if len(sys.argv)>1 else root/'target/debug'/('codex-deck.exe' if os.name=='nt' else 'codex-deck')).resolve()
with tempfile.TemporaryDirectory(prefix='deck-help-') as tmp:
    state = Path(tmp)/'state'
    env = os.environ | {'CODEX_DECK_HOME':str(state), 'CODEX_DECK_CODEX':'missing-backend-for-help-test'}
    help_text = subprocess.check_output([str(binary),'--help'],env=env,text=True)
    assert help_text == subprocess.check_output([str(binary),'-h'],env=env,text=True)
    assert (root/'assets/help.txt').read_text() in help_text
    assert all(command in help_text for command in ['--version','--ipc-path','/side','/btw','/find','/models','/send-parent','Ctrl+G','PgUp/PgDn'])
    assert '\x1b' not in help_text, 'CLI help should be plain text when piped'
    assert not state.exists(), 'help must not launch a daemon or create state'
print('PASS: --help/-h, complete shared command guide, no terminal/backend/state needed')
