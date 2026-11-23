// Workers Free, SQLite-backed Durable Objects. Each object owns exactly one pairing.
const encoder = new TextEncoder();
const base64url = bytes => btoa(String.fromCharCode(...bytes)).replaceAll('+', '-').replaceAll('/', '_').replace(/=+$/, '');
const random = size => base64url(crypto.getRandomValues(new Uint8Array(size)));
const id = () => crypto.randomUUID().replaceAll('-', '');
const digest = async value => base64url(new Uint8Array(await crypto.subtle.digest('SHA-256', encoder.encode(value))));
const validSecret = value => typeof value === 'string' && /^[\w-]{43}$/.test(value);
const reply = (status, body) => Response.json(body ?? {}, { status, headers: { 'cache-control': 'no-store' } });
const offerLifetime = 5 * 60_000;

function sameSecret(value, expected) {
  if (typeof value !== 'string' || typeof expected !== 'string') return false;
  const input = encoder.encode(value);
  const secret = encoder.encode(expected);
  // The protocol fixes the lengths of auth digests, requests and challenges.
  return input.length === secret.length && crypto.subtle.timingSafeEqual(input, secret);
}

function pairingCode() {
  const range = 1_000_000;
  const limit = Math.floor(2 ** 32 / range) * range;
  const sample = new Uint32Array(1);
  do {
    crypto.getRandomValues(sample);
  } while (sample[0] >= limit);
  return String(sample[0] % range).padStart(6, '0');
}

async function body(request) {
  if (!request.body) return {};
  const reader = request.body.getReader();
  const chunks = [];
  let count = 0;
  while (true) {
    const { value, done } = await reader.read();
    if (done) break;
    count += value.length;
    if (count > 8192) { await reader.cancel(); throw new Error('size'); }
    chunks.push(value);
  }
  if (!count) return {};
  const data = JSON.parse(await new Blob(chunks).text());
  if (!data || typeof data !== 'object' || Array.isArray(data)) throw new Error('body');
  return data;
}

const cachedFcmSenders = new WeakMap();
function fcmSender(env) {
  const cached = cachedFcmSenders.get(env);
  if (cached) return cached;
  const { client_email, project_id, private_key } = JSON.parse(env.FCM_CREDENTIAL);
  const pem = private_key.replace(/-----[^-]+-----|\s/g, '');
  const key = crypto.subtle.importKey('pkcs8', Uint8Array.from(atob(pem), c => c.charCodeAt(0)),
    { name: 'RSASSA-PKCS1-v1_5', hash: 'SHA-256' }, false, ['sign']);
  const sender = { email: client_email, project: project_id, key, token: null };
  cachedFcmSenders.set(env, sender);
  return sender;
}

async function accessToken(sender) {
  if (sender.token?.until > Date.now()) return sender.token.value;
  const now = Math.floor(Date.now() / 1000);
  const encode = obj => base64url(encoder.encode(JSON.stringify(obj)));
  const claims = { iss: sender.email, scope: 'https://www.googleapis.com/auth/firebase.messaging',
    aud: 'https://oauth2.googleapis.com/token', iat: now, exp: now + 3600 };
  const jwt = encode({ alg: 'RS256', typ: 'JWT' }) + '.' + encode(claims);
  const signature = await crypto.subtle.sign('RSASSA-PKCS1-v1_5', await sender.key, encoder.encode(jwt));
  const res = await fetch(claims.aud, { method: 'POST', signal: AbortSignal.timeout(8000),
    body: new URLSearchParams({ grant_type: 'urn:ietf:params:oauth:grant-type:jwt-bearer',
      assertion: jwt + '.' + base64url(new Uint8Array(signature)) }) });
  if (!res.ok) throw new Error('sender');
  const token = await res.json();
  sender.token = { value: token.access_token, until: Date.now() + (token.expires_in - 60) * 1000 };
  return sender.token.value;
}

async function sendWake(env, link, challenge) {
  const sender = fcmSender(env);
  const token = await accessToken(sender);
  const stamp = String(Math.floor(Date.now() / 1000));
  const proofKey = await crypto.subtle.importKey('raw',
    Uint8Array.from(atob(link.key.replaceAll('-', '+').replaceAll('_', '/') + '='), c => c.charCodeAt(0)),
    { name: 'HMAC', hash: 'SHA-256' }, false, ['sign']);
  const proof = base64url(new Uint8Array(await crypto.subtle.sign('HMAC', proofKey, encoder.encode(link.id + ':' + stamp))));
  const data = { device: link.id, time: stamp, proof };
  if (challenge) data.verify = challenge;
  const send = target => fetch('https://fcm.googleapis.com/v1/projects/' + sender.project + '/messages:send',
    { method: 'POST', headers: { authorization: 'Bearer ' + token, 'content-type': 'application/json' },
      signal: AbortSignal.timeout(8000), body: JSON.stringify({ message: { ...target, data,
        android: { priority: 'high', ttl: '3600s', collapse_key: 'satchel-mail' } } }) });
  let res = await send(link.token ? { token: link.token } : { topic: link.topic });
  if (!challenge && link.token && res.status === 404) {
    const error = await res.json().catch(() => null);
    const details = error?.error?.details;
    if (Array.isArray(details) && details.some(d => d?.errorCode === 'UNREGISTERED')) {
      res = await send({ topic: link.topic });
      if (res.ok) link.token = null;
    }
  }
  if (!res.ok) throw new Error('delivery');
}

// Invoked only by the public router below. Credentials in the object are hashes,
// except for the FCM destination and the key needed to authenticate wake signals.
export class PushLink {
  constructor(ctx, env) { this.storage = ctx.storage; this.env = env; this.tail = Promise.resolve(); }
  fetch(req) {
    const work = this.tail.then(() => this.handle(req));
    this.tail = work.catch(() => {});
    return work.catch(() => reply(503));
  }
  alarm() {
    const work = this.tail.then(() => this.expire());
    this.tail = work.catch(() => {});
    return work;
  }
  async expire() {
    const link = await this.storage.get('link');
    if (link && link.phase !== 'active' && Date.now() >= link.until) await this.storage.deleteAll();
  }
  async handle(req) {
    const [, device, op] = new URL(req.url).pathname.split('/');
    const store = this.storage;
    const now = Date.now();
    let payload;
    try { payload = await body(req); } catch { return reply(400); }
    const link = await store.get('link');
    if (op === 'create') {
      if (link) return reply(409);
      const sender = random(32);
      const until = now + offerLifetime;
      await store.put('link', { id: device, sender: await digest(sender), phase: 'visible', until });
      await store.setAlarm(until);
      return reply(201, { id: device, sender, until });
    }
    if (!link || (link.phase !== 'active' && link.until <= now)) return reply(410);
    const auth = req.headers.get('authorization')?.replace(/^Bearer /, '');
    if (!validSecret(auth)) return reply(403);
    const who = await digest(auth);
    const sender = sameSecret(who, link.sender);
    let receiver = sameSecret(who, link.receiver);
    if (op === 'join' && link.phase === 'visible') {
      if (sender || !validSecret(payload.key) || typeof payload.token !== 'string' ||
          !/^[\w:-]{16,4096}$/.test(payload.token)) return reply(400);
      Object.assign(link, { receiver: who, phone: String(payload.phone ?? 'Android').replace(/[\x00-\x1f\x7f]/g, '').slice(0, 60), key: payload.key, token: payload.token,
        code: pairingCode(),
        request: id(), topic: 'satchel_' + random(32), phase: 'requested' });
      await store.put('link', link);
      receiver = true;
    } else if (!sender && !receiver) return reply(403);
    if (op === 'join' || op === 'status') {
      if (op === 'join' && !receiver) return reply(403);
      return reply(200, { phase: link.phase, code: link.code, request: link.request, phone: link.phone,
        ...(receiver ? { topic: link.topic } : {}) });
    }
    if (op === 'remove') { await store.deleteAlarm(); await store.deleteAll(); return reply(200); }
    if (op === 'approve' && sender && sameSecret(payload.request, link.request)) {
      if (link.phase === 'verifying') return reply(200);
      if (link.phase !== 'requested') return reply(409);
      link.challenge = random(32);
      await sendWake(this.env, link, link.challenge);
      link.phase = 'verifying';
      await store.put('link', link);
      return reply(200);
    }
    if (op === 'received' && receiver && link.phase === 'active') return reply(200);
    if (op === 'received' && receiver && link.phase === 'verifying' && sameSecret(payload.challenge, link.challenge)) {
      link.phase = 'active';
      delete link.challenge; delete link.code; delete link.request;
      await store.put('link', link);
      await store.deleteAlarm();
      return reply(200);
    }
    if (op === 'wake' && sender && link.phase === 'active') {
      if (Object.keys(payload).length) return reply(400);
      if (link.last && now - link.last < 2000) return reply(429);
      await sendWake(this.env, link);
      link.last = now;
      await store.put('link', link);
      return reply(200);
    }
    return reply(409);
  }
}

export default {
  async fetch(request, env) {
    const path = new URL(request.url).pathname;
    if (request.method === 'GET' && path === '/v1/config') {
      if (!env.FIREBASE_PUBLIC || !env.FCM_CREDENTIAL) return reply(503);
      const { project, sender, app, apiKey } = JSON.parse(env.FIREBASE_PUBLIC);
      return reply(200, { project, sender, app, apiKey });
    }
    if (request.method !== 'POST') return reply(404);
    const create = path === '/v1/offers';
    const match = /^\/v1\/links\/([0-9a-f]{32})\/(join|status|approve|received|wake|remove)$/.exec(path);
    if (!create && !match) return reply(404);
    const ip = request.headers.get('cf-connecting-ip') ?? 'unknown';
    const limiter = create || match?.[2] === 'join' ? env.ENROLL_LIMIT : env.REQUEST_LIMIT;
    if (!(await limiter.limit({ key: ip })).success) return reply(429);
    const device = create ? id() : match[1], op = create ? 'create' : match[2];
    const stub = env.LINKS.get(env.LINKS.idFromName(device));
    return stub.fetch(new Request('https://link.internal/' + device + '/' + op, request));
  },
};
