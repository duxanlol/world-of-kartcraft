# Account sign-up page

One web page where players make their own game account on a Kartcraft server. There's no email
and no third-party service:

- **Captcha.** A captcha image drawn by the page itself. Each one is good for one try and
  10 minutes.
- **Limits.** Each address may make a few accounts a day (`PER_IP_PER_DAY`, default 3), and the
  page makes at most `PER_DAY` (default 100) in all. A single address may also ask for at most
  60 captchas an hour.
- **Never overwrites.** It only creates accounts, as ordinary players. A taken name is refused,
  never overwritten. The page's database user can do nothing but add accounts.
- **HTTPS.** Caddy, in front, gets and renews a Let's Encrypt certificate.

## Running it

On the server host, beside the running game server (Docker project `world-of-kartcraft`):

1. Point a DNS name at the host, and let ports 80 and 443 reach it.
2. Make the page's database user. Use a long random password:

   ```sh
   PW=$(head -c 32 /dev/urandom | base64 | tr -d '/+=' | head -c 32)
   docker compose -p world-of-kartcraft exec -T database mariadb -uroot -p<root password> -e "
     CREATE USER 'register'@'%' IDENTIFIED BY '$PW';
     GRANT SELECT (id, username), INSERT ON realmd.account TO 'register'@'%';
     GRANT SELECT ON realmd.realmlist TO 'register'@'%';
     GRANT INSERT ON realmd.realmcharacters TO 'register'@'%';"
   ```

3. Write `.env` beside `compose.yaml`. It holds a password, so never commit it:

   ```
   SITE_ADDRESS=play.example.com
   REALM_ADDRESS=play.example.com
   DB_USER=register
   DB_PASSWORD=<the password from step 2>
   ```

4. Start it with `docker compose -p kartcraft-register up -d --build`.

The page logs every account it makes, with the address it came from:
`docker compose -p kartcraft-register logs register`.

Set `REGISTER_URL=https://play.example.com` in `.setup\publish.cfg`, and the friends' launcher
shows it on the first start.
