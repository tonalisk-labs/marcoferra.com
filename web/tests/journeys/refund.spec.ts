// Cancelling a paid meeting: in time, or within the withdrawal period, it is refunded in full
// and the page and email say so; an older one within the notice is not refunded.
import { apiTest, calendar, type Coordinates, expect, test } from '@tonalisk-labs/kit/testing';

import site from '../../../marcoferra.config.json' with { type: 'json' };
import type { Booking } from '../../src/api/endpoints.ts';
import { callerFor, cancel, sentTo, services } from '../support/api.ts';
import { icsLines, invitationThenNext } from '../support/mail.ts';
import { bookPaid, fakeStripe, PAID, PRICE } from '../support/payment.ts';

// Covers: booking.refund_until_hours, booking.withdrawal_days

const { refund_until_hours: HOURS, withdrawal_days: DAYS } = site.booking;
const DAY_MS = 24 * 60 * 60_000;
const REFUNDED = `The meeting is cancelled. The cancellation is on its way to your inbox. Your payment of ${PRICE} is refunded in full.`;

/** The host moves the meeting to start within the notice, as if the visitor came back late. */
const startSoon = async (stack: Coordinates, booked: Booking): Promise<void> => {
  const soon = Date.now() + (HOURS * 60 - 30) * 60_000;
  const end = soon + PAID.duration_minutes * 60_000;
  const { google } = services(stack);
  await calendar(google).move(booked.id, new Date(soon).toISOString(), new Date(end).toISOString());
};

test('a paid meeting cancelled in time is refunded in full', async ({ page, stack }) => {
  const { mailpit } = services(stack);
  const { booked, request, session } = await bookPaid(callerFor(stack), 'refunded');
  expect(booked.cancelUrl).toContain(`type=${PAID.slug}`);
  const { next } = await invitationThenNext(mailpit, request.email);
  await page.goto(booked.cancelUrl);
  await expect(page.getByText('cancelling now refunds your payment in full')).toBeVisible();
  await page.getByRole('button', { name: 'Cancel the meeting' }).click();
  await expect(page.getByRole('status')).toHaveText(REFUNDED);

  const cancellation = await next;
  expect(icsLines(cancellation.invite)).toContain('METHOD:CANCEL');
  expect(cancellation.text).toContain('is refunded in full');
  expect(await fakeStripe(stack).session(session)).toMatchObject({
    payment_intent: { latest_charge: { amount_refunded: PAID.price_minor } },
  });
  // Stripe's word of the refund finds the meeting already cancelled, and sends nothing more.
  expect(await fakeStripe(stack).delivered('charge.refunded', session)).toEqual([200]);
  expect(await sentTo(mailpit, request.email)).toHaveLength(2);
});

apiTest(
  'a recent booking cancelled within the notice is refunded: the withdrawal period',
  { tag: '@api' },
  async ({ stack }) => {
    const caller = callerFor(stack);
    const { booked, session } = await bookPaid(caller, 'withdrawn');
    await startSoon(stack, booked);
    const cancelled = await cancel(caller, booked.cancelUrl);
    expect(cancelled).toMatchObject({
      status: 200,
      data: { refundedMinor: PAID.price_minor, currency: PAID.currency },
    });
    expect(await fakeStripe(stack).session(session)).toMatchObject({
      payment_intent: { latest_charge: { amount_refunded: PAID.price_minor } },
    });
  },
);

apiTest(
  'an older booking cancelled within the notice is not refunded',
  { tag: '@api' },
  async ({ stack }) => {
    const { google, mailpit } = services(stack);
    const caller = callerFor(stack);
    const { booked, request, session } = await bookPaid(caller, 'late-cancel');
    const { next } = await invitationThenNext(mailpit, request.email);
    const longAgo = Math.floor((Date.now() - (DAYS + 1) * DAY_MS) / 1000);
    await calendar(google).note(booked.id, 'bookedAt', String(longAgo));
    await startSoon(stack, booked);
    const cancelled = await cancel(caller, booked.cancelUrl);
    expect(cancelled).toMatchObject({
      status: 200,
      data: { refundedMinor: 0, currency: PAID.currency },
    });
    const { text } = await next;
    expect(text).toContain('is not refunded');
    expect(await fakeStripe(stack).session(session)).toMatchObject({
      payment_intent: { latest_charge: { amount_refunded: 0 } },
    });
  },
);
