"""World of Kartcraft account sign-up: one page that makes a game account on the vmangos server.

No email and no third-party service: a captcha image drawn here, used once and expiring, plus
per-address and daily limits, keep it from being scripted. It only ever creates accounts (an
existing name is refused, never overwritten), as ordinary players, through a database user that
can do nothing else.

Configuration (environment):
  DB_HOST, DB_USER, DB_PASSWORD   the realmd database (DB_HOST defaults to `database`)
  REALM_ADDRESS                   shown on the page: what players set their client to
  PER_IP_PER_DAY                  accounts one address may make a day (default 3)
  PER_DAY                         accounts the page makes a day in all (default 100)
"""
import base64
import hashlib
import io
import os
import re
import secrets
import threading
import time
from collections import deque
from html import escape

import pymysql
from captcha.image import ImageCaptcha
from flask import Flask, request

N = int('894B645E89E1535BBDAD5B8B290650530801B18EBFBF5E8FAB3C82872A3E9BB7', 16)
G = 7
NAME = re.compile(r'^[A-Za-z0-9]{3,16}$')
# The 1.12 login screen takes up to 16 characters; SRP6 upper-cases the password.
PASSWORD = re.compile(r'^[\x21-\x7e]{4,16}$')
# No 0/O/Q, 1/I/L, 5/S, 2/Z, 8/B, 6/G: what a player can't tell apart in a distorted image.
ALPHABET = 'ACDEFHJKMNPRTUVWXY34679'
CAPTCHA_TTL = 600
CAPTCHA_MAX = 5000
DAY = 86400
HOUR = 3600

PER_IP_PER_DAY = int(os.environ.get('PER_IP_PER_DAY', '3'))
PER_DAY = int(os.environ.get('PER_DAY', '100'))
REALM_ADDRESS = os.environ.get('REALM_ADDRESS', '')

app = Flask(__name__)
app.config['MAX_CONTENT_LENGTH'] = 4096
lock = threading.Lock()
captchas = {}             # token -> (answer, expiry), in insertion order
made = deque()            # (time, address) of every account made, the last day
shown = {}                # address -> deque of times a captcha was drawn for it, the last hour
# Large, few, well-spaced characters: easy for a person, and the token and limits stop scripts.
drawer = ImageCaptcha(width=280, height=100, font_sizes=(64, 70, 76))
drawer.character_rotate = (-12, 12)
drawer.character_warp_dx = (0.05, 0.12)
drawer.character_warp_dy = (0.05, 0.12)


def verifier(user, password, salt):
    """vmangos `SRP6::CalculateVerifier`: x = SHA1(salt_le || SHA1(USER:PASS)) read
    little-endian, v = g^x mod N. Salt and verifier are stored as big-endian hex."""
    inner = hashlib.sha1(f'{user.upper()}:{password.upper()}'.encode()).digest()
    outer = hashlib.sha1(salt.to_bytes(32, 'little') + inner).digest()
    return pow(G, int.from_bytes(outer, 'little'), N)


def client():
    # Caddy, in front, replaces X-Forwarded-For with the address it was connected from.
    return request.headers.get('X-Forwarded-For', request.remote_addr or '').split(',')[-1].strip()


def new_captcha():
    """A fresh captcha (token, PNG as a data URI), or None when this address asks for too many."""
    now = time.time()
    address = client()
    with lock:
        times = shown.setdefault(address, deque())
        while times and times[0] < now - HOUR:
            times.popleft()
        if len(times) >= 60:
            return None
        times.append(now)
        for token in [t for t, (_, expiry) in captchas.items() if expiry < now]:
            del captchas[token]
        while len(captchas) >= CAPTCHA_MAX:
            del captchas[next(iter(captchas))]
        answer = ''.join(secrets.choice(ALPHABET) for _ in range(5))
        token = secrets.token_urlsafe(24)
        captchas[token] = (answer, now + CAPTCHA_TTL)
    png = io.BytesIO()
    drawer.write(answer, png, format='png')
    return token, 'data:image/png;base64,' + base64.b64encode(png.getvalue()).decode()


def take_captcha(token, guess):
    """Whether the guess solves the token's captcha. A token is good for one guess."""
    with lock:
        answer, expiry = captchas.pop(token, (None, 0))
    return answer is not None and expiry >= time.time() and secrets.compare_digest(
        answer, guess.strip().upper().replace(' ', ''))


def over_limit():
    now = time.time()
    address = client()
    with lock:
        while made and made[0][0] < now - DAY:
            made.popleft()
        mine = sum(1 for _, a in made if a == address)
        if mine >= PER_IP_PER_DAY:
            return 'This address has made enough accounts for today. Try again tomorrow.'
        if len(made) >= PER_DAY:
            return 'Sign-up is closed for today. Try again tomorrow.'
    return None


def create(user, password):
    """Makes the account; False when the name is taken."""
    salt = secrets.randbits(256) | (1 << 255)
    s_hex, v_hex = f'{salt:064X}', f'{verifier(user, password, salt):X}'
    db = pymysql.connect(host=os.environ.get('DB_HOST', 'database'), user=os.environ['DB_USER'],
                         password=os.environ['DB_PASSWORD'], database='realmd', autocommit=True,
                         connect_timeout=5)
    try:
        with db.cursor() as cur:
            try:
                cur.execute('INSERT INTO account (username, s, v, joindate, last_ip) '
                            'VALUES (%s, %s, %s, NOW(), %s)', (user, s_hex, v_hex, client()[:30]))
            except pymysql.err.IntegrityError:
                return False
            cur.execute('INSERT INTO realmcharacters (realmid, acctid, numchars) '
                        'SELECT id, %s, 0 FROM realmlist', (cur.lastrowid,))
    finally:
        db.close()
    with lock:
        made.append((time.time(), client()))
    return True


PAGE = '''<!doctype html>
<html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>World of Kartcraft: make an account</title>
<style>
:root {{ color-scheme: dark; --bg: #121018; --panel: #1d1a26; --ink: #ece8f4; --dim: #a39cb4;
  --line: #3a3448; --accent: #ff8a1f; --bad: #ff6b6b; --good: #63d58a; }}
* {{ box-sizing: border-box; }}
body {{ margin: 0; min-height: 100vh; display: grid; place-items: center; padding: 24px 16px;
  background: radial-gradient(circle at 50% -20%, #3b2410, var(--bg) 60%); color: var(--ink);
  font: 16px/1.5 system-ui, -apple-system, "Segoe UI", sans-serif; }}
main {{ width: 100%; max-width: 400px; background: var(--panel); border: 1px solid var(--line);
  border-radius: 14px; padding: 28px 24px; }}
h1 {{ margin: 0 0 4px; font-size: 1.4rem; }}
h1 span {{ color: var(--accent); }}
p {{ margin: 0 0 18px; color: var(--dim); font-size: .92rem; }}
label {{ display: block; font-size: .85rem; color: var(--dim); margin: 14px 0 5px; }}
input {{ width: 100%; padding: 10px 12px; border-radius: 8px; border: 1px solid var(--line);
  background: #0f0d14; color: var(--ink); font: inherit; }}
input:focus {{ outline: 2px solid var(--accent); outline-offset: 1px; }}
.captcha {{ display: block; width: 100%; height: auto; border-radius: 8px; background: #fff; }}
button {{ width: 100%; margin-top: 22px; padding: 12px; border: 0; border-radius: 8px;
  background: var(--accent); color: #1a0f02; font: 600 1rem system-ui, sans-serif; cursor: pointer; }}
button:hover {{ filter: brightness(1.08); }}
.msg {{ padding: 10px 12px; border-radius: 8px; margin-bottom: 14px; font-size: .92rem; }}
.bad {{ background: #3a1c22; color: var(--bad); }}
.good {{ background: #173022; color: var(--good); }}
code {{ color: var(--ink); background: #0f0d14; padding: 1px 6px; border-radius: 5px; }}
small {{ display: block; margin-top: 18px; color: var(--dim); font-size: .8rem; }}
</style></head>
<body><main>
<h1>World of <span>Kartcraft</span></h1>
{body}
</main></body></html>'''


def form(error='', name=''):
    captcha = new_captcha()
    if captcha is None:
        return PAGE.format(body='<div class="msg bad">Too many requests from this address. '
                                'Wait an hour and try again.</div>'), 429
    token, image = captcha
    message = f'<div class="msg bad">{escape(error)}</div>' if error else ''
    return PAGE.format(body=f'''
<p>Make your game account. No email needed.</p>
{message}
<form method="post" action="/" autocomplete="off">
<input type="hidden" name="token" value="{token}">
<label for="name">Account name (3 to 16 letters and digits)</label>
<input id="name" name="name" value="{escape(name)}" required minlength="3" maxlength="16"
  pattern="[A-Za-z0-9]{{3,16}}" autocapitalize="characters" spellcheck="false">
<label for="password">Password (4 to 16 characters)</label>
<input id="password" name="password" type="password" required minlength="4" maxlength="16">
<label for="again">Password again</label>
<input id="again" name="again" type="password" required minlength="4" maxlength="16">
<label for="captcha">Type the letters in the picture</label>
<img class="captcha" src="{image}" alt="captcha" width="280" height="100">
<input id="captcha" name="captcha" required maxlength="8" autocapitalize="characters"
  spellcheck="false" style="margin-top:8px">
<button type="submit">Make my account</button>
</form>
<small>Passwords are not case sensitive in WoW 1.12. Lost passwords can't be recovered here:
ask whoever runs the server.</small>'''), 200


@app.get('/')
def show():
    return form()


@app.post('/')
def submit():
    name = request.form.get('name', '').strip()
    password = request.form.get('password', '')
    if not take_captcha(request.form.get('token', ''), request.form.get('captcha', '')):
        return form('The letters didn\'t match the picture. Here is a new one.', name)
    if not NAME.match(name):
        return form('The account name takes 3 to 16 letters and digits.', name)
    if not PASSWORD.match(password):
        return form('The password takes 4 to 16 characters, without spaces.', name)
    if password != request.form.get('again', ''):
        return form('The two passwords differ.', name)
    limit = over_limit()
    if limit:
        return form(limit, name)
    user = name.upper()
    try:
        if not create(user, password):
            return form(f'The name {user} is taken. Pick another.', '')
    except pymysql.MySQLError:
        app.logger.exception('creating %s', user)
        return form('The server could not make the account right now. Try again later.', name)
    app.logger.warning('account %s made from %s', user, client())
    realm = (f'<p>Set your game\'s server to <code>{escape(REALM_ADDRESS)}</code>, '
             'or use the World of Kartcraft launcher, which knows it.</p>') if REALM_ADDRESS else ''
    return PAGE.format(body=f'''
<div class="msg good">Account <strong>{escape(user)}</strong> is ready.</div>
<p>Log in with it and your password, make a character, and press J to climb into your kart.</p>
{realm}'''), 200


@app.get('/health')
def health():
    return 'ok'
