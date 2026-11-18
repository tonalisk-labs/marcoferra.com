// A held time: it is refused to others, two holds at once make one, its key answers its
// Checkout, a visitor's new hold ends their unpaid one, and a paid booking needs the waiver.
import { apiTest, calendar, expect } from '@tonalisk-labs/kit/testing';

import type { BookingRequest } from '../../src/api/endpoints.ts';
import {
  availability,
  book,
  callerFor,
  freeStarts,
  merge,
  pastFillTime,
  readyBooking,
  services,
  visitor,
} from '../support/api.ts';
import { MEETING } from '../support/meeting.ts';
import { fakeStripe, holdPaid, PAID, sessionOf } from '../support/payment.ts';

// Covers: event_types.payment_methods

const TAKEN = 'tag:marcoferra.com,2026:slot-taken';

apiTest(
  'a held time is refused to others, and its own key answers its Checkout',
  { tag: '@api' },
  async ({ stack }) => {
    const caller = callerFor(stack);
    const key = crypto.randomUUID();
    const request = await readyBooking(caller, 'held-first', PAID);
    const first = await book(caller, request, key);
    if (first.status !== 202) {
      throw new Error(`the hold answered ${String(first.status)}`);
    }
    const again = await book(caller, request, key);
    expect(again, 'a retry answers the same Checkout').toMatchObject({
      status: 202,
      data: { checkoutUrl: first.data.checkoutUrl },
    });
    const opened = await fakeStripe(stack).session(sessionOf(first.data.checkoutUrl));
    expect(opened['payment_method_types'], "the type's ways to pay").toEqual(PAID.payment_methods);

    const rival = callerFor(stack);
    const { formToken } = await availability(rival, PAID.slug);
    await pastFillTime();
    const rivals: BookingRequest[] = [PAID.slug, MEETING.slug].map((type) =>
      merge(request, merge(visitor(`held-rival-${type}`), { type, formToken })),
    );
    const replies = await Promise.all(rivals.map(async (each) => book(rival, each)));
    for (const reply of replies) {
      expect(reply).toMatchObject({ status: 409, data: { type: TAKEN } });
    }
  },
);

apiTest('two holds of one time at once make one', { tag: '@api' }, async ({ stack }) => {
  const race = await readyBooking(callerFor(stack), 'held-race', PAID);
  const entries = ['held-race-a', 'held-race-b'].map((label) => merge(race, visitor(label)));
  const replies = await Promise.all(entries.map(async (entry) => book(callerFor(stack), entry)));
  const statuses = replies.map(({ status }) => status);
  expect(statuses.toSorted((one, other) => one - other)).toEqual([202, 409]);
});

apiTest(
  "a visitor's new hold ends their unpaid one, freeing its time",
  { tag: '@api' },
  async ({ stack }) => {
    const { google } = services(stack);
    const caller = callerFor(stack);
    const earlier = await holdPaid(caller, 'held-twice');
    const later = await readyBooking(caller, 'held-twice', PAID);
    const same = { name: earlier.request.name, email: earlier.request.email };
    const reply = await book(caller, merge(later, same));
    expect(reply.status).toBe(202);
    expect(await fakeStripe(stack).session(earlier.session)).toMatchObject({ status: 'expired' });
    expect(await calendar(google).event(earlier.held.id)).toMatchObject({ status: 'cancelled' });
    expect(await freeStarts(stack, PAID.slug)).toContain(earlier.request.start);
  },
);

apiTest(
  'a paid booking without the withdrawal waiver is refused, naming it',
  { tag: '@api' },
  async ({ stack }) => {
    const caller = callerFor(stack);
    const request = await readyBooking(caller, 'no-waiver', PAID);
    const reply = await book(caller, merge(request, { withdrawalWaiver: false }));
    expect(reply.status).toBe(400);
    expect(reply.data).toMatchObject({ errors: [{ pointer: '/withdrawalWaiver' }] });
  },
);
