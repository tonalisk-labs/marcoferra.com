// Expiry: a hold lasts its configured minutes and an expired Checkout frees it; one whose
// expiry Stripe never announced is freed when the next visitor asks for its time.
import { apiTest, calendar, expect } from '@tonalisk-labs/kit/testing';

import site from '../../../marcoferra.config.json' with { type: 'json' };
import {
  availability,
  book,
  callerFor,
  freeStarts,
  merge,
  pastFillTime,
  services,
  visitor,
} from '../support/api.ts';
import { fakeStripe, holdPaid, PAID } from '../support/payment.ts';

/** The hold's length: the configured minutes and the minute added for Stripe's clock. */
const HOLD_MS = (site.booking.hold_minutes + 1) * 60_000;
// Covers: booking.hold_minutes

apiTest(
  'an expired Checkout frees its time when Stripe says so',
  { tag: '@api' },
  async ({ stack }) => {
    const { google } = services(stack);
    const caller = callerFor(stack);
    const { held, request, session } = await holdPaid(caller, 'expired');
    const lasts = Date.parse(held.expiresAt) - Date.now();
    expect(lasts, 'the hold lasts its minutes, to the second').toBeLessThanOrEqual(HOLD_MS);
    expect(lasts).toBeGreaterThan(HOLD_MS - 60_000);
    expect(await freeStarts(stack, PAID.slug), 'held meanwhile').not.toContain(request.start);

    await fakeStripe(stack).expire(session);
    expect(await fakeStripe(stack).delivered('checkout.session.expired', session)).toEqual([200]);
    expect(await calendar(google).event(held.id)).toMatchObject({ status: 'cancelled' });
    expect(await freeStarts(stack, PAID.slug), 'free again').toContain(request.start);
  },
);

apiTest(
  'a hold whose expiry was never announced is freed by the next visitor asking',
  { tag: '@api' },
  async ({ stack }) => {
    const { google } = services(stack);
    const caller = callerFor(stack);
    const { held, request, session } = await holdPaid(caller, 'lost-expiry');
    await fakeStripe(stack).expire(session, true);
    expect(await calendar(google).event(held.id), 'nobody told the calendar').toMatchObject({
      status: 'confirmed',
    });

    const next = callerFor(stack);
    const { formToken } = await availability(next, PAID.slug);
    await pastFillTime();
    const taking = merge(request, merge(visitor('after-lost-expiry'), { formToken }));
    const reply = await book(next, taking);
    expect(reply.status, 'the expired hold gave way').toBe(202);
    expect(await calendar(google).event(held.id)).toMatchObject({ status: 'cancelled' });
  },
);
