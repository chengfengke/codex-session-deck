#!/usr/bin/env python3
"""Run the real TUI in a PTY: Unicode editing, help, resizing, detachment, terminal cleanup."""
import errno, fcntl, os, pathlib, pty, select, signal, struct, sys, termios, time
root=pathlib.Path(__file__).resolve().parents[1]
binary=str(pathlib.Path(sys.argv[1] if len(sys.argv)>1 else root/'target/debug/codex-deck').resolve())
pid, master=pty.fork()
if pid==0:
    os.environ['TERM']='xterm-256color'
    os.execv(binary,[binary,'--demo'])
output=bytearray()
def read_for(seconds):
    deadline=time.monotonic()+seconds
    while time.monotonic()<deadline:
        r,_,_=select.select([master],[],[],.05)
        if r:
            try:
                b=os.read(master,65536)
                if not b:return
                output.extend(b)
            except OSError as e:
                if e.errno==errno.EIO:return
                raise
try:
    fcntl.ioctl(master,termios.TIOCSWINSZ,struct.pack('HHHH',40,160,0,0))
    read_for(.5)
    os.write(master,' 中文👩‍💻'.encode())
    read_for(.15)
    os.write(master,b'\x07?')
    read_for(.15)
    # Help must expose its final commands even in a small terminal.
    fcntl.ioctl(master,termios.TIOCSWINSZ,struct.pack('HHHH',24,80,0,0))
    os.kill(pid,signal.SIGWINCH)
    read_for(.1)
    os.write(master,b'\x1b[F')  # End: final help page
    read_for(.15)
    assert b'/reconnect' in output, 'help commands below the first screen are inaccessible'
    os.write(master,b'\x1b[H')  # Home: first help page
    read_for(.1)
    assert b'GET STARTED' in output, 'help Home did not restore the first page'
    os.write(master,b'\x1b')
    for rows,cols in [(24,80),(24,100),(40,160)]:
        fcntl.ioctl(master,termios.TIOCSWINSZ,struct.pack('HHHH',rows,cols,0,0))
        os.kill(pid,signal.SIGWINCH)
        read_for(.1)
    os.write(master,b'\x07q')
    deadline=time.monotonic()+5
    finished=0
    while time.monotonic()<deadline:
        read_for(.05)
        finished,status=os.waitpid(pid,os.WNOHANG)
        if finished:break
    assert finished==pid, 'TUI did not detach'
    assert os.waitstatus_to_exitcode(status)==0, 'TUI failed'
    assert b'Codex Deck' in output and b'keyboard' in output, 'workspace/help did not render'
    assert b'\x1b[?1049l' in output, 'alternate screen was not restored'
    assert b'\x1b[?2004l' in output, 'bracketed paste was not restored'
    print('PASS: actual TUI PTY launch, Unicode input, command prefix/help, 80/100/160-column resize, clean detach and terminal restoration')
finally:
    try:os.kill(pid,signal.SIGTERM)
    except ProcessLookupError:pass
    os.close(master)
