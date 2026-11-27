// Run with: node --test scripts/apple_music_developer_token.test.mjs
import assert from 'node:assert/strict';
import crypto from 'node:crypto';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';

import { main, readExpiry, signToken } from './apple_music_developer_token.mjs';

const { privateKey, publicKey } = crypto.generateKeyPairSync('ec', { namedCurve: 'P-256' });
const pem = privateKey.export({ type: 'pkcs8', format: 'pem' });
const now = 1_800_000_000;
const sign = (extra = {}) => signToken({ privateKey: pem, keyId: 'KID123', teamId: 'TEAM456', now, ...extra });
const decode = (part) => JSON.parse(Buffer.from(part, 'base64url').toString('utf8'));
const tmp = () => fs.mkdtempSync(path.join(os.tmpdir(), 'amtoken-'));

test('token has three base64url parts with the expected claims', () => {
  const { token, exp } = sign({ days: 30 });
  const parts = token.split('.');
  assert.equal(parts.length, 3);
  for (const p of parts) assert.match(p, /^[A-Za-z0-9_-]+$/);
  assert.deepEqual(decode(parts[0]), { alg: 'ES256', kid: 'KID123' });
  assert.deepEqual(decode(parts[1]), { iss: 'TEAM456', iat: now, exp: now + 30 * 86400 });
  assert.equal(exp, now + 30 * 86400);
});

test('signature verifies with the public key', () => {
  const [h, p, s] = sign().token.split('.');
  const ok = crypto.verify('sha256', Buffer.from(`${h}.${p}`), { key: publicKey, dsaEncoding: 'ieee-p1363' }, Buffer.from(s, 'base64url'));
  assert.equal(ok, true);
  assert.equal(Buffer.from(s, 'base64url').length, 64);
});

test('lifetime bounds are enforced', () => {
  assert.throws(() => sign({ days: 183 }));
  assert.throws(() => sign({ days: 0 }));
  assert.doesNotThrow(() => sign({ days: 182 }));
  assert.doesNotThrow(() => sign({ days: 1 }));
  assert.ok(182 * 86400 <= 15777000);
});

test('a key with literal backslash-n sequences works', () => {
  const pasted = pem.trim().replace(/\n/g, '\\n');
  const { token } = signToken({ privateKey: pasted, keyId: 'K', teamId: 'T', now });
  assert.equal(token.split('.').length, 3);
});

test('expiry reports whole days left', () => {
  const file = path.join(tmp(), 't.jwt');
  fs.writeFileSync(file, `${sign({ days: 10 }).token}\n`);
  assert.equal(readExpiry(file, now).daysLeft, 10);
  assert.equal(readExpiry(file, now + 86400 + 100).daysLeft, 8);
  assert.equal(readExpiry(file, now + 11 * 86400).daysLeft, -1);
});

test('expiry exit codes follow --min-days and bad files', () => {
  const dir = tmp();
  const file = path.join(dir, 't.jwt');
  fs.writeFileSync(file, `${sign({ days: 20, now: Math.floor(Date.now() / 1000) }).token}\n`);
  const quiet = () => {};
  assert.equal(main(['expiry', file], {}, quiet), 0);
  assert.equal(main(['expiry', file, '--min-days', '10'], {}, quiet), 0);
  assert.equal(main(['expiry', file, '--min-days', '30'], {}, quiet), 1);
  assert.equal(main(['expiry', path.join(dir, 'missing.jwt'), '--min-days', '1'], {}, quiet), 1);
  const junk = path.join(dir, 'junk.jwt');
  fs.writeFileSync(junk, 'not a jwt\n');
  assert.equal(main(['expiry', junk, '--min-days', '1'], {}, quiet), 1);
});

test('sign command writes one line and never logs the token', () => {
  const out = path.join(tmp(), 'out.jwt');
  const lines = [];
  const env = { APPLE_MUSIC_PRIVATE_KEY: pem, APPLE_MUSIC_KEY_ID: 'K', APPLE_MUSIC_TEAM_ID: 'T' };
  assert.equal(main(['sign', '--out', out, '--days', '5'], env, (l) => lines.push(l)), 0);
  const text = fs.readFileSync(out, 'utf8');
  assert.match(text, /^[^\n]+\n$/);
  assert.ok(!lines.join('\n').includes(text.trim()));
  assert.equal(main(['sign', '--out', out, '--days', '400'], env, () => {}), 2);
});
