# Satchel Android push relay

The computer that receives mail calls the relay over HTTPS. The relay sends a
signed, high-priority FCM data message. Android fetches mail directly from the
mail provider. A user can pair multiple computers. The desktop enrollment UI
uses DNS-SD on the local network and asks the user to compare a six-digit code.
Once the computer approves, FCM delivers a challenge to the phone. The relay
activates the pairing after the phone returns that challenge with its receiver
credential. Unfinished offers expire after five minutes.

## Operation

Use a dedicated Workers **Free** account with SQLite-backed Durable Objects,
and Firebase Cloud Messaging on the Spark plan. Set up a service account in the
Satchel Android Firebase project with the project permission
`cloudmessaging.messages.create`. Keep the service account key on the operator
side. Users do not sign in to the relay or receive this key.

From this directory deploy with Wrangler 4 to the intended Free account. Set
`FCM_CREDENTIAL` with `wrangler secret put FCM_CREDENTIAL` from protected input.
It contains the Google service account JSON. Set `FIREBASE_PUBLIC` with the
Android client fields `project`, `sender`, `app` and `apiKey` from the registered
Satchel Android app. The config endpoint returns only these four fields.

Record the deployed HTTPS origin in `origin.txt` for release builds.
`SATCHEL_PUSH_RELAY_URL` overrides it at compile time for a development build.
An empty origin disables shared enrollment. The optional private Firebase flow
uses its existing configuration.

Workers Free allows 100,000 incoming requests a day. Durable Objects also have
request, compute and storage quotas. Enrollment and retries consume requests in
addition to signals. On Free, exhausted quotas cause errors. Desktop retries
outstanding signals. These quotas do not guarantee timely delivery during an outage.
[Workers pricing](https://developers.cloudflare.com/workers/platform/pricing/)
and [Durable Objects pricing](https://developers.cloudflare.com/durable-objects/platform/pricing/).

Run `node --test worker.node.mjs` for protocol tests with simulated Google
endpoints, and `wrangler deploy --dry-run` to check the Worker bundle.
