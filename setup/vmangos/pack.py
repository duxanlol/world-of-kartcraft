#!/usr/bin/env python3
"""Rebuilds the new-file hunks of kartcraft.patch from setup/vmangos/src/: the Kartcraft
sources live there as plain files and the patch carries them. Run after editing them:

    python3 setup/vmangos/pack.py

The other hunks (the hooks into vmangos's own files) are kept as they are.
"""
import hashlib
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
PATCH = os.path.join(HERE, 'kartcraft.patch')


def hunk(path, text):
    lines = text.split('\n')
    if lines and lines[-1] == '':
        lines.pop()
    data = text.encode()
    blob = hashlib.sha1(b'blob %d\0' % len(data) + data).hexdigest()[:7]
    head = (f'diff --git a/{path} b/{path}\nnew file mode 100644\nindex 0000000..{blob}\n'
            f'--- /dev/null\n+++ b/{path}\n@@ -0,0 +1,{len(lines)} @@\n')
    return head + ''.join(f'+{l}\n' for l in lines)


def main():
    patch = open(PATCH, newline='').read()
    sections = re.split(r'(?m)^(?=diff --git )', patch)
    out = []
    for s in sections:
        m = re.match(r'diff --git a/(\S+) b/\S+', s)
        local = m and os.path.join(HERE, 'src', m.group(1).split('/')[-1])
        if m and '\nnew file mode' in s.split('@@')[0] and os.path.isfile(local):
            out.append(hunk(m.group(1), open(local, newline='').read()))
        else:
            out.append(s)
    packed = ''.join(out)
    if '--check' in sys.argv:
        sys.exit(0 if packed == patch else 1)
    open(PATCH, 'w', newline='\n').write(packed)


if __name__ == '__main__':
    main()
