// A refund from the Stripe Dashboard: charge.refunded cancels the meeting, sends the iMIP
// CANCEL for the same event saying the payment is refunded, and frees the time.
import { apiTest, calendar, expect } from '@tonalisk-labs/kit/testing';

import { callerFor, freeStarts, services } from '../support/api.ts';
import { icsLines, invitationThenNext, mailOf, uidOf } from '../support/mail.ts';
import { bookPaid, fakeStripe, PAID } from '../support/payment.ts';

apiTest(
  "the host's refund cancels the meeting and tells the visitor",
  { tag: '@api' },
  async ({ stack }) => {
    const { google, mailpit } = services(stack);
    const stripe = fakeStripe(stack);
    const { booked, request, session } = await bookPaid(callerFor(stack), 'dashboard');
    const { invitation, next } = await invitationThenNext(mailpit, request.email);
    await stripe.refund(session);
    expect(await stripe.delivered('charge.refunded', session)).toEqual([200]);

    const [requested, cancelled] = await Promise.all([mailOf(mailpit, invitation), next]);
    expect(icsLines(cancelled.invite)).toEqual(
      expect.arrayContaining(['METHOD:CANCEL', 'STATUS:CANCELLED']),
    );
    expect(uidOf(cancelled.invite)).toBe(uidOf(requested.invite));
    expect(cancelled.text).toContain('Your payment is refunded in full.');
    expect(await calendar(google).event(booked.id)).toMatchObject({ status: 'cancelled' });
    expect(await freeStarts(stack, PAID.slug)).toContain(request.start);
  },
);
