import { listen, type Event } from '@tauri-apps/api/event';
import { watch } from 'vue';
import {
  backendMobileNotificationsFrontendReady,
  backendMobileNotificationResponseHandled,
} from '@/services/backend';
import { useConfirm } from '@/composables/useConfirm';
import { useToast } from '@/composables/useToast';
import type { useComposerStore } from '@/stores/composer';
import type { useConnectionStore } from '@/stores/connection';
import {
  MAIL_ACTIONS,
  notificationAction,
  notificationRouteThreadId,
  type NativeNotificationRoute,
} from '@/utils/notificationNavigation';

/*
 * iOS hands mail notification responses to a listener lease and keeps each one
 * until it is ACKed. Nothing else uses that path, so this module stays out of
 * the startup bundle: the bootstrap loads it only once `runtime_capabilities`
 * says iOS, and before it reserves the first lease.
 */

/**
 * How a notification action ended. `done` and `cancelled` (the user kept the
 * open draft) are final and ACKed. The rest are not: native keeps the response,
 * reply text included, and delivers it again to the next lease. `interrupted`
 * (nobody answered the reply's question) and `unsaved` (the open draft could
 * not be saved to make room) wait for the dialog or composer in their way;
 * `failed` is everything else.
 */
export type NotificationActionOutcome = 'done' | 'cancelled' | 'failed' | 'interrupted' | 'unsaved';

function isFinalOutcome(outcome: NotificationActionOutcome) {
  return outcome === 'done' || outcome === 'cancelled';
}

// Shared stores can outlive a bootstrap component. Keep response work with the
// composer store so a replay joins a pending guard/action or ACKs finished work.
// Completed IDs last for that store's lifetime, including a lost ACK; failed
// ones are forgotten so a replay runs the action again. This is in-memory
// deduplication, not recovery of JS side effects after a WebView crash.
const notificationResponses = new WeakMap<object, Map<string, Promise<NotificationActionOutcome>>>();
// Failed deliveries per response, with the same lifetime as the map above.
const notificationFailures = new WeakMap<object, Map<string, number>>();
/**
 * Deliveries after which a read or archive that keeps failing is given up:
 * ACKed with a toast instead of replaying on every lease for the process
 * lifetime. A reply is never given up; its text exists nowhere else.
 */
const NOTIFICATION_ACTION_ATTEMPTS = 3;
/**
 * Pause before a failed read or archive is delivered again, times the
 * deliveries it has failed so far. A failure that repeats reaches the cap
 * above in minutes instead of in a tight loop.
 */
export const NOTIFICATION_RETRY_DELAY_MS = 15_000;
/**
 * The banner's cause while a reply waits for its Retry; a key, translated on
 * render. Only a hold nothing else will bring back reports it.
 */
const NOTIFICATION_REPLY_WAITING = 'backend.errors.notificationReplyWaiting';
// Keep lease reservations in request order even when IPC replies resolve late.
let notificationLifecycle: Promise<unknown> = Promise.resolve();
function updateNotificationListener(id: number | null, ready: boolean): Promise<number | null> {
  const next = notificationLifecycle.then(() => backendMobileNotificationsFrontendReady(id, ready));
  notificationLifecycle = next.catch(() => {});
  return next;
}

function sharedMap<V>(maps: WeakMap<object, Map<string, V>>, owner: object): Map<string, V> {
  let map = maps.get(owner);
  if (!map) {
    map = new Map();
    maps.set(owner, map);
  }
  return map;
}

/** What one bootstrap lends the response handling. */
export interface IosNotificationResponseHost {
  t: (key: string) => string;
  composer: ReturnType<typeof useComposerStore>;
  connection: ReturnType<typeof useConnectionStore>;
  /** Runs the action itself; shared with the Android paths. */
  handleNotificationAction: (route: NativeNotificationRoute, threadId: string) => Promise<NotificationActionOutcome>;
  /** Keeps an event registration for the bootstrap's teardown. */
  adoptDisposer: (dispose: () => void | Promise<void>) => void;
  isDisposed: () => boolean;
}

export interface IosNotificationResponses {
  /**
   * Reserve an iOS listener lease, register the route callback once, then
   * activate the lease so native replays what it retains. The banner's Retry
   * runs this again after any step failed; a new lease replaces the old one,
   * which also replays responses whose ACK was lost.
   */
  connect(): Promise<void>;
  /** Re-issue the lease, so native delivers every held response again. */
  redeliver(): Promise<void>;
  /** Retire the lease and stop every timer and watcher; the bootstrap is gone. */
  dispose(): void;
}

export function createIosNotificationResponses(host: IosNotificationResponseHost): IosNotificationResponses {
  const { t, composer, connection } = host;
  const sharedResponses = sharedMap(notificationResponses, composer);
  const responseFailures = sharedMap(notificationFailures, composer);
  const confirmDialog = useConfirm();
  let notificationListenerId: number | null = null;
  let notificationRouteListening = false;
  let notificationConnection: Promise<void> | null = null;
  /**
   * Responses this bootstrap was handed and could not ACK yet, and whether
   * anything other than the banner's Retry will bring each one back.
   */
  const heldResponses = new Map<string, { automatic: boolean }>();
  /** What has to happen before a new lease can help a held response. */
  const redeliverWhen = { dialogCloses: false, composerCloses: false };
  let redeliveryTimer: ReturnType<typeof setTimeout> | null = null;

  function retireNotificationListener() {
    if (notificationListenerId === null) return;
    const id = notificationListenerId;
    notificationListenerId = null;
    void updateNotificationListener(id, false).catch(error => {
      connection.reportBackendError('notifications', error);
      console.warn('mobile notification listener disposal failed', error);
    });
  }

  /**
   * Run one delivery and apply the retry policy: a failure stays unacknowledged
   * for the next lease, up to NOTIFICATION_ACTION_ATTEMPTS deliveries of a read
   * or archive. A thrown action counts as a failure.
   */
  async function settleNotificationAction(
    notification: NativeNotificationRoute,
    threadId: string,
    responseId: string,
  ): Promise<NotificationActionOutcome> {
    let outcome: NotificationActionOutcome;
    try {
      outcome = await host.handleNotificationAction(notification, threadId);
    } catch (error) {
      console.warn('mobile notification action failed', error);
      outcome = 'failed';
    }
    if (outcome !== 'failed') {
      responseFailures.delete(responseId);
      // Native keeps it for the next lease; say why nothing opened.
      if (outcome === 'interrupted') useToast().info(t('app.notifications.replyWaiting'));
      return outcome;
    }
    if (notificationAction(notification) === MAIL_ACTIONS.reply) return 'failed';
    const attempts = (responseFailures.get(responseId) ?? 0) + 1;
    if (attempts < NOTIFICATION_ACTION_ATTEMPTS) {
      responseFailures.set(responseId, attempts);
      return 'failed';
    }
    responseFailures.delete(responseId);
    useToast().error(t('app.notifications.actionFailed'));
    return 'cancelled';
  }

  async function onNotificationRoute({ payload }: Event<NativeNotificationRoute>) {
    const listenerId = notificationListenerId;
    if (host.isDisposed() || listenerId === null || !payload || payload.listenerId !== listenerId) return;
    const responseId = payload.responseId;
    if (typeof responseId !== 'string' || !responseId.trim() || responseId.length > 128) return;
    const threadId = notificationRouteThreadId(payload);
    if (!threadId || typeof payload.actionId !== 'string'
      || !['tap', ...Object.values(MAIL_ACTIONS)].includes(payload.actionId)) return;
    let work = sharedResponses.get(responseId);
    if (!work) {
      // Defer invocation until the promise is registered, for reentrancy.
      const started = Promise.resolve().then(() => settleNotificationAction(payload, threadId, responseId));
      sharedResponses.set(responseId, started);
      // Registered before any joiner awaits it, so a failure is gone
      // from the map by the time a caller sees it.
      void started.then(outcome => {
        if (!isFinalOutcome(outcome) && sharedResponses.get(responseId) === started) sharedResponses.delete(responseId);
      });
      work = started;
    }
    // Retain native response/reply text for replay after recreation.
    const outcome = await work;
    if (!isFinalOutcome(outcome)) {
      if (!host.isDisposed()) holdNotificationResponse(responseId, outcome, notificationAction(payload));
      return;
    }
    try {
      await backendMobileNotificationResponseHandled(responseId);
      releaseNotificationResponse(responseId);
    } catch (error) {
      // Completed work stays deduplicated, so the next delivery re-ACKs.
      connection.reportBackendError('notifications', error);
      console.warn('mobile notification acknowledgement failed', error);
    }
  }

  function connect(options: { yieldToDialog?: boolean } = {}): Promise<void> {
    notificationConnection ??= (async () => {
      try {
        retireNotificationListener();
        if (host.isDisposed()) return;
        // Reserve before registration, activate only after its callback exists.
        // A disposed bootstrap retires even a reservation that resolves late.
        const listenerId = await updateNotificationListener(null, false);
        notificationListenerId = listenerId;
        if (host.isDisposed()) {
          retireNotificationListener();
          return;
        }
        if (!notificationRouteListening) {
          host.adoptDisposer(await listen<NativeNotificationRoute>('mobile:notification-route', onNotificationRoute));
          notificationRouteListening = true;
        }
        if (host.isDisposed() || notificationListenerId !== listenerId) return;
        // A dialog that opened while the lease travelled: a replayed reply
        // would take its question away. Native keeps everything meanwhile.
        if (options.yieldToDialog && confirmDialog.state.pending) {
          retireNotificationListener();
          redeliverWhen.dialogCloses = true;
          return;
        }
        if (listenerId !== null) await updateNotificationListener(listenerId, true);
        // The lease works; a reply only Retry brings back is still waiting.
        connection.clearBackendError('notifications');
        showWaitingReplies();
      } catch (error) {
        retireNotificationListener();
        connection.reportBackendError('notifications', error);
        console.warn('mobile notification bridge failed', error);
      }
    })().finally(() => {
      notificationConnection = null;
    });
    return notificationConnection;
  }

  /**
   * The banner is for a held response nothing else brings back: a reply whose
   * action threw. The others come back by themselves and already have their
   * toast, or none, so they leave the banner to the sources it is for.
   */
  function showWaitingReplies() {
    if ([...heldResponses.values()].some(held => !held.automatic)) {
      connection.reportBackendError('notifications', NOTIFICATION_REPLY_WAITING);
    } else if (connection.backendErrorFor('notifications') === NOTIFICATION_REPLY_WAITING) {
      connection.clearBackendError('notifications');
    }
  }

  /**
   * Native keeps a response that was not ACKed, but delivers it again only to
   * a new lease. Re-issue one once whatever stopped the response is out of the
   * way: the dialog that took a reply's question, the composer whose draft
   * could not be saved, or, for a read or archive, a pause. A reply whose
   * action threw waits for the banner's Retry, which re-issues the lease too.
   */
  function holdNotificationResponse(responseId: string, outcome: NotificationActionOutcome, action: string | null) {
    const automatic = outcome === 'interrupted' || outcome === 'unsaved' || action !== MAIL_ACTIONS.reply;
    heldResponses.set(responseId, { automatic });
    showWaitingReplies();
    if (outcome === 'interrupted') redeliverWhen.dialogCloses = true;
    if (outcome === 'interrupted' || outcome === 'unsaved') {
      redeliverWhen.composerCloses = true;
      // Closed while the answer travelled: nothing is in the way any more.
      if (!composer.isOpen) redeliverOnceComposerStaysClosed();
    } else if (action !== MAIL_ACTIONS.reply) {
      scheduleNotificationRedelivery(NOTIFICATION_RETRY_DELAY_MS * (responseFailures.get(responseId) ?? 1));
    }
  }

  function releaseNotificationResponse(responseId: string) {
    if (heldResponses.delete(responseId)) showWaitingReplies();
  }

  /**
   * A replacement closes the composer on its way to the next request, the one
   * the user asked for. Only a composer that is still closed after the current
   * task is out of the way.
   */
  function redeliverOnceComposerStaysClosed() {
    setTimeout(() => {
      if (!composer.isOpen && redeliverWhen.composerCloses) void redeliver();
    });
  }

  function scheduleNotificationRedelivery(delay: number) {
    if (redeliveryTimer !== null || host.isDisposed()) return;
    redeliveryTimer = setTimeout(() => {
      redeliveryTimer = null;
      void redeliver();
    }, delay);
  }

  async function redeliver() {
    if (host.isDisposed()) return;
    // A replayed reply asks its question, and a new question takes the open
    // one away. Wait for that dialog to close instead.
    if (confirmDialog.state.pending) {
      redeliverWhen.dialogCloses = true;
      return;
    }
    redeliverWhen.dialogCloses = false;
    redeliverWhen.composerCloses = false;
    if (redeliveryTimer !== null) {
      clearTimeout(redeliveryTimer);
      redeliveryTimer = null;
    }
    // A lease still connecting may have delivered before the response failed.
    await notificationConnection;
    if (host.isDisposed()) return;
    // A dialog may have opened during that wait; the connection checks again
    // right before it activates the new lease.
    if (confirmDialog.state.pending) {
      redeliverWhen.dialogCloses = true;
      return;
    }
    await connect({ yieldToDialog: true });
  }

  // Transitions, not states: each one re-issues the lease at most once, so a
  // response that keeps failing waits for the next thing the user does.
  const stopWatchers = [
    watch(() => confirmDialog.state.pending, (pending, previous) => {
      if (!pending && previous && redeliverWhen.dialogCloses) void redeliver();
    }),
    watch(() => composer.isOpen, (open) => {
      if (!open && redeliverWhen.composerCloses) redeliverOnceComposerStaysClosed();
    }),
  ];

  return {
    connect: () => connect(),
    redeliver,
    dispose() {
      retireNotificationListener();
      if (redeliveryTimer !== null) clearTimeout(redeliveryTimer);
      redeliveryTimer = null;
      for (const stop of stopWatchers) stop();
    },
  };
}
