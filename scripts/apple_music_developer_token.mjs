// Signs and checks the Apple Music developer token (an ES256 JWT) that the
// Android build compiles in and the app falls back to fetching from GitHub.
//
// Sign (needs APPLE_MUSIC_PRIVATE_KEY, APPLE_MUSIC_KEY_ID, APPLE_MUSIC_TEAM_ID):
//   node scripts/apple_music_developer_token.mjs sign --days 90 --out android/apple_music/developer-token.jwt
// Check (no secrets needed, the signature is not verified):
//   node scripts/apple_music_developer_token.mjs expiry android/apple_music/developer-token.jwt --min-days 30
//
// The token is never printed. Apple allows a lifetime of at most 15777000 s.

import crypto from 'node:crypto';
import fs from 'node:fs';
import { pathToFileURL } from 'node:url';

export const MAX_DAYS = 182;
const DAY = 86400;

const b64url = (buf) => Buffer.from(buf).toString('base64url');

// Accepts PEM text, also with literal backslash-n sequences as pasted secrets have.
export function normalizePem(pem) {
  return String(pem ?? '')
    .replace(/\\n/g, '\n')
    .trim();
}

export function signToken({ privateKey, keyId, teamId, days = 90, now = Math.floor(Date.now() / 1000) }) {
  if (!Number.isFinite(days) || days < 1 || days > MAX_DAYS) {
    throw new Error(`--days must be between 1 and ${MAX_DAYS}`);
  }
  if (!privateKey || !keyId || !teamId) {
    throw new Error('the private key, key id and team id are all required');
  }
  const exp = now + Math.round(days * DAY);
  const header = b64url(JSON.stringify({ alg: 'ES256', kid: keyId }));
  const payload = b64url(JSON.stringify({ iss: teamId, iat: now, exp }));
  const signature = crypto.sign('sha256', Buffer.from(`${header}.${payload}`), {
    key: normalizePem(privateKey),
    dsaEncoding: 'ieee-p1363',
  });
  return { token: `${header}.${payload}.${b64url(signature)}`, exp };
}

// Returns { exp, daysLeft } or throws an Error saying why the file is unusable.
export function readExpiry(file, now = Math.floor(Date.now() / 1000)) {
  let text;
  try {
    text = fs.readFileSync(file, 'utf8').trim();
  } catch (e) {
    throw new Error(`cannot read ${file}: ${e.code ?? e.message}`);
  }
  const parts = text.split('.');
  if (parts.length !== 3) throw new Error(`${file} is not a JWT`);
  let exp;
  try {
    exp = JSON.parse(Buffer.from(parts[1], 'base64url').toString('utf8')).exp;
  } catch {
    throw new Error(`${file} has an unreadable payload`);
  }
  if (!Number.isFinite(exp)) throw new Error(`${file} has no exp claim`);
  return { exp, daysLeft: Math.floor((exp - now) / DAY) };
}

const iso = (exp) => new Date(exp * 1000).toISOString();

export function main(argv, env = process.env, log = console.log) {
  const [command, ...rest] = argv;
  const opts = {};
  const positional = [];
  for (let i = 0; i < rest.length; i++) {
    if (rest[i].startsWith('--')) opts[rest[i].slice(2)] = rest[++i];
    else positional.push(rest[i]);
  }
  try {
    if (command === 'sign') {
      if (!opts.out) throw new Error('--out is required');
      const { token, exp } = signToken({
        privateKey: env.APPLE_MUSIC_PRIVATE_KEY,
        keyId: env.APPLE_MUSIC_KEY_ID,
        teamId: env.APPLE_MUSIC_TEAM_ID,
        days: opts.days === undefined ? 90 : Number(opts.days),
      });
      fs.writeFileSync(opts.out, `${token}\n`);
      log(`expires ${iso(exp)}`);
      log(`wrote ${opts.out}`);
      return 0;
    }
    if (command === 'expiry') {
      if (!positional[0]) throw new Error('usage: expiry <file> [--min-days N]');
      const { exp, daysLeft } = readExpiry(positional[0]);
      log(`expires ${iso(exp)}`);
      log(`${daysLeft} days left`);
      if (opts['min-days'] !== undefined && daysLeft < Number(opts['min-days'])) {
        log(`fewer than ${opts['min-days']} days left`);
        return 1;
      }
      return 0;
    }
    throw new Error('usage: sign --out <file> [--days N] | expiry <file> [--min-days N]');
  } catch (e) {
    log(`error: ${e.message}`);
    return command === 'expiry' ? 1 : 2;
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  process.exitCode = main(process.argv.slice(2));
}
