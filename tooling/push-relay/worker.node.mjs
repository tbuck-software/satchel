import { test } from 'node:test';
import assert from 'node:assert/strict';
import { randomBytes, createHmac, generateKeyPairSync } from 'node:crypto';
import router, { PushLink } from './worker.mjs';

function fixture() {
  const rows = new Map();
  const storage = {
    get: async key => structuredClone(rows.get(key)),
    put: async (key, value) => rows.set(key, structuredClone(value)),
    setAlarm: async () => {}, deleteAlarm: async () => {}, deleteAll: async () => rows.clear(),
  };
  const { privateKey } = generateKeyPairSync('rsa', { modulusLength: 2048 });
  const env = { FCM_CREDENTIAL: JSON.stringify({ project_id: 'test', client_email: 'test@example.invalid',
    private_key: privateKey.export({ type: 'pkcs8', format: 'pem' }) }) };
  const object = new PushLink({ storage }, env);
  const key = () => randomBytes(32).toString('base64url');
  const device = '01'.repeat(16);
  function post(op, auth, payload = {}) {
    return object.fetch(new Request('https://link.internal/' + device + '/' + op, {
      method: 'POST', headers: auth ? { authorization: 'Bearer ' + auth } : {}, body: JSON.stringify(payload),
    }));
  }
  return { rows, storage, object, key, device, post };
}

test('a phone must receive the test signal and both peers can revoke', async t => {
  const messages = [];
  let unregistered = false;
  t.mock.method(globalThis, 'fetch', async (url, options) => {
    if (url === 'https://oauth2.googleapis.com/token') return Response.json({ access_token: 'test', expires_in: 3600 });
    assert.equal(url, 'https://fcm.googleapis.com/v1/projects/test/messages:send');
    const { message } = JSON.parse(options.body);
    messages.push(message);
    if (unregistered && message.token) return Response.json({ error: { details: [{ errorCode: 'UNREGISTERED' }] } }, { status: 404 });
    return Response.json({});
  });
  const { rows, key, device, post } = fixture();
  const { sender } = await (await post('create')).json();
  assert.notEqual(rows.get('link').sender, sender);
  const receiver = key(), hmac = key();
  const joined = await (await post('join', receiver, { key: hmac, token: 'a'.repeat(30), phone: 'Pixel' })).json();
  assert.match(joined.code, /^\d{6}$/);
  assert.equal((await post('status', key())).status, 403);
  assert.equal((await post('wake', sender)).status, 409);
  assert.equal((await post('approve', sender, { request: 'stale' })).status, 409);
  assert.equal((await post('approve', receiver, { request: joined.request })).status, 409);
  assert.equal(messages.length, 0);
  assert.equal((await post('approve', sender, { request: joined.request })).status, 200);
  assert.equal(messages.length, 1);
  const challenge = messages[0].data.verify;
  assert.ok(challenge);
  assert.equal((await post('received', receiver, { challenge: 'wrong' })).status, 409);
  assert.equal((await post('received', receiver, { challenge })).status, 200);
  assert.equal((await post('status', sender).then(r => r.json())).phase, 'active');
  assert.equal((await post('wake', receiver)).status, 409);
  assert.equal((await post('wake', sender, { body: 'anything' })).status, 400);
  unregistered = true;
  assert.equal((await post('wake', sender)).status, 200);
  const wake = messages.at(-1);
  assert.equal(wake.topic, joined.topic);
  assert.equal(wake.android.priority, 'high');
  assert.deepEqual(Object.keys(wake.data).sort(), ['device', 'proof', 'time']);
  assert.equal(wake.data.proof, createHmac('sha256', Buffer.from(hmac, 'base64url'))
    .update(device + ':' + wake.data.time).digest('base64url'));
  assert.equal((await post('wake', sender)).status, 429);
  assert.equal((await post('remove', receiver)).status, 200);
  assert.equal((await post('wake', sender)).status, 410);
});

test('expired offers cannot authorize a device and are deleted', async () => {
  const { rows, post, object, key } = fixture();
  const { sender } = await (await post('create')).json();
  rows.get('link').until = Date.now() - 1;
  assert.equal((await post('join', key(), { key: key(), token: 'a'.repeat(16) })).status, 410);
  await object.alarm();
  assert.equal(rows.size, 0);
  assert.equal((await post('status', sender)).status, 410);
});

test('public configuration exposes only Android client settings', async () => {
  const req = new Request('https://relay.invalid/v1/config');
  assert.equal((await router.fetch(req, {})).status, 503);
  const config = { project: 'test', sender: '1', app: 'test-app', apiKey: 'public-client-key' };
  const result = await router.fetch(req, { FIREBASE_PUBLIC: JSON.stringify({ ...config, ignored: 'secret' }), FCM_CREDENTIAL: '{}' });
  assert.deepEqual(await result.json(), config);
});
