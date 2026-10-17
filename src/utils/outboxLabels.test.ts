import { describe, expect, it } from 'vitest';
import {
  outboxActionCanBeCancelled,
  outboxActionCanBeDismissed,
  outboxActionCanBeReopenedAsDraft,
  outboxActionCanBeRetried,
  outboxActionLabel,
  outboxErrorLabel,
  outboxRetryLabel,
  outboxStatusLabel,
  outboxStatusTone,
  outboxTimeLabel,
  outboxVisibleCount,
} from './outboxLabels';

describe('outboxStatusLabel', () => {
  it('translates the statuses the queue can return', () => {
    expect(outboxStatusLabel('delivered')).toBe('Sent · local save failed');
    expect(outboxStatusLabel('delivery_unknown')).toBe('Delivery status uncertain');
    expect(outboxStatusLabel('failed')).toBe('Failed');
    expect(outboxStatusLabel('sending')).toBe('Sending');
    expect(outboxStatusLabel('scheduled')).toBe('Scheduled');
    expect(outboxStatusLabel('pending')).toBe('Queued');
  });

  it('falls back to the raw status instead of hiding it', () => {
    expect(outboxStatusLabel('weird')).toBe('weird');
    expect(outboxStatusTone('weird')).toBe('neutral');
  });
});

describe('outboxActionLabel', () => {
  it('translates the known action types', () => {
    expect(outboxActionLabel('send_message')).toBe('Send');
    expect(outboxActionLabel('remote_mutation')).toBe('Server sync');
  });

  it('only allows unsent mail to be discarded', () => {
    expect(outboxActionCanBeCancelled('send_message', 'pending')).toBe(true);
    expect(outboxActionCanBeCancelled('send_message', 'delivered')).toBe(false);
    expect(outboxActionCanBeCancelled('send_message', 'sending')).toBe(false);
    expect(outboxActionCanBeCancelled('remote_mutation', 'failed')).toBe(true);
    expect(outboxActionCanBeCancelled('remote_mutation', 'sending')).toBe(false);
    expect(outboxActionCanBeCancelled('remote_mutation', 'pending')).toBe(false);
    expect(outboxActionCanBeDismissed('send_message', 'delivered')).toBe(true);
    expect(outboxActionCanBeDismissed('send_message', 'delivery_unknown')).toBe(true);
  });

  it('uses retry copy that matches the queued operation', () => {
    expect(outboxRetryLabel('send_message', 'failed')).toBe('Send again');
    expect(outboxRetryLabel('send_message', 'pending')).toBe('Send now');
    expect(outboxRetryLabel('remote_mutation', 'failed')).toBe('Try again');
  });
});

describe('outboxErrorLabel', () => {
  it('is empty for missing errors', () => {
    expect(outboxErrorLabel(null)).toBe('');
    expect(outboxErrorLabel('   ')).toBe('');
  });

  it('names the provider on a rate limit', () => {
    const label = outboxErrorLabel('gmail send skipped: rate limited for another 730s');
    expect(label).toContain('Gmail');
    expect(label).toContain('send limit');
    expect(label).not.toContain('730s');
  });

  it('points at re-authentication when the grant is gone', () => {
    expect(outboxErrorLabel('oauth refresh failed: invalid_grant')).toContain('reconnect');
  });

  it('reads network trouble as temporary', () => {
    expect(outboxErrorLabel('smtp connection refused')).toContain('No connection');
  });

  it('keeps actionable classifications inside structured queue failures', () => {
    const queuedFailure = (message: string) => JSON.stringify({
      key: 'backend.errors.queueActionFailed',
      params: { message },
    });

    expect(outboxErrorLabel(queuedFailure('gmail rate limit reached'))).toContain('Gmail');
    expect(outboxErrorLabel(queuedFailure('oauth invalid_grant'))).toContain('reconnect');
    expect(outboxErrorLabel(queuedFailure('smtp connection refused'))).toContain('No connection');
    expect(outboxErrorLabel(queuedFailure(JSON.stringify({
      key: 'backend.errors.sentMessagePersistenceFailed',
    })))).toContain('was sent');
  });

  it('passes unknown errors through untouched', () => {
    expect(outboxErrorLabel('mime encode failed at part 3')).toBe('mime encode failed at part 3');
  });
});

describe('outbox presentation', () => {
  it('does not count internal sync work as visible Outbox mail', () => {
    expect(outboxVisibleCount(0, 0)).toBe(0);
    expect(outboxVisibleCount(2, 1)).toBe(3);
  });

  it('never offers to resend a provider-delivered message', () => {
    expect(outboxActionCanBeRetried('send_message', 'delivered')).toBe(false);
    expect(outboxActionCanBeRetried('send_message', 'delivery_unknown', '2026-08-31T09:00:00Z'))
      .toBe(false);
    expect(outboxActionCanBeRetried('send_message', 'failed')).toBe(true);
  });

  it('offers a retry when the crash happened before the first byte went out', () => {
    expect(outboxActionCanBeRetried('send_message', 'delivery_unknown')).toBe(true);
    expect(outboxActionCanBeRetried('send_message', 'delivery_unknown', null)).toBe(true);
    expect(outboxActionCanBeRetried('send_message', 'sending')).toBe(false);
    expect(outboxActionCanBeRetried('remote_mutation', 'failed')).toBe(true);
  });

  it('reopens only mail whose text is still worth rescuing', () => {
    expect(outboxActionCanBeReopenedAsDraft('send_message', 'delivery_unknown')).toBe(true);
    expect(outboxActionCanBeReopenedAsDraft('send_message', 'failed')).toBe(true);
    expect(outboxActionCanBeReopenedAsDraft('send_message', 'delivered')).toBe(false);
    expect(outboxActionCanBeReopenedAsDraft('send_message', 'pending')).toBe(false);
    expect(outboxActionCanBeReopenedAsDraft('remote_mutation', 'failed')).toBe(false);
  });

  it('calls a missed schedule overdue instead of planned in the past', () => {
    const now = new Date('2026-08-11T20:30:00Z').getTime();
    expect(outboxTimeLabel('scheduled', '2026-08-11T20:29:00Z', now)).toBe('Overdue since');
    expect(outboxTimeLabel('scheduled', '2026-08-11T20:31:00Z', now)).toBe('Scheduled for');
  });

  it('labels terminal delivery timestamps by what happened', () => {
    expect(outboxTimeLabel('delivered')).toBe('Sent');
    expect(outboxTimeLabel('delivery_unknown')).toBe('Last send attempt');
  });
});
